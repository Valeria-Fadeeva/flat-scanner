//! # Session Recovery — Горячий рестарт сессии сканирования
//!
//! Модуль реализует механизм восстановления прерванной сессии сканирования
//! при перезапуске приложения.
//!
//! ## Архитектура
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────────┐
//! │ SessionRecovery (pub)                                        │
//! │ ├─ recover_session() — восстановление при старте            │
//! │ ├─ write_pending() — предварительная запись /tmp/<uuid>.pending │
//! │ ├─ confirm_commit() — подтверждение успеха                  │
//! │ ├─ wal_checkpoint() — финальный WAL checkpoint               │
//! │ └─ cleanup_stale_pending() — очистка устаревших pending      │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Двойное журналирование
//!
//! 1. **Предварительная запись** — перед обработкой кадра создаётся
//!    `/tmp/<uuid>.pending` с метаданными (spread_index, timestamp, status)
//! 2. **Подтверждение** — после успешного сохранения страниц pending удаляется
//! 3. **WAL checkpoint** — финальный сброс WAL в основную БД

use crate::session_store::{Book, SessionStore, Spread, SpreadStatus};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Структура предварительной записи (pending)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingRecord {
    /// UUID книги
    pub book_uuid: String,
    /// Индекс разворота
    pub spread_index: i64,
    /// Статус обработки
    pub status: String,
    /// Путь к левой странице (если есть)
    pub left_path: Option<String>,
    /// Путь к правой странице (если есть)
    pub right_path: Option<String>,
    /// Unix timestamp создания
    pub created_at_unix: u64,
    /// ISO 8601 дата создания
    pub created_at_iso: String,
}

/// Результат восстановления сессии
#[derive(Debug, Clone)]
pub struct RecoveryResult {
    /// UUID восстановленной книги
    pub book_uuid: String,
    /// Название книги
    pub book_name: String,
    /// Общее количество страниц
    pub total_pages: i64,
    /// Количество обработанных разворотов
    pub completed_spreads: usize,
    /// Количество ожидающих разворотов
    pub pending_spreads: usize,
    /// Количество разворотов в обработке
    pub processing_spreads: usize,
    /// Количество разворотов с ошибкой
    pub failed_spreads: usize,
    /// Последний незавершённый разворот (если есть)
    pub last_incomplete_spread: Option<Spread>,
    /// Путь к pending-файлу (если существует)
    pub pending_file: Option<PathBuf>,
}

/// Механизм восстановления сессии
pub struct SessionRecovery {
    /// Путь к директории pending-файлов
    pending_dir: PathBuf,
}

