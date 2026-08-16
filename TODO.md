# TODO — Задачи на доработку проекта «Канонисса-Библиотека»

**Дата создания:** 14 августа 2026 г.  
**Дата актуализации:** 16 августа 2026 г.  
**Исходник:** TECH_SPEC.md (разделы 10–12)

---

## Этап B: Computer Vision (MEDIUM-HIGH)

### B1. Доработка деварпинга корешка книги
- [x] Реализовать цилиндрическую трансформацию для выпрямления текста у тугого корешка
  - [x] Детекция центральной тени корешка по градиенту яркости (`detect_spine_shadow`)
  - [x] Построение Mesh Grid деформации через Text Line Tracking (`build_cylindrical_deformation`)
  - [x] Применение remap(cx,cy→x',y') обратной координатной трансформации (`dewarp_spine`)
- **Файл:** `src/cv/warping.rs`
- **Статус:** Реализовано, cargo check + cargo test OK

### B2. Изоляция боковых артефактов ("боковушек") ✅ ЗАВЕРШЕНО
- [x] Градиентный анализ плотности по периферии macro-contour
  - Обнаружение паттерна "частые чередующиеся светлые/тёмные линии"
  - Принудительный сдвиг рамки детекции внутрь на шаг дефекта
- **Файл:** `src/cv/segmentation.rs`
- **Статус:** Реализовано, cargo check + cargo test OK

### B3. Улучшение coarse masking ✅ ЗАВЕРШЕНО
- [x] Доработка маскирования потолка/ламп для сложных сценариев освещения
  - Мультимасштабный анализ (3 масштаба)
  - Morphological closing для объединения близких пятен
- **Файл:** `src/cv/segmentation.rs`
- **Статус:** Реализовано, cargo check + cargo test OK

---

## Этап C: Flutter Desktop клиент (MEDIUM-HIGH)

### C1. Генерация проекта Flutter ✅ ЗАВЕРШЕНО
- [x] Создать проект Flutter Linux desktop (`flat-scanner-client-flutter/`)
- [x] Настроить маршрутизацию HTTP к Axum API (настраиваемый host/port)
- [x] Подключить крейты http/bloc/flutter_bloc/equatable/window_manager
- [x] Настроить структуру lib/{presentation/domain/data}

### C2. ScannerBLoC реактивная модель ✅ ЗАВЕРШЕНО
- [x] Реализовать ScannerEvent (StartScan, ResetScan)
- [x] Реализовать ScannerState (Initial, Scanning, Success, Error)
- [x] Настроить потоковую обработку через BlocProvider
- [x] Интегрировать POST запросы к Axum API
- **Файл:** `flat-scanner-client-flutter/lib/domain/scanner_bloc.dart`
- **Статус:** flutter analyze OK, flutter build linux --release OK

### C3. UI редактора сканирования ✅ ЗАВЕРШЕНО
- [x] ScanEditorPage: выбор профиля, кнопка сканирования, результат
- [x] Отображение вершин страницы и времени обработки
- [x] Опциональный полноэкранный режим (window_manager)
- [x] ThemeService: адаптация под KDE/Breeze + Material 3
- **Файлы:** `lib/presentation/scan_editor_page.dart`, `lib/data/theme_service.dart`
- **Статус:** flutter analyze OK, flutter build linux --release OK

### C4. CustomPainter интерактивной сетки ✅ ЗАВЕРШЕНО
- [x] Реализовать VertexEditor (CustomPainter) с Drag-and-Drop вершин
- [x] Добавить Draggable Point с UX-кольцами подсветки (активная вершина — оранжевая, увеличенная)
- [x] Интегрировать GestureDetector.onPanStart/onPanUpdate/onPanEnd
- [x] Отправка PATCH запросов к Axum API для корректировки вершин (adjust-vertex, G2)
- [x] ApiService вынесен в RepositoryProvider (main.dart), доступен виджетам
- **Файл:** `flat-scanner-client-flutter/lib/presentation/vertex_editor.dart`
- **Статус:** Реализовано, подключено в `_ScanResultCard` (scan_editor_page.dart), flutter analyze OK
- **Зависимость:** G2 (endpoint adjust-vertex)

---

## Этап F: Упаковка и дистрибуция (MEDIUM)

### F1. Сервер: конфигурация и сервисы ✅ ЗАВЕРШЕНО
- [x] CLI-флаги --host/--port + config.toml
- [x] systemd service unit
- [x] PKGBUILD для flat_scanner_server
- [x] README сервера

### F2. Клиент: дистрибуция ✅ ЗАВЕРШЕНО
- [x] .desktop entry (flat-scanner-client.desktop)
- [x] PKGBUILD для flat-scanner-client
- [x] README клиента
- **Статус:** flutter build linux --release OK

---

## Этап D: Session Store + Hot Restart (HIGH)

### D1. SQLite транзакционная модель ✅ ЗАВЕРШЕНО
- [x] Создать модуль session_store.rs на базе rusqlite
- [x] Реализовать схему БД:
  - Таблица books (uuid, name, start_date, total_pages, status)
  - Таблица spreads (book_uuid, spread_index, left_path, right_path, left_vertices, right_vertices, threshold_k, status)
- [x] Атомарные INSERT+UPDATE операции в BEGIN TRANSACTION...COMMIT
- [x] Настроить journal_mode=WAL
- **Файл:** `src/session_store.rs`
- **Статус:** Реализовано, интегрировано в main.rs, cargo check + cargo test OK (28 тестов)

