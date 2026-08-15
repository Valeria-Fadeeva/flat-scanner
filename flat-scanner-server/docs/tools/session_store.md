# Session Store — Транзакционное хранение сессий сканирования

## Назначение

Модуль `session_store` реализует SQLite-хранилище для управления книгами и разворотами (спредами) в процессе сканирования. Поддерживает WAL-режим для конкурентного доступа и атомарные транзакции.

## Архитектура

```
┌─────────────────────────────────────────┐
│ SessionStore (pub)                      │
│ ├─ create_book()                       │
│ ├─ add_spread()                        │
│ ├─ update_spread_status()              │
│ ├─ update_spread_paths()               │
│ ├─ update_spread_vertices()            │
│ ├─ update_book_status()                │
│ ├─ update_book_total_pages()           │
│ ├─ get_book() / get_spread()           │
│ ├─ get_last_spread()                   │
│ ├─ get_in_progress_book()              │
│ ├─ list_books() / list_spreads()       │
│ ├─ delete_book()                       │
│ └─ close()                             │
└─────────────────────────────────────────┘
```

## Схема БД

```sql
CREATE TABLE books (
    uuid TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    start_date TEXT NOT NULL,
    total_pages INTEGER DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'in_progress',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE spreads (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    book_uuid TEXT NOT NULL,
    spread_index INTEGER NOT NULL,
    left_path TEXT,
    right_path TEXT,
    left_vertices TEXT,
    right_vertices TEXT,
    threshold_k REAL,
    status TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (book_uuid) REFERENCES books(uuid) ON DELETE CASCADE,
    UNIQUE(book_uuid, spread_index)
);
```

## API

### Создание хранилища

```rust
let store = SessionStore::new("/path/to/db.sqlite")?;
```

### Создание книги

```rust
let book = store.create_book("Мой роман")?;
// book.uuid, book.name, book.status == BookStatus::InProgress
```

### Добавление разворота

```rust
let spread = store.add_spread(
    &book.uuid,
    0,
    Some("/tmp/left.tiff"),
    Some("/tmp/right.tiff"),
)?;
```

### Обновление статуса

```rust
store.update_spread_status(spread.id, SpreadStatus::Processing)?;
store.update_book_status(&book.uuid, BookStatus::Completed)?;
```

### Восстановление сессии

```rust
// Получить UUID последней незавершённой книги
if let Some(uuid) = store.get_in_progress_book()? {
    let book = store.get_book(&uuid)?.unwrap();
    let spreads = store.list_spreads(&uuid)?;
    // ... восстановить очередь обработки
}
```

## Статусы

### BookStatus
- `InProgress` — книга в процессе сканирования
- `Completed` — книга завершена
- `Cancelled` — книга отменена
- `Paused` — книга на паузе

### SpreadStatus
- `Pending` — ожидает обработки
- `Processing` — обработка в процессе
- `Completed` — обработан успешно
- `Failed` — ошибка обработки

## Пример использования

```rust
use scan::session_store::{SessionStore, BookStatus, SpreadStatus};

fn main() -> Result<(), String> {
    let store = SessionStore::new("data/sessions.sqlite")?;

    // Создаём книгу
    let book = store.create_book("Толстой. Война и мир")?;

    // Добавляем развороты
    for i in 0..100 {
        store.add_spread(&book.uuid, i, None, None)?;
    }

    // Обновляем статусы
    store.update_book_total_pages(&book.uuid, 100)?;
    store.update_book_status(&book.uuid, BookStatus::Completed)?;

    Ok(())
}
```

## Известные ограничения

1. **Одно соединение** — `SessionStore` не поддерживает параллельный доступ к одному файлу БД из разных процессов без WAL.
2. **Глобальный экземпляр** — `global_session_store()` использует `OnceLock`, повторная инициализация невозможна.
3. **Дата/время** — `chrono_now()` реализован без внешней зависимости `chrono`, использует упрощённое преобразование Unix timestamp.

## Зависимости

- `rusqlite 0.40` (feature `bundled`)
- `uuid 1.x` (feature `v4`)
- `serde 1.x` (feature `derive`)

## Тесты

Все 14 тестов модуля проходят успешно:
- `test_create_and_get_book`
- `test_add_and_get_spread`
- `test_update_spread_status`
- `test_update_book_status`
- `test_list_books`
- `test_list_spreads`
- `test_get_in_progress_book`
- `test_delete_book`
- `test_book_status_parsing`
- `test_spread_status_parsing`
- `test_update_spread_paths`
- `test_update_spread_vertices`
- `test_update_book_total_pages`
- `test_get_last_spread`