impl SessionRecovery {
    /// Создаёт новый экземпляр SessionRecovery
    ///
    /// # Аргументы
    /// * `pending_dir` — директория для pending-файлов (по умолчанию `/tmp`)
    pub fn new(pending_dir: Option<&str>) -> Self {
        let dir = pending_dir
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));

        // Создаём директорию если не существует
        if !dir.exists() {
            let _ = fs::create_dir_all(&dir);
        }

        Self { pending_dir: dir }
    }

    /// Возвращает путь к pending-файлу для указанного UUID
    pub fn pending_path(&self, book_uuid: &str) -> PathBuf {
        self.pending_dir.join(format!("{}.pending", book_uuid))
    }

    /// Восстанавливает сессию при старте приложения
    ///
    /// # Аргументы
    /// * `store` — ссылка на SessionStore
    ///
    /// # Возвращает
    /// `Result<Option<RecoveryResult>, String>` — результат восстановления или None
    pub fn recover_session(&self, store: &SessionStore) -> Result<Option<RecoveryResult>, String> {
        // 1. Получаем UUID последней незавершённой книги
        let book_uuid = match store.get_in_progress_book()? {
            Some(uuid) => uuid,
            None => {
                println!("[🔄 RECOVERY]: Незавершённых сессий не найдено");
                return Ok(None);
            }
        };

        println!("[🔄 RECOVERY]: Восстанавливаю сессию {}", book_uuid);

        // 2. Получаем данные книги
        let book: Book = match store.get_book(&book_uuid)? {
            Some(b) => b,
            None => return Err(format!("Книга {} не найдена в БД", book_uuid)),
        };

        // 3. Получаем все развороты книги
        let spreads: Vec<Spread> = store.list_spreads(&book_uuid)?;

        // 4. Считаем статусы
        let mut completed = 0;
        let mut pending = 0;
        let mut processing = 0;
        let mut failed = 0;
        let mut last_incomplete: Option<Spread> = None;

        for spread in &spreads {
            match spread.status {
                SpreadStatus::Completed => completed += 1,
                SpreadStatus::Pending => pending += 1,
                SpreadStatus::Processing => {
                    processing += 1;
                    // Разворот в обработке — кандидат на восстановление
                    if last_incomplete.is_none() {
                        last_incomplete = Some(spread.clone());
                    }
                }
                SpreadStatus::Failed => failed += 1,
            }
        }

        // 5. Если есть pending-файл — читаем его
        let pending_file = self.pending_path(&book_uuid);
        let pending_exists = pending_file.exists();

        if pending_exists {
            println!(
                "[🔄 RECOVERY]: Найден pending-файл: {}",
                pending_file.display()
            );
            if let Ok(content) = fs::read_to_string(&pending_file) {
                if let Ok(record) = serde_json::from_str::<PendingRecord>(&content) {
                    println!(
                        "[🔄 RECOVERY]: Pending: spread_index={}, status={}",
                        record.spread_index, record.status
                    );
                }
            }
        }

        // 6. Если есть разворот в обработке — переводим в pending
        if let Some(ref processing_spread) = last_incomplete {
            if let Err(e) = store.update_spread_status(
                processing_spread.id,
                SpreadStatus::Pending,
            ) {
                println!(
                    "[⚠️ RECOVERY]: Не удалось перевести spread {} в pending: {}",
                    processing_spread.id, e
                );
            } else {
                println!(
                    "[🔄 RECOVERY]: Spread {} переведён в pending",
                    processing_spread.id
                );
            }
        }

        // 7. Формируем результат
        let result = RecoveryResult {
            book_uuid: book.uuid,
            book_name: book.name,
            total_pages: book.total_pages,
            completed_spreads: completed,
            pending_spreads: pending,
            processing_spreads: processing,
            failed_spreads: failed,
            last_incomplete_spread: last_incomplete,
            pending_file: if pending_exists { Some(pending_file) } else { None },
        };

        println!(
            "[✅ RECOVERY]: Сессия восстановлена: {} страниц, {} завершено, {} pending, {} failed",
            result.total_pages,
            result.completed_spreads,
            result.pending_spreads,
            result.failed_spreads
        );

        Ok(Some(result))
    }

    /// Записывает предварительный pending-файл перед обработкой кадра
    ///
    /// # Аргументы
    /// * `book_uuid` — UUID книги
    /// * `spread_index` — индекс разворота
    /// * `status` — статус обработки
    /// * `left_path` — путь к левой странице (опционально)
    /// * `right_path` — путь к правой странице (опционально)
    ///
    /// # Возвращает
    /// `Result<PathBuf, String>` — путь к pending-файлу или описание ошибки
    pub fn write_pending(
        &self,
        book_uuid: &str,
        spread_index: i64,
        status: &str,
        left_path: Option<&str>,
        right_path: Option<&str>,
    ) -> Result<PathBuf, String> {
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let record = PendingRecord {
            book_uuid: book_uuid.to_string(),
            spread_index,
            status: status.to_string(),
            left_path: left_path.map(|s| s.to_string()),
            right_path: right_path.map(|s| s.to_string()),
            created_at_unix: now_unix,
            created_at_iso: chrono_now(),
        };

        let path = self.pending_path(book_uuid);

        // Атомарная запись: сначала во временный файл, потом rename
        let tmp_path = self.pending_dir.join(format!("{}.pending.tmp", book_uuid));
        let json = serde_json::to_string_pretty(&record)
            .map_err(|e| format!("Ошибка сериализации pending: {}", e))?;

        fs::write(&tmp_path, &json)
            .map_err(|e| format!("Ошибка записи pending: {}", e))?;

        fs::rename(&tmp_path, &path)
            .map_err(|e| format!("Ошибка rename pending: {}", e))?;

        println!(
            "[💾 PENDING]: Записан {} (spread={}, status={})",
            path.display(),
            spread_index,
            status
        );

        Ok(path)
    }

    /// Подтверждает успешный коммит — удаляет pending-файл
    ///
    /// # Аргументы
    /// * `book_uuid` — UUID книги
    ///
    /// # Возвращает
    /// `Result<(), String>` — успех или описание ошибки
    pub fn confirm_commit(&self, book_uuid: &str) -> Result<(), String> {
        let path = self.pending_path(book_uuid);

        if path.exists() {
            fs::remove_file(&path)
                .map_err(|e| format!("Ошибка удаления pending: {}", e))?;
            println!("[✅ COMMIT]: Pending подтверждён для {}", book_uuid);
        } else {
            println!("[ℹ️ COMMIT]: Pending не найден для {}", book_uuid);
        }

        Ok(())
    }

    /// Выполняет WAL checkpoint для сброса WAL в основную БД
    ///
    /// # Аргументы
    /// * `store` — ссылка на SessionStore
    ///
    /// # Возвращает
    /// `Result<(), String>` — успех или описание ошибки
    pub fn wal_checkpoint(&self, store: &SessionStore) -> Result<(), String> {
        // Используем PRAGMA wal_checkpoint(TRUNCATE)
        // Это сбрасывает WAL-файл и записывает все данные в основную БД
        let result = store.execute_pragma("wal_checkpoint(TRUNCATE)");
        match result {
            Ok(_) => {
                println!("[💾 WAL]: Checkpoint выполнен успешно");
                Ok(())
            }
            Err(e) => Err(format!("Ошибка WAL checkpoint: {}", e)),
        }
    }

    /// Очищает устаревшие pending-файлы старше указанного времени
    ///
    /// # Аргументы
    /// * `max_age` — максимальный возраст pending-файла
    ///
    /// # Возвращает
    /// `Result<Vec<PathBuf>, String>` — список удалённых файлов
    pub fn cleanup_stale_pending(&self, max_age: Duration) -> Result<Vec<PathBuf>, String> {
        let mut removed = Vec::new();
        let now = SystemTime::now();

        let entries = fs::read_dir(&self.pending_dir)
            .map_err(|e| format!("Ошибка чтения директории: {}", e))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !file_name.ends_with(".pending") {
                continue;
            }

            // Проверяем возраст файла
            if let Ok(metadata) = fs::metadata(&path) {
                if let Ok(modified) = metadata.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age > max_age {
                            if fs::remove_file(&path).is_ok() {
                                println!(
                                    "[🧹 CLEANUP]: Удалён устаревший pending: {}",
                                    path.display()
                                );
                                removed.push(path);
                            }
                        }
                    }
                }
            }
        }

        Ok(removed)
    }

    /// Возвращает путь к директории pending-файлов
    pub fn pending_dir(&self) -> &Path {
        &self.pending_dir
    }
}

