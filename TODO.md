# TODO — Задачи на доработку проекта «Канонисса-Библиотека»

**Дата создания:** 14 августа 2026 г.  
**Исходник:** docs/UNIFIED.md (разделы 10.2–10.3, 11)

---

## Этап B: Computer Vision (MEDIUM-HIGH)

### B1. Доработка деварпинга корешка книги
- [ ] Реализовать цилиндрическую трансформацию для выпрямления текста у тугого корешка
  - Детекция центральной тени корешка по градиенту яркости
  - Построение Mesh Grid деформации через Text Line Tracking
  - Применение remap(cx,cy→x',y') обратной координатной трансформации
- **Файл:** `src/cv/warping.rs`
- **Статус:** Перспективная трансформация реализована, цилиндрическая — нет

### B2. Изоляция боковых артефактов ("боковушек")
- [ ] Градиентный анализ плотности по периферии macro-contour
  - Обнаружение паттерна "частые чередующиеся светлые/тёмные линии"
  - Принудительный сдвиг рамки детекции внутрь на шаг дефекта
- **Файл:** `src/cv/segmentation.rs`
- **Зависимость:** coarse_mask() (реализовано)

### B3. Улучшение coarse masking
- [ ] Доработка маскирования потолка/ламп для сложных сценариев освещения
- **Файл:** `src/cv/segmentation.rs`
- **Статус:** Базовая реализация есть, требует улучшения

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

### E1. Экспорт в CCITT Group 4 TIFF
- [ ] Подключить крейт `tiff`
- [ ] Реализовать функцию encode_ccitt_g4(src)
- [ ] Заменить imgcodecs::imwrite("png") на CCITT G4 энкодер
- **Цель:** ~80–120 KB/страницу, книга 400 стр ≤40 MB

### E2. Multi-profile фильтрация
- [ ] Создать модуль profile_filtering.rs
- [ ] Реализовать enum ProcessingProfile:
  - Text_BW_1bit (Sauvola + CCITT G4)
  - Illustration_Grayscale_8bit (gamma correction + contrast)
  - Color_RGB_24bit (оригинальная палитра)
- [ ] Функция apply_profile(mat, profile)
- [ ] Передача параметра профиля из Flutter UI через API
- **Файл:** `src/cv/profile_filtering.rs`

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

### M8. Пакетная калибровка порогов Sauvola
- [ ] Реализовать hot-reload параметра k_factor
- [ ] Добавить динамическую реконфигурацию через tokio::sync::Mutex
- [ ] Интегрировать с Flutter UI для мгновенного результата
- **Значения:** 70, 80, 90, 110 единиц смещения