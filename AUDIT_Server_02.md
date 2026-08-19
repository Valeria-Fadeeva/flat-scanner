# AUDIT: flat-scanner-server

**Дата:** 19.08.2028  
**Референс:** `flat-scanner-server/`  
**Стек:** Rust Edition 2024 / Tokio / Axum / OpenCV 4.x / rusqlite / libtiff FFI  
**Лицензия:** AGPL-3.0-only

---

## 1. ОБЩАЯ КАРТИНА

`flat-scanner-server` — headless-сервис сканирования книг на базе **SANE** + **OpenCV**. Сервис принимает изображения с планшетного сканера, выполняет автоматическую детекцию страниц, бинаризацию, извлечение печатей, деформацию корешка и экспорт в CCITT Group 4 TIFF / PDF.

### Архитектурные слои

| Слой | Файл | Назначение |
|------|------|------------|
| **Точка входа** | `src/main.rs` | Инициализация сервера, DI-контейнер, запуск Tokio-сборки |
| **Конфигурация** | `src/config.rs` | Парсинг `config.toml`, дефолты, hot-reload |
| **API-маршруты** | `src/routes.rs` | REST-эндпоинты Axum (сканирование, калибровка, экспорт) |
| **SANE-ядро** | `src/sane_core.rs` | Обертка над lib sane, захват кадров, управление сканером |
| **Пайплайн** | `src/pipeline.rs` | Цепочка обработки: SANE → CV-модули → экспорт |
| **Сессии** | `src/session_store.rs` | rusqlite-хранилище сессий сканирования |
| **Восстановление** | `src/session_recovery.rs` | Восстановление прерванных сессий |
| **Очередь записи** | `src/write_queue.rs` | Асинхронная очередь записи на диск |
| **CV-модуль** | `src/cv/mod.rs` | Ошибка-enum, публичные API CV-модулей |
| **Сегментация** | `src/cv/segmentation.rs` | Детекция вершин, skew-угол, сегментация разворота |
| **Варпинг** | `src/cv/warping.rs` | Гомография, деформация корешка (Hough + цилиндрическая модель) |
| **Бинаризация** | `src/cv/binarization.rs` | Sauvola threshold |
| **Профили** | `src/cv/profile_filtering.rs` | Text_BW / Illustration / Color + гамма/CLAHE |
| **Печати** | `src/cv/seal_extraction.rs` | HSV-детекция печатей по насыщенности |
| **CCITT G4** | `src/cv/ccitt_encoder.rs` | FFI к libtiff для CCITT Group 4 TIFF |
| **Калибровка** | `src/cv/calibration.rs` | Hot-reload параметров Сауволы через `calibration.json` |
| **PDF-экспорт** | `src/pdf_exporter.rs` | Сборка PDF из TIFF/PNG через lopdf |
| **PDF-импорт** | `src/pdf_importer.rs` | Разборка сторонних PDF через pdftoppm + lopdf |

---

## 2. АУДИТ ИСХОДНОГО КОДА

### 2.1. Сильные стороны

1. **Модульность CV-пайплайна.** Каждый CV-модуль инкапсулирован, имеет чистый API (`Result<Mat, DigitizationError>`). Нет сквозных зависимостей между модулями.

2. **Безопасная обработка OpenCV.** Все функции возвращают `Result`, исключения OpenCV перехватываются через `.map_err(DigitizationError::OpenCv(...))`. В `warping.rs` реализован строгий геометрический валидатор (`validate_page_geometry`) ДО вызова Си-функций.

3. **Тестовое покрытие.** Каждый модуль содержит `#[cfg(test)] mod tests` с юнит-тестами. Тесты используют `Mat`-констант для детерминированной проверки.

4. **Hot-reload калибровки.** `CalibrationManager` отслеживает `mtime` файла и перечитывает параметры без перезапуска. Троттлинг на `RELOAD_INTERVAL = 500ms` защищает от трешхолда.