impl Default for SessionRecovery {
    fn default() -> Self {
        Self::new(None)
    }
}

/// Возвращает текущую дату/время в формате ISO 8601
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();

    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year_from_unix(secs),
        month_from_unix(secs),
        day_from_unix(secs),
        hour_from_unix(secs),
        minute_from_unix(secs),
        second_from_unix(secs),
        millis
    )
}

fn year_from_unix(secs: u64) -> u32 {
    let mut days = secs / 86400;
    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    year
}

fn month_from_unix(secs: u64) -> u32 {
    let days = secs / 86400;
    let year = year_from_unix(secs);
    let days_in_year = if is_leap_year(year) { 366 } else { 365 };
    let day_of_year = days % days_in_year;

    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    let mut remaining = day_of_year;
    for &md in &month_days {
        let actual_md = if month == 2 && is_leap_year(year) { 29 } else { md };
        if remaining < actual_md {
            break;
        }
        remaining -= actual_md;
        month += 1;
    }
    month
}

fn day_from_unix(secs: u64) -> u32 {
    let days = secs / 86400;
    let year = year_from_unix(secs);
    let days_in_year = if is_leap_year(year) { 366 } else { 365 };
    let day_of_year = days % days_in_year;

    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut remaining = day_of_year;
    for (i, &md) in month_days.iter().enumerate() {
        let actual_md = if i == 1 && is_leap_year(year) { 29 } else { md };
        if remaining < actual_md {
            return (remaining + 1) as u32;
        }
        remaining -= actual_md;
    }
    1
}

