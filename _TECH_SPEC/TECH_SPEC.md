# Канонисса-Библиотека — Единая техническая спецификация

**Версия:** 2.1  
**Дата актуализации:** 16 августа 2026 г.  
**Автор:** Valeria Fadeeva <valeria.fadeeva.me@gmail.com>  
**Платформа:** AMD Ryzen 7 5700X / Radeon 9070 XT 16GB / 128 GB DDR4 / Arch Linux  
**Стек:** Flutter Desktop (Dart) ↔ Rust Core Engine (Axum REST API) + OpenCV 4.x / SANE / libtiff

---

## СОДЕРЖАНИЕ

1. [Назначение системы](#1-назначение-системы)
2. [Архитектура слоёв](#2-архитектура-слоев)
3. [Входные данные (SANE)](#3-входные-данные-sane)
4. [Computer Vision конвейер](#4-computer-vision-конвейер)
5. [Обработка изображений и бинаризация](#5-обработка-изображений-и-бинаризация)
6. [CCITT Group 4 и экспорт](#6-ccitt-group-4-и-экспорт)
7. [Flutter UI](#7-flutter-ui)
8. [Отказоустойчивость и сессии](#8-отказоустойчивость-и-сессии)
9. [Упаковка и дистрибуция](#9-упаковка-и-дистрибуция)
10. [Инженерные директивы](#10-инженерные-директивы)
11. [Текущее состояние кодовой базы](#11-текущее-состояние-кодовой-базы)
12. [Roadmap](#12-roadmap)

---

## 1. НАЗНАЧЕНИЕ СИСТЕМЫ

Высокоскоростная потоковая оцифровка архивных и библиотечных книг (включая издания старше 75 лет, перешедшие в общественное достояние) на планшетном сканере формата А3 (EPSON GT-20000 / Canon LiDE) в условиях незакрытой крышки.

### Метрики рантайма

| Метрика | Значение |
|---------|----------|
| Производительность | ≥ 3368 страниц/мес (~5ч 20мин оператора по Pomodoro) |
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
               │ HTTP REST API (localhost:8080)
               ▼
┌─────────────────────────────────────────┐
│ Rust Core Engine                        │
│ ─────────────────────────────────────── │
│ Axum Web Server                         │
│ ├─ GET  /api/v1/health                  │
│ ├─ POST /api/v1/scanner/init            │
│ └─ POST /api/v1/scanner/process         │
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

1. **Web Mode** (дефолт): Axum-сервер на `127.0.0.1:8080` (настраивается через `--host`/`--port` или `config.toml`) для взаимодействия с Flutter через JSON-API.
2. **CLI Mode** (`--cli` флаг): автономный конвейер без сервера — чтение файла или прямой захват со сканера → обработка → сохранение TIFF/PNG в папку `./split`.

### Коллизия А: Фриз UI при маршалинге RAW-буфера

При захвате полного кадра А3 @ 300 DPI получается растр ~3300×4700 px (~45 MB RGB). Передача такого буфера во Flutter требует Zero-Copy обёртки или превью JPEG/WebP — иначе фриз UI на 300–450 мс.

**Решение:** Rust-ядро преобразует RAW-буфер в лёгкий JPEG/WebP и отдаёт во Flutter указатель на `Uint8List` через Zero-Copy.

---

## 3. ВХОДНЫЕ ДАННЫЕ (SANE)

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

---

## 4. COMPUTER VISION КОНВЕЙЕР

### 4.1 Coarse Masking (B3) ✅

**Файл:** `src/cv/segmentation.rs` → `coarse_mask()`

Мультимасштабный алгоритм для изоляции зоны книги от потолка/ламп:

1. Grayscale.
2. Три масштаба размытия (51/101/201) → Otsu INV на каждом.
3. На каждом масштабе — кандидаты контуров, оценка `solidity` (площадь / площадь выпуклой оболочки).
4. Лучший кандидат (макс. solidity при площади 10%–95% кадра).
5. Морфологическое закрытие (ellipse 15×15, 2 итерации) для заполнения провалов.
6. `bitwise_and` к исходному изображению.

### 4.2 Изоляция боковых артефактов (B2) ✅

**Файл:** `src/cv/segmentation.rs` → `isolate_side_artifacts()`

Градиентный анализ плотности по периферии macro-contour:

1. Извлечение полосы (band) шириной 8 px вокруг контура (XOR маска с эродированной).
2. Подсчёт частоты чередований светлых/тёмных пикселей вдоль строк.
3. Если частота > 30% — паттерн "боковушек" обнаружен.
4. Эрозия маски на 3 итерации (сдвиг рамки внутрь).

### 4.3 Детекция четырёх вершин ✅

**Файл:** `src/cv/segmentation.rs` → `process_book_contours()`

Пайплайн:
1. Coarse masking → отсечение артефактов.
2. Grayscale → Otsu бинаризация (THRESH_BINARY_INV).
3. `findContours` (RETR_EXTERNAL) → крупнейший контур по площади (мин. 1% кадра).
4. B2: изоляция боковых артефактов → пересчёт контура из улучшенной маски.
5. `approxPolyDP(ε = perimeter × 0.02)` → если 4 точки → сортировка TL→TR→BR→BL.
6. Fallback: `minAreaRect` → 4 угла → сортировка.

Сортировка: диагональная проекция (x+y), разделение на верхнюю/нижнюю пары, сортировка внутри по X.

### 4.4 Деварпинг корешка (B1) ✅

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

### 4.5 Сегментация разворота ✅

**Файл:** `src/cv/segmentation.rs` → `segment_pages()`

1. Разделение по вертикальной оси (half_width).
2. `crop_to_content` для каждой половины: Otsu → `boundingRect` → ROI.

### 4.6 Детекция и выравнивание скоса ✅

**Файл:** `src/cv/segmentation.rs` → `detect_skew_angle()`, `rotate_image()`

1. Otsu бинаризация.
2. Горизонтальная проекция (сумма тёмных пикселей по строкам).
3. Поиск пиков (строки с текстом, > 50% среднего).
4. Линейная регрессия по пикам → угол наклона.
5. `getRotationMatrix2D` + `warpAffine` (BORDER_REPLICATE, fill=255).

---

## 5. ОБРАБОТКА ИЗОБРАЖЕНИЙ И БИНАРИЗАЦИЯ

### 5.1 Multi-profile фильтрация (E2) ✅

**Файл:** `src/cv/profile_filtering.rs`

| Профиль | Описание | Формат | Экспорт |
|---------|----------|--------|---------|
| `TextBw1bit` | Sauvola + инверсия (белая бумага, чёрный текст) | 1-bit B/W | CCITT G4 TIFF |
| `IllustrationGrayscale8bit` | Гамма (γ=1.2) + CLAHE (clip=2.0, tile=8×8) | 8-bit Gray | PNG |
| `ColorRgb24bit` | Оригинальная палитра, гарантированная BGR 3-канальность | 24-bit RGB | PNG |

Профиль передаётся из Flutter UI через поле `profile` в `ScanTriggerRequest`. При отсутствии — берётся из `calibration.json`.

### 5.2 Адаптивная бинаризация Sauvola ✅

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

### 5.3 Калибровка с hot-reload (M8) ✅

**Файл:** `src/cv/calibration.rs`

- Файл `calibration.json` в корне проекта: `{"k_factor": 0.2, "window_size": 15, "profile": "text_bw_1bit"}`.
- `CalibrationManager` отслеживает `mtime` файла, перечитывает при изменении (троттлинг 500 мс).
- Методы `reload()` и `save()` — публичный API для Flutter UI (endpoint `/api/v1/calibration` — TODO).
- Глобальный экземпляр через `OnceLock` (ленивая инициализация).

### 5.4 Сохранение печатей и штампов ❌

Алгоритм обязан дифференцировать текст и библиотечные штампы/печати. Синяя или красная печать изолируется в отдельном цветовом канале (инвертированный Cr в YCbCr), очищается от шума бумаги и накладывается поверх бинаризированного текстового слоя.

**Статус:** Не реализовано.

---

## 6. CCITT GROUP 4 И ЭКСПОРТ

### 6.1 Экспорт CCITT G4 TIFF (E1) ✅

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
4. `TIFFWriteScanline` построчно.
5. `TIFFClose` → возврат размера файла в байтах.

**Замечание:** Крейт `tiff` (0.11) не поддерживает CCITT G4. OpenCV `imwrite` требует 1-битный вход для G4, но принимает 8-битный с предупреждением. FFI к libtiff решает обе проблемы.

### 6.2 Модуль разборки сторонних PDF (M7) ❌

Программа должна уметь открывать существующие PDF:
- Замена испорченных страниц
- Внедрение пропущенных листов
- Очистка от шума сторонних сканов

Конвейер: векторный текст отделяется от графической подложки, страницы экспортируются как растровые слои для точечной замены.

**Статус:** Не реализовано. Требуется `src/pdf_importer.rs` с крейтом `pdf-extract`/`poppler` или `pdftoppm`.

---

## 7. FLUTTER UI (РЕАЛИЗОВАНО)

**Каталог:** `flat-scanner-client/`

### 7.1 Архитектура ScannerBLoC

**Файл:** `lib/domain/scanner_bloc.dart`

| Состояние | Описание |
|-----------|----------|
| `ScannerInitial` | Ожидание подключения сканера |
| `ScannerScanning` | Системный вызов SANE API, блокировка ввода |
| `ScannerSuccess` | Готовое превью с вершинами и временем обработки |
| `ScannerError` | Ошибка с сообщением |

События: `StartScan`, `ResetScan`.

### 7.2 UI редактора сканирования

**Файл:** `lib/presentation/scan_editor_page.dart`

- Выбор профиля (TextBw1bit / IllustrationGrayscale8bit / ColorRgb24bit)
- Кнопка сканирования, отображение вершин и времени обработки
- Опциональный полноэкранный режим (window_manager)
- ThemeService: адаптация под KDE/Breeze + Material 3 (`lib/data/theme_service.dart`)

### 7.3 CustomPainter интерактивной сетки (отложено)

- `ScanEditorPainter` с Drag-and-Drop вершин — на следующий этап.
- `GestureDetector.onPanUpdate` → PATCH `/api/v1/scan/<uuid>/adjust-vertex?index=N&x=X&y=Y`.

### 7.4 JSON контракты API

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

---

## 8. ОТКАЗОУСТОЙЧИВОСТЬ И СЕССИИ (РЕАЛИЗОВАНО)

### 8.1 SQLite транзакционная модель

**Файл:** `src/session_store.rs`

- Таблица `books`: uuid, name, start_date, total_pages, status.
- Таблица `spreads`: book_uuid, spread_index, left_path, right_path, left_vertices[4], right_vertices[4], threshold_k, status.
- Атомарные INSERT+UPDATE в `BEGIN TRANSACTION...COMMIT`.
- `journal_mode=WAL`.

### 8.2 Горячий рестарт сессии

**Файл:** `src/session_recovery.rs`

При перезапуске:
1. Чтение последнего незавершённого UUID (`SELECT * FROM books WHERE status='in_progress' ORDER BY updated_at DESC LIMIT 1`).
2. Восстановление очереди спредов.
3. Открытие книги на прерванной странице.

### 8.3 Коллизия В: Race Condition

При пропадании питания во время записи координат — повреждение индекса. Решение: двойное журналирование (`/tmp/<uuid>.pending` → подтверждение → WAL checkpoint). Очистка устаревших pending-файлов (старше 24 часов).

---

## 9. УПАКОВКА И ДИСТРИБУЦИЯ

### Сервер

| Файл | Назначение |
|------|------------|
| `flat-scanner-server/PKGBUILD` | Arch Linux пакет `flat-scanner-server` |
| `flat-scanner-server/flat-scanner-server.service` | systemd unit (After=network.target, Restart=on-failure) |
| `flat-scanner-server/config.example.toml` | Шаблон конфигурации (host, port, device) |
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

---

## 10. ИНЖЕНЕРНЫЕ ДИРЕКТИВЫ

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

---

## 11. ТЕКУЩЕЕ СОСТОЯНИЕ КОДОВОЙ БАЗЫ

### Структура файлов

| Файл | Назначение | Статус |
|------|-----------|--------|
| `Cargo.toml` | Зависимости (clap, tokio, axum, opencv, serde, tiff), release LTO+opt3 | ✅ |
| `build.rs` | Линковка SANE через pkg-config | ✅ |
| `src/main.rs` | Двухрежимное ядро: CLI / Axum Web API; multi-profile; calibration | ✅ |
| `src/sane_core.rs` | Автообнаружение + захват TIFF RAW @300 DPI → Mat | ✅ |
| `src/cv/mod.rs` | Реэкспорт публичных API; `rectify_and_dewarp_page` | ✅ |
| `src/cv/segmentation.rs` | coarse_mask, isolate_side_artifacts, process_book_contours, segment_pages, detect_skew_angle, rotate_image | ✅ |
| `src/cv/binarization.rs` | Sauvola threshold (полная формула, раздельные буферы) | ✅ |
| `src/cv/warping.rs` | perspective_warp, dewarp_spine (Hough + цилиндрическая модель + remap) | ✅ |
| `src/cv/ccitt_encoder.rs` | FFI libtiff: CCITT G4 TIFF экспорт | ✅ |
| `src/cv/profile_filtering.rs` | Multi-profile: TextBw1bit / IllustrationGrayscale8bit / ColorRgb24bit | ✅ |
| `src/cv/calibration.rs` | Hot-reload k_factor/window_size/profile из calibration.json | ✅ |
| `src/session_store.rs` | SQLite (rusqlite): books + spreads, WAL, транзакции | ✅ |
| `src/session_recovery.rs` | Hot restart: восстановление UUID + очередь + pending-журналирование | ✅ |
| `src/config.rs` | Загрузка config.toml + CLI-флаги (--host/--port) | ✅ |
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
| R3 | Axum REST API (health, init, process) с CORS | `main.rs` |
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

### Не реализовано ❌

| # | Функциональность | Приоритет |
|---|-----------------|-----------|
| M3 | REST endpoint `/api/v1/calibration` (GET/POST) | MEDIUM |
| M4 | REST endpoint `/api/v1/scan/<uuid>/adjust-vertex` (PATCH) | MEDIUM |
| M5 | Сохранение печатей/штампов (YCbCr Cr-канал) | MEDIUM |
| M6 | Модуль разборки сторонних PDF (`pdf_importer.rs`) | MEDIUM |
| M7 | Сборка финального PDF из CCITT G4 страниц | MEDIUM |
| M8 | Zero-Copy передача превью во Flutter (JPEG/WebP) | LOW |

### Тесты

34 unit-теста, все проходят (`cargo test`):
- `calibration`: default_params, profile_parsing, save_and_reload, json_deserialization
- `ccitt_encoder`: encode_ccitt_g4, encode_ccitt_g4_all_white
- `profile_filtering`: profile_from_str, profile_text_bw, profile_grayscale, profile_color
- `warping`: perspective_warp_identity, detect_spine_shadow, build_cylindrical_deformation, apply_cylindrical_correction

---

## 12. ROADMAP

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
| C4: CustomPainter Drag-and-Drop вершин | ⏳ отложено |

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

### Этап G: Дополнительные модули 🟡 MEDIUM

| Задача | Описание |
|--------|----------|
| G1 | REST endpoint `/api/v1/calibration` (GET/POST) |
| G2 | REST endpoint `/api/v1/scan/<uuid>/adjust-vertex` (PATCH) |
| G3 | Сохранение печатей/штампов (YCbCr) |
| G4 | Модуль разборки сторонних PDF |
| G5 | Сборка финального PDF из CCITT G4 страниц |
| G6 | CustomPainter Drag-and-Drop вершин (C4) |

---

## ПРИЛОЖЕНИЕ: Зависимости

| Крейт | Версия | Назначение |
|-------|--------|------------|
| `clap` | 4.6 | CLI-парсер |
| `tokio` | 1.53 | Асинхронный runtime |
| `axum` | 0.8.9 | Web-сервер (REST API) |
| `tower-http` | 0.7 | CORS middleware |
| `serde` / `serde_json` | 1.0 | JSON сериализация |
| `opencv` | 0.100 | Computer Vision (OpenCV 4.x) |
| `tiff` | 0.11 | (запасной, основной путь — FFI libtiff) |
| `pkg-config` | 0.3.34 | build-dep: линковка SANE |
| `rusqlite` | 0.31 | SQLite Session Store (WAL) |
| `toml` | 0.8 | Парсинг config.toml |
| `uuid` | 1.x | UUID генерация сессий |

### Системные зависимости (Arch Linux)

```bash
sudo pacman -S opencv sane-backends libtiff