### D2. Горячий рестарт сессии ✅ ЗАВЕРШЕНО
- [x] Логика восстановления при старте:
  - [x] Чтение последнего незавершённого UUID
  - [x] Восстановление очереди спредов
  - [x] Открытие книги на прерванной странице
- [x] Двойное журналирование:
  - [x] Предварительная запись `/tmp/<uuid>.pending`
  - [x] Подтверждение успеха коммитом
  - [x] Финальный WAL checkpoint
- [x] Очистка устаревших pending-файлов (старше 24 часов)
- **Файл:** `src/session_recovery.rs`
- **Статус:** Реализовано, интегрировано в main.rs, cargo check + cargo test OK (34 теста)
- **Зависимость:** D1

---

## Этап E: PDF Exporter + Multi-profile (MEDIUM)

### E1. Экспорт в CCITT Group 4 TIFF ✅ ЗАВЕРШЕНО
- [x] Подключить крейт `tiff`
- [x] Реализовать функцию encode_ccitt_g4_to_file() через OpenCV imgcodecs::imwrite
- [x] Заменить imgcodecs::imwrite("png") на CCITT G4 энкодер в CLI и Web API режимах
- **Файл:** `src/cv/ccitt_encoder.rs`
- **Статус:** Реализовано, интегрировано, cargo check + cargo test OK
- **Замечание:** OpenCV требует 1-битный ввод для CCITT G4 (Bits/sample=1). При 8-битном вводе файл создаётся, но с предупреждением. Необходимо бинаризовать изображение до 1-битного формата перед вызовом.

### E2. Multi-profile фильтрация ✅ ЗАВЕРШЕНО
- [x] Создать модуль profile_filtering.rs
- [x] Реализовать enum ProcessingProfile:
  - Text_BW_1bit (Sauvola + CCITT G4)
  - Illustration_Grayscale_8bit (gamma correction + CLAHE contrast)
  - Color_RGB_24bit (оригинальная палитра)
- [x] Функция apply_profile(mat, profile, k_factor, window_size)
- [x] Передача параметра профиля из Flutter UI через API (поле `profile` в ScanTriggerRequest)
- [x] Сохранение: CCITT G4 TIFF для 1-бит, PNG для grayscale/color
- **Файл:** `src/cv/profile_filtering.rs`
- **Статус:** Реализовано, интегрировано в main.rs, cargo check + cargo test OK

---

## Этап G: Сохранение печатей и разборка PDF (HIGH)

### G3. Сохранение печатей и штампов (YCbCr) ✅ ЗАВЕРШЕНО
- [x] Создать модуль seal_extraction.rs
- [x] Извлечение маски печати из канала Cr (YCbCr) с инверсией
- [x] Порог Otsu + морфологическая очистка (открытие/закрытие)
- [x] Порог площади (MIN_SEAL_AREA_RATIO) для отсечения шума бумаги
- [x] overlay_seal_on_text: принудительная чёрная заливка пикселей печати
- [x] Интеграция в apply_profile (TextBw1bit) — печать сохраняется в 1-битном растре
- [x] 4 unit-теста (детекция красной печати, grayscale-noop, overlay, empty-mask)
- **Файл:** `src/cv/seal_extraction.rs`
- **Статус:** Реализовано, интегрировано, cargo check + cargo test OK (44 теста)
- **Зависимость:** E2 (profile_filtering), E1 (ccitt_encoder)

### G4. Разборка сторонних PDF
- [ ] Создать модуль pdf_importer.rs
- [ ] Подключить крейт pdf-extract/poppler или pdftoppm
- [ ] Реализовать:
  - Открытие "чужих" PDF
  - Декомпиляцию страниц в растровые слои
  - Точечную замену дефектных листов
  - Сборку обновлённого PDF
- **Файл:** `src/pdf_importer.rs`

### G5. Сборка финального PDF из CCITT G4
- [ ] Собрать финальный PDF из CCITT G4 TIFF-страниц
- [ ] Сохранить метаданные (название книги, страницы)
- **Файл:** `src/pdf_exporter.rs`

---

## Дополнительные задачи

### M8. Пакетная калибровка порогов Sauvola ✅ ЗАВЕРШЕНО
- [x] Реализовать hot-reload параметра k_factor (отслеживание mtime файла)
- [x] Динамическая реконфигурация через std::sync::Mutex + троттлинг 500мс
- [x] Файл калибровки `calibration.json` (k_factor, window_size, profile)
- [x] Методы `reload()` и `save()` для Flutter UI (endpoint /api/v1/calibration — TODO)
- [x] Интеграция в main.rs: параметры читаются при каждой обработке кадра
- **Файл:** `src/cv/calibration.rs`
- **Статус:** Реализовано, интегрировано, cargo check + cargo test OK
- **Замечание:** REST endpoint `/api/v1/calibration` — ✅ ЗАВЕРШЕНО 16.08.2026. `CalibrationManager` (global OnceLock, mtime cache 500ms), `process_scan_frame` читает `calib.get()` перед каждой обработкой, Flutter `api_service.dart` — `getCalibration()`/`updateCalibration()`, `CalibrationParams` model, порт по умолчанию 8080. 4 теста калибровки зелёные, `flutter analyze` чисто. Документация: `flat-scanner-server/docs/tools/calibration_api.md`.
