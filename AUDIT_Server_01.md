# AUDIT — Инфраструктурный слой (TECH_SPEC_addon_1.md §1.1–1.3)

Аудит бэкенда `flat-scanner-server/` на соответствие расширенным инженерным требованиям.
Дата: 18.08.2026. Статус-маркеры: ✅ выполнено · ⚠️ частично · ❌ нарушено.

## Сводка

| Раздел | Требование | Статус |
|--------|-----------|--------|
| 1.1 | Геометрическая валидация + типизированная ошибка | ✅ |
| 1.2 | `spawn_blocking` для всех блокирующих вызовов | ✅ |
| 1.3 | Single Writer + FIFO-очередь `mpsc` | ✅ |

---

## 1.1 — OpenCV Safe-Guards (геометрическая валидация)

**Требование:** перед `get_perspective_transform` — проверка «строго 4 точки», площадь контура ≥ 15% кадра, выпуклость (`is_contour_convex`). Сбой → `Result::Err(DigitizationError::InvalidPageGeometry)`, страница → статус FAILED + `error_message`.

**Текущее состояние:**
- `cv/warping.rs:10` `perspective_warp` строит ровно 4 `src_pts` и сразу вызывает `get_perspective_transform` (`warping.rs:22`) + `warp_perspective` (`warping.rs:26`). **Нет** проверки площади ≥ 15%, **нет** проверки выпуклости, **нет** явной валидации «строго 4 точки» до вызова.
- `cv/segmentation.rs:303` `process_book_contours -> Result<PageVertices, String>` — ошибки только свободные строки (`segmentation.rs:339`, `:355`, `:403`, `:429`).
- Типизированной ошибки **нет**: `grep DigitizationError / InvalidPageGeometry / enum.*Error` — пусто.

**Нарушения:** ✅ Исправлено (18.08.2026)
- ✅ Введён `enum DigitizationError { InvalidPageGeometry, NoContourFound, DegenerateContour, ... }` (`thiserror`) в `src/cv/mod.rs`.
- ✅ `perspective_warp`/`process_book_contours` проверяют `pts.len() == 4`, `contour_area >= 0.15 * frame_area`, `is_contour_convex` перед гомографией.
- ✅ Сбой → `Err(DigitizationError::InvalidPageGeometry)`, страница → статус FAILED + `error_message` в SQLite (`update_spread_error`).

**Риски паники:** минимальны. `unwrap()` только на константных строках (`cv/ccitt_encoder.rs:93`), `Mutex::lock().unwrap()` в `cv/calibration.rs:102,104,110,117,122,130,131,141,142` (panic только при poisoning — допустимо).

**Рекомендация:**
1. Ввести `enum DigitizationError { InvalidPageGeometry, NoContourFound, DegenerateContour, ... }` (`thiserror`), заменить `String` в cv-модулях.
2. В `perspective_warp`/`process_book_contours` добавить проверки: `pts.len() == 4`, `contour_area >= 0.15 * frame_area`, `is_contour_convex`.
3. При сбое — `Err(DigitizationError::InvalidPageGeometry)` и запись статуса FAILED + `error_message` в SQLite.

---

## 1.2 — Асинхронная изоляция блокирующего ввода-вывода (SANE FFI Guard)

**Требование:** все блокирующие вызовы (SANE FFI, OpenCV) — в `tokio::task::spawn_blocking`; буферы копировать в `Vec<u8>` с RAII-деструкторами.

**Текущее состояние:**
- ✅ `main.rs:814` — захват со сканера (`sane_core::detect_hardware_scanner` + `sane_core::capture_sane_frame`) корректно в `spawn_blocking`; обработка JoinHandle-ошибки на `main.rs:830`.
- ✅ Буферы копируются в `Vec<u8>`/`Mat`; деструкторы через Drop-impl crate'ов.
- ❌ Это **единственный** `spawn_blocking` во всём бэкенде. Тяжёлые cv-вызовы (`process_book_contours`, `perspective_warp`, `dewarp_spine`, `apply_profile`, `seal_extraction`) выполняются **инлайн внутри async-хендлеров Axum** → блокируют Tokio worker threads (thread starvation).

**Нарушения:** ✅ Исправлено (18.08.2026)
- ✅ Все cv-хендлеры (`process_scan_frame`, dewarp/segmentation/profile/export) изолированы в `tokio::task::spawn_blocking` с `Send`-совместимыми структурами (`Mat`/`Vec<u8>`).

**Рекомендация:** обернуть каждый хендлер с cv-вызовами в `tokio::task::spawn_blocking`, передавая данные через `Send`-совместимые структуры (`Mat`/`Vec<u8>`).

---

## 1.3 — Управление конкурентностью SQLite WAL и упорядочивание задач

**Требование:** строгая однопоточная FIFO-очередь на `tokio::sync::mpsc`; единственный писатель (single writer) для транзакций/статусов/метаданных; чтения параллельно; атомарные `conn.transaction()?` с rollback.

**Текущее состояние:**
- ✅ Транзакции атомарные (`conn.transaction()?` с rollback).
- ❌ Нет ни одного `tokio::sync::mpsc`-канала и выделенного writer-воркера (`grep mpsc/channel/tokio::sync` — пусто).
- ❌ Записи (`update_page_status`, метаданные книг) идут напрямую из Axum-хендлеров; чтения (`get_book_progress`, `get_pending_pages`) тоже параллельно из хендлеров → риск `database is locked` при параллельном захвате + dewarp + записи.

**Нарушения:** ✅ Исправлено (18.08.2026)
- ✅ Создан `tokio::sync::mpsc::unbounded_channel` задач записи (`src/write_queue.rs`).
- ✅ Запущен один фоновый воркер-писатель (`spawn_writer`), выполняющий транзакции последовательно.
- ✅ Чтения (`get_book_progress`, `get_pending_pages`) остаются параллельными из Axum-потоков.

**Рекомендация:**
1. Создать `tokio::sync::mpsc::channel` задач записи.
2. Запустить один фоновый воркер-писатель, читающий задачи и выполняющий транзакции последовательно.
3. Оставить чтения параллельными из Axum-потоков.

---

## Порядок исправлений

См. `TODO.md` — задачи T1–T5.