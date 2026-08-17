# Session Recovery — Горячий рестарт сессии сканирования

## Назначение

Модуль `session_recovery` реализует механизм восстановления прерванной сессии сканирования при перезапуске приложения. Обеспечивает атомарность операций через двойное журналирование (pending-файлы + WAL checkpoint).

## Архитектура

```text
┌──────────────────────────────────────────────────────────────┐
│ SessionRecovery (pub)                                        │
│ ├─ recover_session() — восстановление при старте            │
│ ├─ write_pending() — предварительная запись /tmp/<uuid>.pending │
│ ├─ confirm_commit() — подтверждение успеха                  │
│ ├─ wal_checkpoint() — финальный WAL checkpoint               │
│ └─ cleanup_stale_pending() — очистка устаревших pending      │
└──────────────────────────────────────────────────────────────┘
```

## Двойное журналирование

1. **Предварительная запись** — перед обработкой кадра создаётся `/tmp/<uuid>.pending` с метаданными (spread_index, timestamp, status)
2. **Подтверждение** — после успешного сохранения страниц pending удаляется
3. **WAL checkpoint** — финальный сброс WAL в основную БД

## API

### `SessionRecovery::new(pending_dir: Option<&str>) -> Self`

Создаёт новый экземпляр. По умолчанию pending-файлы хранятся в `/tmp`.

### `recover_session(&self, store: &SessionStore) -> Result<Option<RecoveryResult>, String>`

Восстанавливает сессию при старте приложения:
- Находит последнюю незавершённую книгу
- Считает статусы разворотов
- Переводит развороты в статусе `Processing` в `Pending`
- Читает pending-файл если существует

### `write_pending(&self, book_uuid, spread_index, status, left_path, right_path) -> Result<PathBuf, String>`

Записывает предварительный pending-файл перед обработкой кадра. Использует атомарную запись (write + rename).

### `confirm_commit(&self, book_uuid) -> Result<(), String>`

Подтверждает успешный коммит — удаляет pending-файл.

### `wal_checkpoint(&self, store: &SessionStore) -> Result<(), String>`

Выполняет `PRAGMA wal_checkpoint(TRUNCATE)` для сброса WAL в основную БД.

### `cleanup_stale_pending(&self, max_age: Duration) -> Result<Vec<PathBuf>, String>`

Очищает устаревшие pending-файлы старше указанного времени.

## Пример использования

```rust
use scan::session_recovery::SessionRecovery;
use scan::session_store::global_session_store;

fn main() {
    let store = global_session_store("./data.db");
    let recovery = SessionRecovery::new(None);

    // Восстановление при старте
    if let Ok(store) = store.lock() {
        if let Ok(Some(result)) = recovery.recover_session(&store) {
            println!("Восстановлена сессия: {}", result.book_name);
        }

        // WAL checkpoint
        let _ = recovery.wal_checkpoint(&store);

        // Очистка устаревших pending (старше 24 часов)
        let _ = recovery.cleanup_stale_pending(std::time::Duration::from_secs(86400));
    }
}
```

## Известные ограничения

- Pending-файлы хранятся в `/tmp` — при перезагрузке ОС они будут удалены
- Не поддерживает параллельные сессии (одна незавершённая книга)
- WAL checkpoint блокирует запись на время выполнения

## Зависимости

- `rusqlite` — SQLite для хранения сессий
- `filetime` — установка mtime для pending-файлов
- `serde` / `serde_json` — сериализация pending-записей