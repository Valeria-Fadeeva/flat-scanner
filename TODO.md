# TODO — Задачи на доработку проекта «Канонисса-Библиотека»

**Дата создания:** 14 августа 2026 г.  
**Исходник:** docs/UNIFIED.md (разделы 10.2–10.3, 11)

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

### C1. Генерация проекта Flutter
- [ ] Создать проект Flutter Linux desktop
- [ ] Настроить маршрутизацию HTTP к Axum API localhost:54321
- [ ] Подключить крейты http/bloc/flutter_bloc/provider
- [ ] Настроить структуру lib/{presentation/domain/data}

### C2. ScannerBLoC реактивная модель
- [ ] Реализовать ScannerEvent (StartScan, CancelScan, AdjustVertex)
- [ ] Реализовать ScannerState (Initial, Ready, InProgress, ProcessingInCore, PreviewReady, SavingPage, Error)
- [ ] Настроить потоковую обработку через BlocProvider
- [ ] Интегрировать POST запросы к Axum API

### C3. CustomPainter интерактивной сетки
- [ ] Реализовать ScanEditorPainter с Drag-and-Drop вершин
- [ ] Добавить Draggable Point с UX-кольцами подсветки
- [ ] Интегрировать GestureDetector.onPanUpdate
- [ ] Отправка PATCH запросов к Axum API для корректировки вершин
- **Reference:** docs/проверить.md (scan_editor_painter.dart)

---

## Этап D: Session Store + Hot Restart (HIGH)

### D1. SQLite транзакционная модель
- [ ] Создать модуль session_store.rs на базе rusqlite
- [ ] Реализовать схему БД:
  - Таблица books (uuid, name, start_date, total_pages, status)
  - Таблица spreads (book_uuid, spread_index, left_path, right_path, left_vertices, right_vertices, threshold_k, status)
- [ ] Атомарные INSERT+UPDATE операции в BEGIN TRANSACTION...COMMIT
- [ ] Настроить journal_mode=WAL
- **Файл:** `src/session_store.rs`

### D2. Горячий рестарт сессии
- [ ] Логика восстановления при старте:
  - Чтение последнего незавершённого UUID
  - Восстановление очереди спредов
  - Открытие книги на прерванной странице
- [ ] Двойное журналирование:
  - Предварительная запись `/tmp/<uuid>.pending`
  - Подтверждение успеха коммитом
  - Финальный WAL checkpoint
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

## Дополнительные задачи

### M7. Модуль разборки сторонних PDF
- [ ] Создать модуль pdf_importer.rs
- [ ] Подключить крейт pdf-extract/poppler или pdftoppm
- [ ] Реализовать:
  - Открытие "чужих" PDF
  - Декомпиляцию страниц в растровые слои
  - Точечную замену дефектных листов
  - Сборку обновлённого PDF
- **Файл:** `src/pdf_importer.rs`

### M8. Пакетная калибровка порогов Sauvola ✅ ЗАВЕРШЕНО
- [x] Реализовать hot-reload параметра k_factor (отслеживание mtime файла)
- [x] Динамическая реконфигурация через std::sync::Mutex + троттлинг 500мс
- [x] Файл калибровки `calibration.json` (k_factor, window_size, profile)
- [x] Методы `reload()` и `save()` для Flutter UI (endpoint /api/v1/calibration — TODO)
- [x] Интеграция в main.rs: параметры читаются при каждой обработке кадра
- **Файл:** `src/cv/calibration.rs`
- **Статус:** Реализовано, интегрировано, cargo check + cargo test OK
- **Замечание:** REST endpoint для Flutter UI ещё не создан (см. Этап C)