fn hour_from_unix(secs: u64) -> u32 {
    ((secs % 86400) / 3600) as u32
}

fn minute_from_unix(secs: u64) -> u32 {
    ((secs % 3600) / 60) as u32
}

fn second_from_unix(secs: u64) -> u32 {
    (secs % 60) as u32
}

fn is_leap_year(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("scan_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn test_pending_write_and_read() {
        let dir = temp_dir();
        let recovery = SessionRecovery::new(Some(dir.to_str().unwrap()));

        let path = recovery
            .write_pending("test-uuid", 42, "processing", Some("/tmp/left.tiff"), None)
            .unwrap();

        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        let record: PendingRecord = serde_json::from_str(&content).unwrap();

        assert_eq!(record.book_uuid, "test-uuid");
        assert_eq!(record.spread_index, 42);
        assert_eq!(record.status, "processing");
        assert_eq!(record.left_path, Some("/tmp/left.tiff".to_string()));
        assert_eq!(record.right_path, None);

        // Cleanup
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn test_confirm_commit_removes_pending() {
        let dir = temp_dir();
        let recovery = SessionRecovery::new(Some(dir.to_str().unwrap()));

        let path = recovery
            .write_pending("test-uuid-2", 0, "pending", None, None)
            .unwrap();
        assert!(path.exists());

        recovery.confirm_commit("test-uuid-2").unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_recover_session_no_books() {
        let dir = temp_dir();
        let recovery = SessionRecovery::new(Some(dir.to_str().unwrap()));
        let store = SessionStore::new(":memory:").unwrap();

        let result = recovery.recover_session(&store).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_recover_session_with_book() {
        let dir = temp_dir();
        let recovery = SessionRecovery::new(Some(dir.to_str().unwrap()));
        let store = SessionStore::new(":memory:").unwrap();

        // Создаём книгу и развороты
        let book = store.create_book("Тестовая книга").unwrap();
        store.add_spread(&book.uuid, 0, None, None).unwrap();
        store.add_spread(&book.uuid, 1, None, None).unwrap();
        store.add_spread(&book.uuid, 2, None, None).unwrap();

        // Завершаем первый разворот
        let spread0 = store.get_last_spread(&book.uuid).unwrap().unwrap();
        store.update_spread_status(spread0.id, SpreadStatus::Completed).unwrap();

        // Второй разворот — в обработке
        let spreads = store.list_spreads(&book.uuid).unwrap();
        let spread1 = spreads.iter().find(|s| s.spread_index == 1).unwrap();
        store.update_spread_status(spread1.id, SpreadStatus::Processing).unwrap();

        // Восстанавливаем сессию
        let result = recovery.recover_session(&store).unwrap().unwrap();

        assert_eq!(result.book_uuid, book.uuid);
        assert_eq!(result.completed_spreads, 1);
        assert_eq!(result.pending_spreads, 1);
        assert_eq!(result.processing_spreads, 1);

        // Spread в обработке должен быть переведён в pending
        let spreads_after = store.list_spreads(&book.uuid).unwrap();
        let spread1_after = spreads_after.iter().find(|s| s.spread_index == 1).unwrap();
        assert_eq!(spread1_after.status, SpreadStatus::Pending);
    }

    #[test]
    fn test_cleanup_stale_pending() {
        let dir = temp_dir();
        let recovery = SessionRecovery::new(Some(dir.to_str().unwrap()));

        // Создаём pending-файл
        let path = recovery
            .write_pending("stale-uuid", 0, "pending", None, None)
            .unwrap();

        // Устанавливаем старый mtime (1 час назад)
        use filetime::{FileTime, set_file_mtime};
        let old_time = SystemTime::now() - Duration::from_secs(3600);
        let filetime = FileTime::from_system_time(old_time);
        set_file_mtime(&path, filetime).unwrap();

        // Очищаем pending старше 30 минут
        let removed = recovery.cleanup_stale_pending(Duration::from_secs(1800)).unwrap();
        assert_eq!(removed.len(), 1);
        assert!(!path.exists());
    }

    #[test]
    fn test_pending_path_format() {
        let dir = temp_dir();
        let recovery = SessionRecovery::new(Some(dir.to_str().unwrap()));

        let path = recovery.pending_path("my-uuid");
        assert!(path.ends_with("my-uuid.pending"));
    }
}