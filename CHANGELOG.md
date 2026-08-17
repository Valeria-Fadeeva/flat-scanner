# Changelog

## [Unreleased]

### Changed
- **Release pipeline**: `release.sh` теперь публикует релиз через совместимый REST API (GitHub + Forgejo) и загружает два таргетированных архива (`v<ver>-server.tar.gz`, `v<ver>-client.tar.gz`) как assets, вместо полагания на автогенерированный GitHub-артефакт всего дерева. `archive/` добавлен в `.gitignore`. Перед упаковкой выполняется предварительная очистка артефактов сборки (`cargo clean` / `flutter clean` + удаление `target/`, `build/`, `.dart_tool/`), чтобы релизные архивы не содержали кэши компиляции.

### Fixed
- **Session Store**: `execute_pragma` теперь автоматически добавляет префикс `PRAGMA `, если строка не начинается с него. Исправлен syntax error при выполнении `wal_checkpoint(TRUNCATE)` в `session_recovery.rs`.

### Added
- **G4 (Flutter)**: Экран разборки стороннего PDF (`lib/presentation/pdf_import_page.dart`) — растеризация страниц, замена/вставка/очистка страниц.
- **G4 (Flutter)**: 4 метода в `ApiService` (`importPdf`, `replacePdfPage`, `insertPdfPage`, `cleanPdfPage`) + модели `ImportPdfResponse`, `PdfOperationResponse`.
- **G4 (Flutter)**: Кнопка навигации на экран разборки PDF в AppBar `ScanEditorPage`.
- G4: Разборка сторонних PDF — модуль `flat-scanner-server/src/pdf_importer.rs`. REST endpoints: `POST /api/v1/import-pdf` (растеризация страниц через `pdftoppm`), `POST /api/v1/replace-pdf-page` и `POST /api/v1/insert-pdf-page` (структурные операции через `lopdf`), `POST /api/v1/clean-pdf-page` (очистка от шума через `cv::profile_filtering::apply_profile`). 6 unit-тестов.
- G5: Экспорт книги в PDF — модуль `flat-scanner-server/src/pdf_exporter.rs` на базе `lopdf 0.44`. REST endpoint `POST /api/v1/export-pdf` собирает финальный PDF из всех страниц книги (spreads по `spread_index ASC`, левая → правая), с метаданными (title/author/subject). Flutter-клиент: метод `exportPdf` в `api_service.dart` + кнопка «Экспортировать PDF» в UI.
- G3: Сохранение печатей и штампов — модуль `src/cv/seal_extraction.rs`. Извлечение маски печати по **насыщенности (S-канал HSV)** — универсально для красных, синих, голубых и фиолетовых чернил (в отличие от прежнего канала Cr, который ловил только красный). Порог Otsu, морфологическая очистка, порог площади. Интеграция в `apply_profile` (TextBw1bit) — печать сохраняется в 1-битном растре и не стирается Sauvola-бинаризацией. 6 unit-тестов (красная/синяя/фиолетовая печать, grayscale-noop, overlay, empty-mask).
- G1: REST endpoint `/api/v1/calibration` (GET/POST) — `CalibrationManager` с mtime-cache 500мс.
- G2: REST endpoint `/api/v1/scan/<uuid>/adjust-vertex` (PATCH) — корректировка вершин страницы.
- G6: CustomPainter Drag-and-Drop вершин во Flutter-клиенте (`vertex_editor.dart`).