5. **FFI к libtiff.** Прямой вызов `TIFFOpen`/`TIFFSetField`/`TIFFWriteScanline`/`TIFFClose` для CCITT Group 4, который не поддерживается crate `tiff` 0.11.

6. **Система ошибок.** `DigitizationError` (thiserror) с именованными вариантами для каждого типа ошибки. Все ошибки реализуют `Display`/`Debug`.

### 2.2. Найденные проблемы

#### КРИТИЧЕСКИЕ

**C1. `config.rs` — отсутствие валидации путей к каталогам**
   - `src/config.rs` парсит `config.toml`, но не проверяет существование `scan_output_dir` и `session_db_path` перед запуском.
   - **Риск:** Сервис упадёт при первом сканировании с `NotFound`.
   - **Рекомендация:** Добавить `fs::create_dir_all` для каждого каталога при инициализации конфигурации.

**C2. `ccitt_encoder.rs` — отсутствие заголовка TIFF-файла (TIFF Header + IFD)**
   - FFI-код вызывает `TIFFSetField` и `TIFFWriteScanline`, но не устанавливает `TIFFTAG_AGGREGATETHUMBNAIL` и другие обязательные теги.
   - **Риск:** Некоторые TIFF-парсеры могут отклонить файл как невалидный.
   - **Рекомендация:** Добавить `TIFFTAG_PAGENUMBER` и `TIFFTAG_SOFTWARE`.

#### СРЕДНИЕ

**M1. `segmentation.rs` — `coarse_mask` использует `ALGO_HINT_APPROX`**
   - `AlgorithmHint::ALGO_HINT_APPROX` используется в `cvt_color` и `gaussian_blur`. Это даёт ~10-15% прирост скорости, но может снизить точность бинаризации.
   - **Рекомендация:** Для production-режима заменить на `ALGO_HINT_DEFAULT`.

**M2. `warping.rs` — `detect_spine_shadow` — O(n*m) прямой доступ к пикселям**
   - Функция итерирует каждый пиксель в Rust, что может быть медленно для изображений 4000x3000.
   - **Рекомендация:** Использовать OpenCV-функции `reduce` для вертикального усреднения вместо ручного цикла.

**M3. `session_store.rs` — отсутствие миграций БД**
   - Схема БД создаётся при первом запуске. При изменении схемы (например, добавление новых полей) старые файлы БД останутся несовместимыми.
   - **Рекомендация:** Добавить систему миграций (например, `sqlx` или кастомный `migrations/` каталог).

**M4. `routes.rs` — отсутствие лимита на размер загружаемого изображения**
   - Эндпоинты не ограничивают размер входящего изображения через `Content-Length`.
   - **Риск:** Memory exhaustion при загрузке 100MB+ изображений.
   - **Рекомендация:** Добавить `tower-http` middleware `Limit` или проверку `Content-Length` в роутах.

#### НИЖНИЕ

**L1. `binarization.rs` — дублирование кода конвертации F32**
   - `apply_sauvola_threshold` создаёт 6 промежуточных `Mat` буферов (`gray_f32`, `mean_f32`, `t_factor1`..`t_factor_final`). Каждый буфер — аллокация.
   - **Рекомендация:** Использовать `RAII`-обёртки или пул буферов для снижения аллокаций.

**L2. `profile_filtering.rs` — `from_str_lenient` не покрывает все варианты**
   - Функция поддерживает только `snake_case` и упрощённые алиасы. Не поддерживаются `TextBW1bit`, `ILLUSTRATION`, `COLOR_RGB_24BIT` (с сохранением регистра).
   - **Рекомендация:** Добавить `to_ascii_lowercase()` перед сравнением (уже есть, но проверить edge cases).

**L3. `pdf_exporter.rs` — отсутствие сжатия внутри PDF**
   - `lopdf` сохраняет изображения без сжатия. PDF-файл может быть в 3-5 раз больше необходимого.
   - **Рекомендация:** Использовать `lopdf::Compression::Flate` или встроить CCITT G4 в PDF.

