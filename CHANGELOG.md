# Changelog

## [Unreleased]

### Added
- G3: Сохранение печатей и штампов (YCbCr) — модуль `src/cv/seal_extraction.rs`. Извлечение маски печати из канала Cr с инверсией, порог Otsu, морфологическая очистка, порог площади. Интеграция в `apply_profile` (TextBw1bit) — печать сохраняется в 1-битном растре и не стирается Sauvola-бинаризацией. 4 unit-теста.
- G1: REST endpoint `/api/v1/calibration` (GET/POST) — `CalibrationManager` с mtime-cache 500мс.
- G2: REST endpoint `/api/v1/scan/<uuid>/adjust-vertex` (PATCH) — корректировка вершин страницы.
- G6: CustomPainter Drag-and-Drop вершин во Flutter-клиенте (`vertex_editor.dart`).