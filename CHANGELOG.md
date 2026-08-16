# Changelog

## [Unreleased]

### Added
- G3: Сохранение печатей и штампов — модуль `src/cv/seal_extraction.rs`. Извлечение маски печати по **насыщенности (S-канал HSV)** — универсально для красных, синих, голубых и фиолетовых чернил (в отличие от прежнего канала Cr, который ловил только красный). Порог Otsu, морфологическая очистка, порог площади. Интеграция в `apply_profile` (TextBw1bit) — печать сохраняется в 1-битном растре и не стирается Sauvola-бинаризацией. 6 unit-тестов (красная/синяя/фиолетовая печать, grayscale-noop, overlay, empty-mask).
- G1: REST endpoint `/api/v1/calibration` (GET/POST) — `CalibrationManager` с mtime-cache 500мс.
- G2: REST endpoint `/api/v1/scan/<uuid>/adjust-vertex` (PATCH) — корректировка вершин страницы.
- G6: CustomPainter Drag-and-Drop вершин во Flutter-клиенте (`vertex_editor.dart`).