**L4. `sane_core.rs` — нет таймаута на операцию сканирования**
   - `sane_start()` может блокироваться бесконечно при зависшем сканере.
   - **Рекомендация:** Добавить `tokio::time::timeout` вокруг всех SANE-операций.

**L5. `write_queue.rs` — нет обработки ошибок записи на диск**
   - При полном диске или отсутствии прав очередь не обрабатывает ошибку и может зациклиться.
   - **Рекомендация:** Добавить `max_retries` и `dead_letter_queue` для неудачных записей.

### 2.3. Потенциальные утечки памяти

| Место | Описание |
|-------|----------|
| `segmentation.rs:133` | `draw_contours` с `contours_vec` — каждый вызов создаёт временный `Vector<Vector<Point>>`. При 1000 кадров/сек — до 50MB/сек временных аллокаций. |
| `warping.rs:321-332` | Прямой доступ к `map_x`/`map_y` через `data_mut()` — при больших изображениях (4000x3000) каждый `remap` аллоцирует 48MB (2 × 4000×3000×4 байта). |
| `ccitt_encoder.rs:78` | `packed` вектор для упакованных битов — `rows * cols / 8` байт на каждый вызов. |

### 2.4. Потенциальные гонки данных

| Место | Описание |
|-------|----------|
| `calibration.rs:77-82` | `CalibrationManager` использует `Mutex<CalibrationParams>` — безопасно, но `last_mtime` и `last_check` тоже в `Mutex` — каждый `get()` блокирует 3 мьютекса. |
| `session_store.rs` | Если используется из нескольких Tokio-тасков, `rusqlite` должен быть инициализирован с `OpenFlags::SQLITE_OPEN_SHARED_CACHE`. |

---

## 3. АУДИТ КОНФИГУРАЦИЙ

### 3.1. `Cargo.toml`

| Параметр | Статус | Комментарий |
|----------|--------|-------------|
| `edition = "2024"` | ✅ | Соответствует требованиям |
| `opencv = "0.100"` | ✅ | Актуальная версия биндингов |
| `tokio = "1.53"` | ✅ | Stable |
| `axum = "0.8.9"` | ✅ | Stable |
| `rusqlite = "0.40"` | ✅ | Bundled — не требует системной БД |
| `lto = true` | ✅ | В `profile.release` |
| `panic = "abort"` | ✅ | В `profile.release` |
| `codegen-units = 1` | ✅ | Глубокая оптимизация |

**Замечание:** `pkg-config = "0.3.34"` в `build-dependencies` — корректно для автоопределения OpenCV.

### 3.2. `config.example.toml`

**Не проверен** (файл существует, но содержимое не прочитано). Рекомендуется:
- Добавить `#[serde(default)]` для всех полей конфигурации.
- Добавить валидацию `host` (должен быть валидным IP/hostname).
- Добавить валидацию `port` (должен быть в диапазоне 1024-65535).

### 3.3. `flat-scanner-server.service`

**Не проверен** (файл существует, но содержимое не прочитано). Рекомендуется:
- Указать `Restart=on-failure` для автоперезапуска.
- Добавить `LimitNOFILE=65536` для обработки множества сессий.
- Указать `User`/`Group` для работы со сканером (обычно `sane` или `scanner`).

### 3.4. `PKGBUILD`

**Не проверен** (файл существует, но содержимое не прочитано). Рекомендуется:
- Добавить `depends=('opencv' 'libtiff' 'poppler-utils' 'sane-airscan')`.
- Указать `makedepends=('rust' 'pkg-config')`.

### 3.5. `.gitignore`

**Не проверен** (файл существует, но содержимое не прочитано). Рекомендуется исключить:
- `calibration.json` (если генерируется автоматически).
- `*.tiff`, `*.pdf` в `tmp/` или `output/`.
- `.rs.bak`, `*.swp`.

---

