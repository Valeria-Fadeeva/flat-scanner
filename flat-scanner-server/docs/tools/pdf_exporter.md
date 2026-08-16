# pdf_exporter — сборка финального PDF из страниц книги

## Назначение

Собирает финальный PDF-файл из всех отсканированных страниц книги (CCITT G4 TIFF для 1-битного профиля, PNG для grayscale/color), сохраняя порядок страниц и метаданные.

## Архитектура

- **Модуль:** `flat-scanner-server/src/pdf_exporter.rs`
- **Зависимость:** `lopdf 0.44` (чистый Rust, без внешних бинарных утилит)
- **Источник страниц:** `session_store` (SQLite) — развороты книги по `spread_index ASC`, внутри разворота: левая → правая страница
- **REST endpoint:** `POST /api/v1/export-pdf`

## API

### `assemble_pdf_from_tiff_pages(page_paths, metadata, output_path) -> Result<usize, String>`

- `page_paths` — упорядоченный список путей к TIFF/PNG страницам
- `metadata` — `PdfMetadata { title, author, subject }`
- `output_path` — путь к выходному PDF
- Возвращает размер PDF в байтах

### `POST /api/v1/export-pdf`

Запрос:
```json
{ "uuid": "<book-uuid>", "output_path": "./export/book.pdf" }
```
`output_path` опционален, по умолчанию `./export/<uuid>.pdf`.

Ответ:
```json
{ "path": "./export/book.pdf", "size_bytes": 123456, "page_count": 42 }
```

Ошибки: `404` (книга не найдена), `422` (нет страниц), `500` (ошибка сборки).

## Пример использования

```bash
curl -X POST http://127.0.0.1:54321/api/v1/export-pdf \
  -H 'Content-Type: application/json' \
  -d '{"uuid": "abc-123"}'
```

## Известные ограничения

- Блокирующий вызов (imread + сжатие) выполняется напрямую в async-хендлере без `spawn_blocking` — для локального инструмента блокировка event loop допустима; это обходит проблему не-Send future из-за `std::sync::MutexGuard` session store.
- Страницы читаются с диска по путям из БД; отсутствующие файлы пропускаются с предупреждением.