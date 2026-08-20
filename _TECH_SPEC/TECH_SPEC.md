# Канонисса-Библиотека — Единая техническая спецификация

**Версия:** 3.0  
**Дата актуализации:** 20 августа 2026 г.  
**Автор:** Valeria Fadeeva <valeria.fadeeva.me@gmail.com>  
**Платформа:** AMD Ryzen 7 5700X / Radeon 9070 XT 16GB / 128 GB DDR4 / Arch Linux  
**Стек:** Flutter Desktop (Dart) ↔ Rust Core Engine (Axum REST API) + OpenCV 4.x / SANE / libtiff

---

## СОДЕРЖАНИЕ

1. [Назначение системы](#1-назначение-системы)
2. [Архитектура слоёв](#2-архитектура-слоев)
3. [Конфигурация и хранение данных](#3-конфигурация-и-хранение-данных)
4. [Входные данные (SANE)](#4-входные-данные-sane)
5. [Computer Vision конвейер](#5-computer-vision-конвейер)
6. [Детекция типа страницы: обложка vs разворот](#6-детекция-типа-страницы-обложка-vs-разворот)
7. [Обработка изображений и бинаризация](#7-обработка-изображений-и-бинаризация)
8. [CCITT Group 4 и экспорт](#8-ccitt-group-4-и-экспорт)
9. [Flutter UI](#9-flutter-ui)
10. [Отказоустойчивость и сессии](#10-отказоустойчивость-и-сессии)
11. [Безопасность и валидация](#11-безопасность-и-валидация)
12. [Упаковка и дистрибуция](#12-упаковка-и-дистрибуция)
13. [Инженерные директивы](#13-инженерные-директивы)
14. [Текущее состояние кодовой базы](#14-текущее-состояние-кодовой-базы)
15. [Roadmap](#15-roadmap)

---

## 1. НАЗНАЧЕНИЕ СИСТЕМЫ

Высокоскоростная потоковая оцифровка архивных и библиотечных книг (включая издания старше 75 лет, перешедшие в общественное достояние) на планшетном сканере формата А3 (EPSON GT-20000 / Canon LiDE) в условиях незакрытой крышки.

### Метрики рантайма

| Метрика | Значение |
|---------|----------|
| Производительность | ≥ 165 страниц/смену (~5ч 20мин оператора по Pomodoro) |
| Скорость конвейера | ≤ 150 мс на разворот (захват → детекция → обрезка → бинаризация → сохранение) |
| Точность контуров | 100% за счёт математического восстановления геометрии при шумах |
| Размер страницы | ~80–120 KB/страницу (CCITT Group 4 монохром A4/A3) |
| Книга 400 стр. | ≤ 40 MB итогового PDF |

### Матрица внешних паразитных помех

Алгоритмы обязаны программно вырезать:
- Элементы интерьера помещения (потолок, лампы дневного света)
- Блики от освещения на глянцевых элементах страниц
- Тени рук оператора при удержании книги
- Торцы предыдущих/последующих страниц толстых книг ("боковушки")

---

## 2. АРХИТЕКТУРА СЛОЁВ

```text
┌─────────────────────────────────────────┐
│ Flutter Desktop Client (Dart)           │
│ • GUI управление сканированием          │
│ • CustomPainter интерактивной сетки     │
│ • BLoC реактивная модель состояний      │
│ • Drag-and-Drop вершин вручную          │
│ • Нелинейная навигация по книге         │
└──────────────┬──────────────────────────┘
               │ HTTP REST API (localhost:54321)
               ▼
┌─────────────────────────────────────────┐
│ Rust Core Engine                        │
│ ─────────────────────────────────────── │
│ Axum Web Server                         │
│ ├─ GET  /api/v1/health                  │
│ ├─ POST /api/v1/scanner/init            │
│ ├─ POST /api/v1/scanner/process         │
│ ├─ POST /api/v1/scan                    │
│ ├─ GET/POST /api/v1/calibration         │
│ ├─ PATCH /api/v1/scan/{uuid}/adjust-vertex │
│ ├─ POST /api/v1/export-pdf              │
│ ├─ POST /api/v1/import-pdf              │
│ ├─ POST /api/v1/replace-pdf-page        │
│ ├─ POST /api/v1/insert-pdf-page         │
│ └─ POST /api/v1/clean-pdf-page          │
│                                         │
│ SANE Layer (sane_core.rs)               │
│ ├─ scanimage -L автообнаружение         │
│ ├─ захват TIFF RAW @ 300 DPI в RAM      │
│ └─ декодирование в OpenCV Mat           │
│                                         │
│ Computer Vision Pipeline (cv/)          │
│ ├─ coarse_mask: мультимасштабная маска  │
│ ├─ isolate_side_artifacts: боковушки    │
│ ├─ process_book_contours: вершины P₁..P₄│
│ ├─ detect_page_type: обложка/разворот   │
│ ├─ rectify_and_dewarp_page: перспектива │
│ ├─ dewarp_spine: цилиндрический деварпинг│
│ ├─ segment_pages: левая/правая          │
│ ├─ detect_skew_angle + rotate_image     │
│ ├─ apply_profile: multi-profile         │
│ ├─ apply_sauvola_threshold: бинаризация │
│ ├─ encode_ccitt_g4_to_file: экспорт     │
│ └─ calibration: hot-reload параметров   │
│                                         │
│ Session Store (SQLite)                  │
│ ├─ session_store.rs: UUID-сессии, WAL   │
│ └─ session_recovery.rs: hot restart     │
└─────────────────────────────────────────┘
```

### Режимы работы Rust-ядра

1. **Web Mode** (дефолт): Axum-сервер на `127.0.0.1:54321` (настраивается через `--host`/`--port` или `config.toml`) для взаимодействия с Flutter через JSON-API.
2. **CLI Mode** (`--cli` флаг): автономный конвейер без сервера — чтение файла или прямой захват со сканера → обработка → сохранение TIFF/PNG в папку из конфига.

### Коллизия А: Фриз UI при маршалинге RAW-буфера

При захвате полного кадра А3 @ 300 DPI получается растр ~3300×4700 px (~45 MB RGB). Передача такого буфера во Flutter требует Zero-Copy обёртки или превью JPEG/WebP — иначе фриз UI на 300–450 мс.

**Решение:** Rust-ядро преобразует RAW-буфер в лёгкий JPEG/WebP и отдаёт во Flutter указатель на `Uint8List` через Zero-Copy.

---

## 3. КОНФИГУРАЦИЯ И ХРАНЕНИЕ ДАННЫХ

### 3.1. Единый конфигурационный файл

Сервер и клиент читают **один и тот же** файл конфигурации по XDG Base Directory Specification.

**Путь к конфигурации (приоритет):**
1. `$FLAT_SCANNER_CONFIG` (переменная окружения, для systemd)
2. `~/.config/flat-scanner/config.toml`
3. `/etc/flat-scanner/config.toml` (системный, устанавливается PKGBUILD)

### 3.2. Структура config.toml

```toml
# Конфигурация Flat Scanner (Core Engine + Client)
#
# Приоритет источников:
#   1. CLI-флаги (--host, --port) — высший приоритет (только сервер)
#   2. Этот файл (config.toml)
#   3. Дефолтные значения

[server]
# Адрес привязки:
#   "127.0.0.1" — только локальная машина (безопасно по умолчанию)
#   "0.0.0.0"   — доступ по сети (для удалённого Flutter-клиента)
host = "127.0.0.1"

# Порт HTTP-шлюза
port = 54321

[paths]
# Базовый каталог для всех данных (поддерживается ~)
base_dir = "~/.local/share/flat-scanner"

# Подкаталоги относительно base_dir
raw_dir = "raw"
processed_dir = "processed"
export_dir = "export"
import_dir = "import"

# База данных (относительно base_dir)
database = "data.db"
```

### 3.3. Карта каталогов

| Тип данных | Путь | Описание |
|------------|------|----------|
| Конфигурация | `~/.config/flat-scanner/config.toml` | Единый конфиг для сервера и клиента |
| Сырые сканы | `~/.local/share/flat-scanner/raw/` | Исходные TIFF от сканера (опционально) |
| Обработанные | `~/.local/share/flat-scanner/processed/` | CCITT G4 / PNG страницы |
| Экспорт PDF | `~/.local/share/flat-scanner/export/` | Финальные PDF |
| Импорт PDF | `~/.local/share/flat-scanner/import/` | Временные файлы импорта |
| База данных | `~/.local/share/flat-scanner/data.db` | SQLite сессии |

### 3.4. Требования к реализации

**Сервер (Rust):**
- Модуль `config.rs` обязан резолвить `~` в `$HOME` при загрузке путей
- При старте создавать все каталоги из конфига (`fs::create_dir_all`)
- Передавать пути через `Arc<Config>` в обработчики через `Extension`

**Клиент (Dart):**
- Читать тот же `config.toml` при инициализации
- Парсить секции `[server]` и `[paths]`
- Использовать `host` и `port` из `[server]` для HTTP-запросов
- Использовать пути из `[paths]` для отображения/экспорта файлов

### 3.5. Калибровка (calibration.json)

Отдельный файл для параметров бинаризации с hot-reload:

**Путь:** `~/.config/flat-scanner/calibration.json`

```json
{
  "k_factor": 0.2,
  "window_size": 15,
  "profile": "text_bw_1bit"
}
```

`CalibrationManager` отслеживает `mtime` файла, перечитывает при изменении (троттлинг 500 мс).

---

## 4. ВХОДНЫЕ ДАННЫЕ (SANE)

### Интерфейс захвата

- Старт строго по триггеру оператора (кнопка во Flutter), не автономно.
- Системный вызов `scanimage` из Rust через `std::process::Command`:
  - Разрешение: **300 DPI**
  - Формат: несжатый RAW/TIFF (исключает артефакты компрессии перед CV-анализом)
  - Область: полный кадр планшета A3 (или А4 при обнаружении Canon LiDE)

### Автоопределение геометрии устройства

При обнаружении в имени SANE-устройства строк `"genesys"`, `"pixma"` или `"niash"` автоматически переключается профиль на A4 (210×297 мм). Иначе используется полный A3 (297×420 мм).

### Реализация (`sane_core.rs`)

- `detect_hardware_scanner()` — опрос `scanimage -L`, фильтрация веб-камер, парсинг device address.
- `capture_sane_frame(device_name)` — захват TIFF в RAM, декодирование через `imgcodecs::imdecode`.
- RAII: `SaneScanner` реализует `Drop` для гарантированного закрытия дескрипторов.
- Таймауты: все SANE-операции обёрнуты в `tokio::time::timeout`.

---

## 5. COMPUTER VISION КОНВЕЙЕР

### 5.1 Coarse Masking (B3) ✅

**Файл:** `src/cv/segmentation.rs` → `coarse_mask()`

Мультимасштабный алгоритм для изоляции зоны книги от потолка/ламп:

1. Grayscale.
2. Три масштаба размытия (51/101/201) → Otsu INV на каждом.
3. На каждом масштабе — кандидаты контуров, оценка `solidity` (площадь / площадь выпуклой оболочки).
4. Лучший кандидат (макс. solidity при площади 10%–95% кадра).
5. Морфологическое закрытие (ellipse 15×15, 2 итерации) для заполнения провалов.
6. `bitwise_and` к исходному изображению.

### 5.2 Изоляция боковых артефактов (B2) ✅

**Файл:** `src/cv/segmentation.rs` → `isolate_side_artifacts()`

Градиентный анализ плотности по периферии macro-contour:

1. Извлечение полосы (band) шириной 8 px вокруг контура (XOR маска с эродированной).
2. Подсчёт частоты чередований светлых/тёмных пикселей вдоль строк.
3. Если частота > 30% — паттерн "боковушек" обнаружен.
4. Эрозия маски на 3 итерации (сдвиг рамки внутрь).

### 5.3 Детекция четырёх вершин ✅

**Файл:** `src/cv/segmentation.rs` → `process_book_contours()`

Пайплайн:
1. Coarse masking → отсечение артефактов.
2. Grayscale → Otsu бинаризация (THRESH_BINARY_INV).
3. `findContours` (RETR_EXTERNAL) → крупнейший контур по площади (мин. 1% кадра).
4. B2: изоляция боковых артефактов → пересчёт контура из улучшенной маски.
5. `approxPolyDP(ε = perimeter × 0.02)` → если 4 точки → сортировка TL→TR→BR→BL.
6. Fallback: `minAreaRect` → 4 угла → сортировка.

Сортировка: диагональная проекция (x+y), разделение на верхнюю/нижнюю пары, сортировка внутри по X.

### 5.4 Деварпинг корешка (B1) ✅

**Файл:** `src/cv/warping.rs`

Двухуровневая коррекция:

**Перспективная трансформация** (`perspective_warp`):
- `getPerspectiveTransform` (DECOMP_LU) по вершинам P₁..P₄ → `warpPerspective` (INTER_LINEAR, BORDER_TRANSPARENT).

**Цилиндрический деварпинг** (`dewarp_spine`):
1. Grayscale + Otsu бинаризация.
2. `HoughLinesP` (threshold=40, minLen=30) → фильтрация вертикальных линий (70°–110°).
3. Группировка по колонкам X → среднее смещение dx для каждой колонки.
4. Сглаживание скользящим окном (15 px).
5. Цилиндрическая модель: `detect_spine_shadow` (вертикальное усреднение → градиент → минимум в центральной трети) → `build_cylindrical_deformation` (квадратичная зависимость от расстояния до корешка).
6. Комбинация смещений, ограничение ±20 px.
7. Построение карт `map_x`, `map_y` (CV_32F) → `remap` (INTER_CUBIC).

### 5.5 Сегментация разворота ✅

**Файл:** `src/cv/segmentation.rs` → `segment_pages()`

1. Разделение по вертикальной оси (half_width).
2. `crop_to_content` для каждой половины: Otsu → `boundingRect` → ROI.

**Важно:** Сегментация выполняется только для типа страницы `spread`. Для `cover` — см. раздел 6.

### 5.6 Детекция и выравнивание скоса ✅

**Файл:** `src/cv/segmentation.rs` → `detect_skew_angle()`, `rotate_image()`

1. Otsu бинаризация.
2. Горизонтальная проекция (сумма тёмных пикселей по строкам).
3. Поиск пиков (строки с текстом, > 50% среднего).
4. Линейная регрессия по пикам → угол наклона.
5. `getRotationMatrix2D` + `warpAffine` (BORDER_REPLICATE, fill=255).

---

## 6. ДЕТЕКЦИЯ ТИПА СТРАНИЦЫ: ОБЛОЖКА VS РАЗВОРОТ

### 6.1. Проблема

Обложка книги занимает меньше площади, чем разворот. Попытка разделить обложку пополам приводит к:
- Потере контента (часть обложки обрезается)
- Некорректной бинаризации (пустые зоны)
- Ошибочным метаданным в PDF

### 6.2. Алгоритм детекции

**Файл:** `src/cv/segmentation.rs` → `detect_page_type()`

```rust
pub enum PageType {
    /// Разворот: две страницы, разделённые корешком
    Spread,
    /// Обложка: одна страница (фронт/бэк)
    Cover,
}

pub fn detect_page_type(frame: &Mat, vertices: &PageVertices) -> PageType {
    // 1. Вычисляем aspect ratio контура
    let width = vertices.p2.x - vertices.p1.x;
    let height = vertices.p3.y - vertices.p1.y;
    let aspect_ratio = width as f64 / height as f64;
    
    // 2. Анализируем центральную зону (корешок)
    //    Для разворота: тёмная полоса в центре (тень корешка)
    //    Для обложки: нет выраженной центральной тени
    let center_zone = extract_center_zone(frame, vertices, 0.1); // 10% ширины по центру
    let center_brightness = calculate_mean_brightness(center_zone);
    
    // 3. Критерии классификации:
    //    - aspect_ratio < 0.8 → Cover (книга стоит вертикально или наклонена)
    //    - center_brightness < 50 → Spread (выраженная тень корешка)
    //    - Иначе → Cover (нет выраженного корешка)
    
    if aspect_ratio < 0.8 {
        PageType::Cover
    } else if center_brightness < 50.0 {
        PageType::Spread
    } else {
        PageType::Cover
    }
}
```

### 6.3. Логика обработки по типу

| Тип | Обработка |
|-----|-----------|
| `Spread` | Стандартный конвейер: perspective warp → dewarp spine → segment_pages (левая/правая) → skew correction → binarization |
| `Cover` | Упрощённый конвейер: perspective warp → skew correction → binarization (без сегментации и деварпинга) |

### 6.4. Сохранение результата

Для `Cover`:
- Сохраняется как **одна** страница (не две)
- В БД: `left_path` = путь к обложке, `right_path` = NULL
- В PDF: одна страница вместо двух

### 6.5. Интеграция в пайплайн

**Файл:** `src/pipeline.rs` → `process_page()`

```rust
// После детекции вершин:
let page_type = cv::detect_page_type(&rotated_frame, &vertices);

match page_type {
    PageType::Spread => {
        // Полный конвейер с сегментацией
        let corrected = cv::rectify_and_dewarp_page(...)?;
        let (left, right) = cv::segment_pages(&corrected)?;
        // ... обработка обеих страниц
    }
    PageType::Cover => {
        // Упрощённый конвейер без сегментации
        let corrected = cv::perspective_warp_only(...)?;
        // ... обработка одной страницы
    }
}
```

---

## 7. ОБРАБОТКА ИЗОБРАЖЕНИЙ И БИНАРИЗАЦИЯ

### 7.1 Multi-profile фильтрация (E2) ✅

**Файл:** `src/cv/profile_filtering.rs`

| Профиль | Описание | Формат | Экспорт |
|---------|----------|--------|---------|
| `TextBw1bit` | Sauvola + инверсия (белая бумага, чёрный текст) | 1-bit B/W | CCITT G4 TIFF |
| `IllustrationGrayscale8bit` | Гамма (γ=1.2) + CLAHE (clip=2.0, tile=8×8) | 8-bit Gray | PNG |
| `ColorRgb24bit` | Оригинальная палитра, гарантированная BGR 3-канальность | 24-bit RGB | PNG |

Профиль передаётся из Flutter UI через поле `profile` в `ScanTriggerRequest`. При отсутствии — берётся из `calibration.json`.

### 7.2 Адаптивная бинаризация Sauvola ✅

**Файл:** `src/cv/binarization.rs` → `apply_sauvola_threshold(src, k, window_size)`

Формула:
```
T(x, y) = m(x, y) · [1 + k · (s(x, y) / R − 1)]
```

Где:
- `m(x,y)` — локальное среднее через `box_filter(window_size × window_size)`
- `s(x,y)` — стандартное отклонение (√(E[X²] − E[X]²))
- `R` = 128.0 (динамический диапазон 8-bit)
- `k` — коэффициент чувствительности (0.1–0.5, дефолт 0.2)

Реализация через поканальные операции OpenCV с раздельными буферами (`t_factor1` → `t_factor2` → `t_factor3` → `t_factor_final`) для избежания aliasing. Сравнение `CMP_LT` → чёрный текст на белом фоне.

### 7.3 Калибровка с hot-reload (M8) ✅

**Файл:** `src/cv/calibration.rs`

- Файл `calibration.json` в `~/.config/flat-scanner/`: `{"k_factor": 0.2, "window_size": 15, "profile": "text_bw_1bit"}`.
- `CalibrationManager` отслеживает `mtime` файла, перечитывает при изменении (троттлинг 500 мс).
- Методы `reload()` и `save()` — публичный API для Flutter UI (endpoint `/api/v1/calibration`).
- Глобальный экземпляр через `OnceLock` (ленивая инициализация).

### 7.4 Сохранение печатей и штампов 🟡

Алгоритм обязан дифференцировать текст и библиотечные штампы/печати. Синяя или красная печать изолируется в отдельном цветовом канале (инвертированный Cr в YCbCr), очищается от шума бумаги и накладывается поверх бинаризированного текстового слоя.

**Статус:** Реализовано в `src/cv/seal_extraction.rs`, интеграция в пайплайн — в roadmap.

---

## 8. CCITT GROUP 4 И ЭКСПОРТ

### 8.1 Экспорт CCITT G4 TIFF (E1) ✅

**Файл:** `src/cv/ccitt_encoder.rs` → `encode_ccitt_g4_to_file(src, path)`

Прямой FFI к системной libtiff (4.7+):

1. Grayscale → бинаризация (threshold 127.5, THRESH_BINARY).
2. Упаковка битов MSB-first (8 пикселей в байт).
3. `TIFFOpen` → установка тегов:
   - `ImageWidth`, `ImageLength`
   - `BitsPerSample = 1`
   - `Compression = CCITTFAX4` (Group 4)
   - `Photometric = MinIsBlack`
   - `FillOrder = MSB2LSB`
   - `Software` (метаданные программы)
4. `TIFFWriteScanline` построчно.
5. `TIFFClose` → возврат размера файла в байтах.

**Замечание:** Крейт `tiff` (0.11) не поддерживает CCITT G4. OpenCV `imwrite` требует 1-битный вход для G4, но принимает 8-битный с предупреждением. FFI к libtiff решает обе проблемы.

### 8.2 Модуль разборки сторонних PDF (M7) ✅

**Файл:** `src/pdf_importer.rs`

Программа уметь открывать существующие PDF:
- Замена испорченных страниц
- Внедрение пропущенных листов
- Очистка от шума сторонних сканов

Конвейер: векторный текст отделяется от графической подложки, страницы экспортируются как растровые слои для точечной замены.

### 8.3 Сборка финального PDF (M6) ✅

**Файл:** `src/pdf_exporter.rs`

Сборка PDF из CCITT G4 TIFF / PNG страниц через `lopdf`:
- Сохранение метаданных (title, author, subject)
- Оптимизация размера (Flate сжатие)
- Порядок страниц: по `spread_index` ASC, левая → правая

---

## 9. FLUTTER UI (РЕАЛИЗОВАНО)

**Каталог:** `flat-scanner-client/`

### 9.1 Архитектура ScannerBLoC

**Файл:** `lib/domain/scanner_bloc.dart`

| Состояние | Описание |
|-----------|----------|
| `ScannerInitial` | Ожидание подключения сканера |
| `ScannerScanning` | Системный вызов SANE API, блокировка ввода |
| `ScannerSuccess` | Готовое превью с вершинами и временем обработки |
| `ScannerError` | Ошибка с сообщением |

События: `StartScan`, `ResetScan`.

### 9.2 UI редактора сканирования

**Файл:** `lib/presentation/scan_editor_page.dart`

- Выбор профиля (TextBw1bit / IllustrationGrayscale8bit / ColorRgb24bit)
- Кнопка сканирования, отображение вершин и времени обработки
- Опциональный полноэкранный режим (window_manager)
- ThemeService: адаптация под KDE/Breeze + Material 3 (`lib/data/theme_service.dart`)

### 9.3 CustomPainter интерактивной сетки ✅

**Файл:** `lib/presentation/vertex_editor.dart`

- `ScanEditorPainter` с Drag-and-Drop вершин
- `GestureDetector.onPanUpdate` → PATCH `/api/v1/scan/<uuid>/adjust-vertex?index=N&x=X&y=Y`

### 9.4 JSON контракты API

**Запрос** `POST /api/v1/scanner/process`:
```json
{
  "uuid": "book-uuid",
  "threshold_preset": 80,
  "profile": "text_bw_1bit"
}
```

**Ответ** `ScanResponse`:
```json
{
  "status": "PreviewReady",
  "uuid": "book-uuid",
  "vertices": {
    "p1": {"x": 100, "y": 200},
    "p2": {"x": 3200, "y": 195},
    "p3": {"x": 3180, "y": 4500},
    "p4": {"x": 95, "y": 4520}
  },
  "execution_time_ms": 142
}
```

### 9.5 Конфигурация клиента

**Файл:** `lib/data/api_service.dart`

- При инициализации читает `~/.config/flat-scanner/config.toml`
- Парсит секции `[server]` (host, port) и `[paths]`
- Использует `Uri.http` для формирования запросов (без ручной конкатенации)
- Реализует `dispose()` для закрытия HTTP-сессии

---

## 10. ОТКАЗОУСТОЙЧИВОСТЬ И СЕССИИ (РЕАЛИЗОВАНО)

### 10.1 SQLite транзакционная модель

**Файл:** `src/session_store.rs`

- Таблица `books`: uuid, name, start_date, total_pages, status.
- Таблица `spreads`: book_uuid, spread_index, left_path, right_path, left_vertices[4], right_vertices[4], threshold_k, status.
- Атомарные INSERT+UPDATE в `BEGIN TRANSACTION...COMMIT`.
- `journal_mode=WAL`.
- Система миграций для совместимости со старыми БД.

### 10.2 Горячий рестарт сессии

**Файл:** `src/session_recovery.rs`

При перезапуске:
1. Чтение последнего незавершённого UUID (`SELECT * FROM books WHERE status='in_progress' ORDER BY updated_at DESC LIMIT 1`).
2. Восстановление очереди спредов.
3. Открытие книги на прерванной странице.

### 10.3 Race Condition Guard

При пропадании питания во время записи координат — повреждение индекса. Решение: двойное журналирование (`/tmp/<uuid>.pending` → подтверждение → WAL checkpoint). Очистка устаревших pending-файлов (старше 24 часов).

### 10.4 Single Writer Pattern

**Файл:** `src/write_queue.rs`

- `tokio::sync::mpsc::unbounded_channel` для задач записи.
- Один фоновый воркер-писатель выполняет транзакции последовательно.
- Чтения (`get_book_progress`, `get_pending_pages`) остаются параллельными.
- Обработка ошибок записи с ретраями.

---

## 11. БЕЗОПАСНОСТЬ И ВАЛИДАЦИЯ

### 11.1 Path Traversal Protection

**Файл:** `src/pipeline.rs` → `safe_resolve_path()`

- Жёсткая проверка `book_id`: только алфавитно-цифровые символы и дефисы.
- Результативный путь обязан начинаться с базовой директории.
- Запрет на символы разделителей путей во входных данных.

### 11.2 OpenCV Safe-Guards

**Файл:** `src/cv/warping.rs` → `validate_page_geometry()`

Перед `get_perspective_transform`:
1. Строго 4 точки.
2. Площадь контура ≥ 15% кадра.
3. Выпуклость (`is_contour_convex`).

Сбой → `Result::Err(DigitizationError::InvalidPageGeometry)`, страница → статус FAILED + `error_message`.

### 11.3 SANE FFI Guard

- Все блокирующие вызовы в `tokio::task::spawn_blocking`.
- RAII: `SaneScanner` реализует `Drop` для гарантированного закрытия.
- Таймауты на все SANE-операции.

### 11.4 Лимиты загрузки

- `tower-http` middleware `RequestBodyLimitLayer` (50MB).
- Предотвращение memory exhaustion при загрузке больших файлов.

### 11.5 Типизированные ошибки

**Файл:** `src/cv/mod.rs` → `DigitizationError`

```rust
#[derive(Debug, thiserror::Error)]
pub enum DigitizationError {
    #[error("Ошибка подсистемы SANE FFI: {0}")]
    SaneError(String),
    
    #[error("Критический сбой геометрии страницы: {0}")]
    InvalidPageGeometry(String),
    
    #[error("Исключение ядра OpenCV C++ (Перехвачено): {0}")]
    OpenCVPanic(String),
    
    #[error("Ошибка транзакции базы данных SQLite: {0}")]
    DatabaseError(#[from] rusqlite::Error),
    
    #[error("Ошибка ввода-вывода файловой системы: {0}")]
    IoError(#[from] std::io::Error),
}
```

---

## 12. УПАКОВКА И ДИСТРИБУЦИЯ

### Сервер

| Файл | Назначение |
|------|------------|
| `flat-scanner-server/PKGBUILD` | Arch Linux пакет `flat-scanner-server` |
| `flat-scanner-server/flat-scanner-server.service` | systemd unit (After=network.target, Restart=on-failure) |
| `flat-scanner-server/config.example.toml` | Шаблон конфигурации (host, port, paths) |
| `flat-scanner-server/README.md` | Инструкция по установке и запуску |

### Клиент

| Файл | Назначение |
|------|------------|
| `flat-scanner-client/PKGBUILD` | Arch Linux пакет `flat-scanner-client` |
| `flat-scanner-client/flat-scanner-client.desktop` | .desktop entry для меню приложений |
| `flat-scanner-client/README.md` | Инструкция по установке и запуску |

### Установка (Arch Linux)

```bash
# Сервер
makepkg -si flat-scanner-server/PKGBUILD
systemctl enable --now flat-scanner-server

# Клиент
makepkg -si flat-scanner-client/PKGBUILD
```

### PKGBUILD: создание каталогов

При установке пакет обязан создать каталоги из конфига:
```bash
package() {
    # ...
    install -Dm755 /dev/null "$pkgdir/.local/share/flat-scanner/raw"
    install -Dm755 /dev/null "$pkgdir/.local/share/flat-scanner/processed"
    install -Dm755 /dev/null "$pkgdir/.local/share/flat-scanner/export"
    install -Dm755 /dev/null "$pkgdir/.local/share/flat-scanner/import"
}
```

---

## 13. ИНЖЕНЕРНЫЕ ДИРЕКТИВЫ

### Правила генерации кода

1. **Запрет плейсхолдеров.** Каждая функция — 100% реализация.
2. **Безопасные обёртки.** `unsafe` только в FFI-блоках (libtiff, прямой доступ к Mat).
3. **Обработка ошибок.** `unwrap()` и `panic!` запрещены. Все сбои → `Result<T, String>`.
4. **Модульность.** Каждая структура/функция — отдельный файл.
5. **Zero-Copy.** Избегать глубокого копирования RAW-буферов между слоями.
6. **Превью.** Передача во Flutter только в JPEG/WebP или 1-bit после бинаризации.

### Release профиль

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
```

### Rust Edition

- **Edition 2024** для всех бэкенд-компонентов.
- **Stable** toolchain, без nightly фич.

### Лицензия

- **AGPL-3.0-only** для всего программного обеспечения.

---

## 14. ТЕКУЩЕЕ СОСТОЯНИЕ КОДОВОЙ БАЗЫ

### Структура файлов

| Файл | Назначение | Статус |
|------|-----------|--------|
| `Cargo.toml` | Зависимости (clap, tokio, axum, opencv, serde, tiff), release LTO+opt3 | ✅ |
| `build.rs` | Линковка SANE через pkg-config | ✅ |
| `src/main.rs` | Двухрежимное ядро: CLI / Axum Web API; multi-profile; calibration | ✅ |
| `src/config.rs` | Загрузка config.toml + CLI-флаги (--host/--port) | ✅ |
| `src/sane_core.rs` | Автообнаружение + захват TIFF RAW @300 DPI → Mat | ✅ |
| `src/pipeline.rs` | Сквозной конвейер: SANE → CV → экспорт | ✅ |
| `src/routes.rs` | Axum HTTP routes (scan API) | ✅ |
| `src/cv/mod.rs` | Реэкспорт публичных API; `DigitizationError` | ✅ |
| `src/cv/segmentation.rs` | coarse_mask, isolate_side_artifacts, process_book_contours, segment_pages, detect_skew_angle, rotate_image | ✅ |
| `src/cv/binarization.rs` | Sauvola threshold (полная формула, раздельные буферы) | ✅ |
| `src/cv/warping.rs` | perspective_warp, dewarp_spine (Hough + цилиндрическая модель + remap) | ✅ |
| `src/cv/ccitt_encoder.rs` | FFI libtiff: CCITT G4 TIFF экспорт | ✅ |
| `src/cv/profile_filtering.rs` | Multi-profile: TextBw1bit / IllustrationGrayscale8bit / ColorRgb24bit | ✅ |
| `src/cv/calibration.rs` | Hot-reload k_factor/window_size/profile из calibration.json | ✅ |
| `src/cv/seal_extraction.rs` | HSV-детекция печатей | 🟡 |
| `src/session_store.rs` | SQLite (rusqlite): books + spreads, WAL, транзакции | ✅ |
| `src/session_recovery.rs` | Hot restart: восстановление UUID + очередь + pending-журналирование | ✅ |
| `src/write_queue.rs` | Single Writer + FIFO-очередь | ✅ |
| `src/pdf_exporter.rs` | Сборка PDF из TIFF/PNG через lopdf | ✅ |
| `src/pdf_importer.rs` | Разборка сторонних PDF через pdftoppm + lopdf | ✅ |
| `flat-scanner-client/lib/` | Flutter клиент: BLoC, ApiService, ThemeService, ScanEditorPage | ✅ |
| `flat-scanner-server/PKGBUILD` | Arch Linux пакет сервера | ✅ |
| `flat-scanner-server/flat-scanner-server.service` | systemd unit | ✅ |
| `flat-scanner-client/PKGBUILD` | Arch Linux пакет клиента | ✅ |
| `flat-scanner-client/flat-scanner-client.desktop` | .desktop entry | ✅ |

### Реализовано ✅

| # | Функциональность | Модуль |
|---|-----------------|--------|
| R1 | Двухрежимный движок (CLI / Web) | `main.rs` |
| R2 | SANE интеграция (автообнаружение + захват в RAM) | `sane_core.rs` |
| R3 | Axum REST API (health, init, process, calibration, vertex, export/import PDF) | `main.rs`, `routes.rs` |
| R4 | CLI конвейер: rotate → contours → split → Sauvola → CCITT G4 | `main.rs` |
| R5 | Sauvola бинаризация (полная формула) | `cv/binarization.rs` |
| R6 | Coarse masking (мультимасштаб, 3 масштаба, closing) | `cv/segmentation.rs` |
| R7 | Изоляция боковых артефактов (градиентный анализ) | `cv/segmentation.rs` |
| R8 | Детекция вершин P₁..P₄ (contours + approxPolyDP + minAreaRect) | `cv/segmentation.rs` |
| R9 | Перспективная трансформация (getPerspectiveTransform + warpPerspective) | `cv/warping.rs` |
| R10 | Цилиндрический деварпинг (Hough + spine shadow + remap) | `cv/warping.rs` |
| R11 | Сегментация разворота (split + crop_to_content) | `cv/segmentation.rs` |
| R12 | Детекция и выравнивание скоса (проекция + регрессия) | `cv/segmentation.rs` |
| R13 | CCITT G4 TIFF экспорт (FFI libtiff) | `cv/ccitt_encoder.rs` |
| R14 | Multi-profile фильтрация (3 профиля) | `cv/profile_filtering.rs` |
| R15 | Hot-reload калибровки (mtime tracking, 500ms throttle) | `cv/calibration.rs` |
| R16 | Result<T, String> обработка ошибок (без unwrap/panic) | весь код |
| R17 | Release профиль LTO + opt3 + codegen-units=1 + panic=abort | `Cargo.toml` |
| R18 | SQLite Session Store (books + spreads, WAL, транзакции) | `session_store.rs` |
| R19 | Hot restart сессии (pending-журналирование, WAL checkpoint) | `session_recovery.rs` |
| R20 | CLI-флаги --host/--port + config.toml | `config.rs` |
| R21 | Flutter клиент (BLoC + ApiService + ThemeService + ScanEditorPage) | `flat-scanner-client/` |
| R22 | PKGBUILD + systemd + .desktop (сервер + клиент) | `PKGBUILD`, `.service`, `.desktop` |
| R23 | Single Writer + FIFO-очередь для SQLite | `write_queue.rs` |
| R24 | Path Traversal Protection | `pipeline.rs` |
| R25 | OpenCV Safe-Guards (геометрическая валидация) | `cv/warping.rs` |
| R26 | SANE FFI Guard (RAII, таймауты, spawn_blocking) | `sane_core.rs` |
| R27 | PDF экспорт (lopdf) | `pdf_exporter.rs` |
| R28 | PDF импорт (pdftoppm + lopdf) | `pdf_importer.rs` |
| R29 | Лимит загрузки (50MB) | `main.rs` |
| R30 | Типизированные ошибки (DigitizationError) | `cv/mod.rs` |

### Не реализовано / В работе 🟡

| # | Функциональность | Приоритет | Статус |
|---|-----------------|-----------|--------|
| M1 | Детекция типа страницы (обложка vs разворот) | HIGH | ❌ |
| M2 | Интеграция seal_extraction в пайплайн | MEDIUM | 🟡 |
| M3 | Zero-Copy передача превью во Flutter (JPEG/WebP) | LOW | ❌ |
| M4 | Единый config.toml для сервера и клиента | HIGH | ❌ |
| M5 | XDG paths в config.toml | HIGH | ❌ |

### Тесты

34+ unit-теста, все проходят (`cargo test`):
- `calibration`: default_params, profile_parsing, save_and_reload, json_deserialization
- `ccitt_encoder`: encode_ccitt_g4, encode_ccitt_g4_all_white
- `profile_filtering`: profile_from_str, profile_text_bw, profile_grayscale, profile_color
- `warping`: perspective_warp_identity, detect_spine_shadow, build_cylindrical_deformation, apply_cylindrical_correction
- `routes`: scan_request_deserialize, error_mapping_sane, error_mapping_geometry, error_mapping_internal
- `config`: default_config_is_local, parse_toml_config, cli_overrides_win
- `pipeline`: fast_binarize_small_image, page_processor_new
- `main`: adjust_vertex_query_deserialize, adjust_vertex_response_serialize, adjust_vertex_updates_correct_point

---

## 15. ROADMAP

### Этап A: Критические исправления Rust Core ✅ ЗАВЕРШЁН

| Задача | Статус |
|--------|--------|
| A1: Реальная детекция вершин P₁..P₄ | ✅ |
| A2: Coarse masking (мультимасштаб) | ✅ |
| A3: Реальный SANE захват в Web API | ✅ |

### Этап B: Computer Vision ✅ ЗАВЕРШЁН

| Задача | Статус |
|--------|--------|
| B1: Деварпинг корешка (цилиндрическая трансформация) | ✅ |
| B2: Изоляция боковых артефактов | ✅ |
| B3: Улучшение coarse masking (мультимасштаб + closing) | ✅ |

### Этап C: Flutter Desktop клиент ✅ ЗАВЕРШЁН

| Задача | Статус |
|--------|--------|
| C1: Генерация проекта Flutter Linux + HTTP к Axum | ✅ |
| C2: ScannerBLoC (Initial→Scanning→Success/Error) | ✅ |
| C3: ScanEditorPage + ThemeService + fullscreen | ✅ |
| C4: CustomPainter Drag-and-Drop вершин | ✅ |

### Этап D: Session Store + Hot Restart ✅ ЗАВЕРШЁН

| Задача | Статус |
|--------|--------|
| D1: SQLite (rusqlite): books + spreads, WAL, транзакции | ✅ |
| D2: Hot restart: восстановление UUID + очередь + pending-журналирование | ✅ |

### Этап E: PDF Export + Multi-profile ✅ ЗАВЕРШЁН

| Задача | Статус |
|--------|--------|
| E1: CCITT G4 TIFF экспорт | ✅ |
| E2: Multi-profile фильтрация (3 профиля) | ✅ |

### Этап F: Упаковка и дистрибуция ✅ ЗАВЕРШЁН

| Задача | Статус |
|--------|--------|
| F1: PKGBUILD сервера + systemd unit + config.toml | ✅ |
| F2: PKGBUILD клиента + .desktop entry | ✅ |

### Этап G: Безопасность и валидация ✅ ЗАВЕРШЁН

| Задача | Статус |
|--------|--------|
| G1: Path Traversal Protection | ✅ |
| G2: OpenCV Safe-Guards | ✅ |
| G3: SANE FFI Guard (RAII, таймауты) | ✅ |
| G4: Single Writer + FIFO-очередь | ✅ |
| G5: Лимит загрузки (50MB) | ✅ |
| G6: Типизированные ошибки | ✅ |

### Этап H: Унификация конфигурации 🟡 В РАБОТЕ

| Задача | Приоритет | Статус |
|--------|-----------|--------|
| H1: Единый config.toml для сервера и клиента | HIGH | ❌ |
| H2: XDG paths в config.toml | HIGH | ❌ |
| H3: Резолвинг ~ в Rust config.rs | HIGH | ❌ |
| H4: Парсинг TOML в Dart клиенте | HIGH | ❌ |
| H5: Создание каталогов в PKGBUILD | MEDIUM | ❌ |

### Этап I: Детекция типа страницы 🟡 В РАБОТЕ

| Задача | Приоритет | Статус |
|--------|-----------|--------|
| I1: Алгоритм detect_page_type (обложка vs разворот) | HIGH | ❌ |
| I2: Интеграция в pipeline.rs | HIGH | ❌ |
| I3: Логика обработки Cover (без сегментации) | HIGH | ❌ |
| I4: Сохранение Cover как одной страницы | HIGH | ❌ |

### Этап J: Дополнительные модули 🟡 MEDIUM

| Задача | Описание |
|--------|----------|
| J1 | Интеграция seal_extraction в пайплайн |
| J2 | Zero-Copy передача превью во Flutter (JPEG/WebP) |
| J3 | Оптимизация detect_spine_shadow через OpenCV reduce |
| J4 | Пул буферов для Sauvola |
| J5 | Сжатие изображений в PDF (Flate) |

---

## ПРИЛОЖЕНИЕ: Зависимости

| Крейт | Версия | Назначение |
|-------|--------|------------|
| `clap` | 4.6 | CLI-парсер |
| `tokio` | 1.53 | Асинхронный runtime |
| `axum` | 0.8.9 | Web-сервер (REST API) |
| `tower-http` | 0.7 | CORS middleware, RequestBodyLimit |
| `serde` / `serde_json` | 1.0 | JSON сериализация |
| `opencv` | 0.100 | Computer Vision (OpenCV 4.x) |
| `tiff` | 0.11 | (запасной, основной путь — FFI libtiff) |
| `pkg-config` | 0.3.34 | build-dep: линковка SANE |
| `rusqlite` | 0.40 | SQLite Session Store (WAL) |
| `toml` | 0.8 | Парсинг config.toml |
| `uuid` | 1.x | UUID генерация сессий |
| `lopdf` | 0.34 | PDF экспорт/импорт |
| `thiserror` | 1.0 | Типизированные ошибки |

### Системные зависимости (Arch Linux)

```bash
sudo pacman -S opencv sane-backends libtiff poppler-utils