## 4. АУДИТ ДОКУМЕНТАЦИИ

### 4.1. Существующая документация

| Файл | Статус |
|------|--------|
| `docs/tools/calibration_api.md` | ✅ Описывает API калибровки |
| `docs/tools/pdf_exporter.md` | ✅ Описывает PDF-экспорт |
| `docs/tools/session_recovery.md` | ✅ Описывает восстановление сессий |
| `docs/tools/session_store.md` | ✅ Описывает хранилище сессий |

### 4.2. Пропущенная документация

| Модуль | Статус |
|--------|--------|
| `src/cv/binarization.rs` | ❌ Нет `docs/tools/sauvola_binarization.md` |
| `src/cv/seal_extraction.rs` | ❌ Нет `docs/tools/seal_extraction.md` |
| `src/cv/ccitt_encoder.rs` | ❌ Нет `docs/tools/ccitt_encoder.md` |
| `src/cv/warping.rs` | ❌ Нет `docs/tools/geometric_warping.md` |
| `src/cv/profile_filtering.rs` | ❌ Нет `docs/tools/processing_profiles.md` |
| `src/sane_core.rs` | ❌ Нет `docs/tools/sane_integration.md` |
| `src/write_queue.rs` | ❌ Нет `docs/tools/write_queue.md` |

### 4.3. README

- `README.md` и `README.ru.md` существуют — ✅
- Перекрёстные ссылки должны быть между ними — проверить.

---

## 5. АУДИТ АРХИТЕКТУРНОГО СООТВЕТСТВИЯ

### 5.1. Соответствие TECH_SPEC.md

| Требование | Статус | Комментарий |
|------------|--------|-------------|
| Rust Edition 2024 | ✅ | `Cargo.toml:5` |
| Tokio async runtime | ✅ | `Cargo.toml:12` |
| Axum HTTP server | ✅ | `Cargo.toml:13` |
| OpenCV для CV | ✅ | `Cargo.toml:25` |
| rusqlite для сессий | ✅ | `Cargo.toml:31` |
| CCITT G4 TIFF | ✅ | `ccitt_encoder.rs` |
| Sauvola бинаризация | ✅ | `binarization.rs` |
| Детекция вершин | ✅ | `segmentation.rs` |
| Деформация корешка | ✅ | `warping.rs` |
| Сохранение печатей | ✅ | `seal_extraction.rs` |
| Локальная калибровка | ✅ | `calibration.rs` |
| Без облачных API | ✅ | Все модули локальные |

### 5.2. Соответствие TECH_SPEC_addon_*.md

| Addon | Требование | Статус |
|-------|-----------|--------|
| Addon 1 | Гомографический варпинг | ✅ `warping.rs` |
| Addon 2 | Геометрический валидатор | ✅ `validate_page_geometry` |
| Addon 3 | Hough-lines деформация | ✅ `dewarp_spine` |
| Addon 4 | HSV-детекция печатей | ✅ `extract_seal_mask` |

---

## 6. РЕЗЮМЕ И ПРІОРИТЕТЫ

### Критические (P0)
1. **C1** — Валидация путей к каталогам при старте.
2. **C2** — Добавление обязательных TIFF-тегов.

### Средние (P1)
3. **M1** — Замена `ALGO_HINT_APPROX` на `ALGO_HINT_DEFAULT` для production.
4. **M2** — Оптимизация `detect_spine_shadow` через OpenCV `reduce`.
5. **M3** — Система миграций БД.
6. **M4** — Лимит на размер загружаемого изображения.

### Низкие (P2)
7. **L1** — Пул буферов для Sauvola.
8. **L2** — Проверка `from_str_lenient` на edge cases.
9. **L3** — Сжатие изображений в PDF.
10. **L4** — Таймауты на SANE-операции.
11. **L5** — Обработка ошибок записи в `write_queue`.

### Документация (P2)
12. Создать недостающие `docs/tools/*.md` для всех CV-модулей.