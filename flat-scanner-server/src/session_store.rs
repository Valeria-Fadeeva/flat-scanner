//! # Session Store — Транзакционное хранение сессий сканирования
//!
//! Модуль реализует SQLite-хранилище для управления книгами и разворотами.
//! Поддерживает WAL-режим для конкурентного доступа и атомарные транзакции.
//!
//! ## Архитектура
//!
//! ```text
//! ┌─────────────────────────────────────────┐
//! │ SessionStore (pub)                      │
//! │ ├─ create_book()                       │
//! │ ├─ add_spread()                        │
//! │ ├─ update_spread_status()              │
//! │ ├─ get_in_progress_book()              │
//! │ ├─ list_books()                        │
//! │ └─ close()                             │
//! └─────────────────────────────────────────┘
//! ```
//!
//! ## Схема БД
//!
//! ```sql
//! CREATE TABLE books (
//!     uuid TEXT PRIMARY KEY,
//!     name TEXT NOT NULL,
//!     start_date TEXT NOT NULL,
//!     total_pages INTEGER DEFAULT 0,
//!     status TEXT NOT NULL DEFAULT 'in_progress',
//!     created_at TEXT NOT NULL,
//!     updated_at TEXT NOT NULL
//! );
//!
//! CREATE TABLE spreads (
//!     id INTEGER PRIMARY KEY AUTOINCREMENT,
//!     book_uuid TEXT NOT NULL,
//!     spread_index INTEGER NOT NULL,
//!     left_path TEXT,
//!     right_path TEXT,
//!     left_vertices TEXT,
//!     right_vertices TEXT,
//!     threshold_k REAL,
//!     status TEXT NOT NULL DEFAULT 'pending',
//!     created_at TEXT NOT NULL,
//!     updated_at TEXT NOT NULL,
//!     FOREIGN KEY (book_uuid) REFERENCES books(uuid) ON DELETE CASCADE,
//!     UNIQUE(book_uuid, spread_index)
//! );
//! ```

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// Статус книги в сессии
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BookStatus {
    /// Книга в процессе сканирования
    InProgress,
    /// Книга завершена
    Completed,
    /// Книга отменена
    Cancelled,
    /// Книга на паузе
    Paused,
}

impl BookStatus {
    /// Преобразование в строку для хранения в БД
    pub fn as_str(&self) -> &'static str {
        match self {
            BookStatus::InProgress => "in_progress",
            BookStatus::Completed => "completed",
            BookStatus::Cancelled => "cancelled",
            BookStatus::Paused => "paused",
        }
    }

    /// Преобразование из строки
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "in_progress" => Some(BookStatus::InProgress),
            "completed" => Some(BookStatus::Completed),
            "cancelled" => Some(BookStatus::Cancelled),
            "paused" => Some(BookStatus::Paused),
            _ => None,
        }
    }
}

/// Статус разворота (спреда)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpreadStatus {
    /// Ожидает обработки
    Pending,
    /// Обработка в процессе
    Processing,
    /// Обработан успешно
    Completed,
    /// Ошибка обработки
    Failed,
}

impl SpreadStatus {
    /// Преобразование в строку для хранения в БД
    pub fn as_str(&self) -> &'static str {
        match self {
            SpreadStatus::Pending => "pending",
            SpreadStatus::Processing => "processing",
            SpreadStatus::Completed => "completed",
            SpreadStatus::Failed => "failed",
        }
    }

    /// Преобразование из строки
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(SpreadStatus::Pending),
            "processing" => Some(SpreadStatus::Processing),
            "completed" => Some(SpreadStatus::Completed),
            "failed" => Some(SpreadStatus::Failed),
            _ => None,
        }
    }
}

/// Структура книги
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Book {
    /// Уникальный идентификатор книги
    pub uuid: String,
    /// Название книги
    pub name: String,
    /// Дата начала сканирования (ISO 8601)
    pub start_date: String,
    /// Общее количество страниц
    pub total_pages: i64,
    /// Статус книги
    pub status: BookStatus,
    /// Дата создания записи
    pub created_at: String,
    /// Дата последнего обновления
    pub updated_at: String,
}

/// Структура разворота (спреда)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spread {
    /// Идентификатор записи в БД
    pub id: i64,
    /// UUID книги
    pub book_uuid: String,
    /// Индекс разворота (0-based)
    pub spread_index: i64,
    /// Путь к левой странице
    pub left_path: Option<String>,
    /// Путь к правой странице
    pub right_path: Option<String>,
    /// Вершины левой страницы (JSON)
    pub left_vertices: Option<String>,
    /// Вершины правой страницы (JSON)
    pub right_vertices: Option<String>,
    /// Коэффициент Sauvola
    pub threshold_k: Option<f64>,
    /// Статус разворота
    pub status: SpreadStatus,
    /// Дата создания записи
    pub created_at: String,
    /// Дата последнего обновления
    pub updated_at: String,
}

/// Транзакционный хранилище сессий сканирования
///
/// Обёртка над SQLite-подключением с поддержкой WAL-режима и транзакций.
/// Все операции возвращают `Result<T, String>` для безопасной обработки ошибок.
pub struct SessionStore {
    conn: Connection,
    db_path: String,
}

impl SessionStore {
    /// Создаёт новое хранилище сессий
    ///
    /// # Аргументы
    /// * `db_path` — путь к файлу SQLite (будет создан если не существует)
    ///
    /// # Возвращает
    /// `Result<SessionStore, String>` — хранилище или описание ошибки
    pub fn new(db_path: &str) -> Result<Self, String> {
        // Создаём директорию если нужно
        if let Some(parent) = Path::new(db_path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Ошибка создания директории {}: {}", parent.display(), e))?;
            }
        }

        let conn = Connection::open(db_path)
            .map_err(|e| format!("Ошибка открытия SQLite: {}", e))?;

        // Включаем WAL-режим для конкурентного доступа
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("Ошибка установки WAL: {}", e))?;

        // Включаем foreign keys
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| format!("Ошибка включения FK: {}", e))?;

        // Создаём схему БД
        let store = Self {
            conn,
            db_path: db_path.to_string(),
        };

        store.create_schema()?;

        Ok(store)
    }

    /// Создаёт схему БД (таблицы books и spreads)
    fn create_schema(&self) -> Result<(), String> {
        let schema = r#"
            CREATE TABLE IF NOT EXISTS books (
                uuid TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                start_date TEXT NOT NULL,
                total_pages INTEGER DEFAULT 0,
                status TEXT NOT NULL DEFAULT 'in_progress',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS spreads (
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

            CREATE INDEX IF NOT EXISTS idx_books_status ON books(status);
            CREATE INDEX IF NOT EXISTS idx_spreads_book ON spreads(book_uuid);
            CREATE INDEX IF NOT EXISTS idx_spreads_status ON spreads(status);
        "#;

        self.conn
            .execute_batch(schema)
            .map_err(|e| format!("Ошибка создания схемы: {}", e))?;

        Ok(())
    }

    /// Создаёт новую книгу в сессии
    ///
    /// # Аргументы
    /// * `name` — название книги
    ///
    /// # Возвращает
    /// `Result<Book, String>` — созданная книга или описание ошибки
    pub fn create_book(&self, name: &str) -> Result<Book, String> {
        let uuid = Uuid::new_v4().to_string();
        let now = chrono_now();

        let book = Book {
            uuid: uuid.clone(),
            name: name.to_string(),
            start_date: now.clone(),
            total_pages: 0,
            status: BookStatus::InProgress,
            created_at: now.clone(),
            updated_at: now,
        };

        self.conn
            .execute(
                "INSERT INTO books (uuid, name, start_date, total_pages, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    book.uuid,
                    book.name,
                    book.start_date,
                    book.total_pages,
                    book.status.as_str(),
                    book.created_at,
                    book.updated_at
                ],
            )
            .map_err(|e| format!("Ошибка создания книги: {}", e))?;

        Ok(book)
    }

    /// Добавляет разворот к книге
    ///
    /// # Аргументы
    /// * `book_uuid` — UUID книги
    /// * `spread_index` — индекс разворота
    /// * `left_path` — путь к левой странице (опционально)
    /// * `right_path` — путь к правой странице (опционально)
    ///
    /// # Возвращает
    /// `Result<Spread, String>` — созданный разворот или описание ошибки
    pub fn add_spread(
        &self,
        book_uuid: &str,
        spread_index: i64,
        left_path: Option<&str>,
        right_path: Option<&str>,
    ) -> Result<Spread, String> {
        let now = chrono_now();

        // Проверяем существование книги
        let book_exists: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM books WHERE uuid = ?1",
                params![book_uuid],
                |row| row.get(0),
            )
            .map_err(|e| format!("Ошибка проверки книги: {}", e))?;

        if book_exists == 0 {
            return Err(format!("Книга с UUID {} не найдена", book_uuid));
        }

        let spread = Spread {
            id: 0, // будет заполнено после INSERT
            book_uuid: book_uuid.to_string(),
            spread_index,
            left_path: left_path.map(|s| s.to_string()),
            right_path: right_path.map(|s| s.to_string()),
            left_vertices: None,
            right_vertices: None,
            threshold_k: None,
            status: SpreadStatus::Pending,
            created_at: now.clone(),
            updated_at: now,
        };

        self.conn
            .execute(
                "INSERT INTO spreads (book_uuid, spread_index, left_path, right_path, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    spread.book_uuid,
                    spread.spread_index,
                    spread.left_path,
                    spread.right_path,
                    spread.status.as_str(),
                    spread.created_at,
                    spread.updated_at
                ],
            )
            .map_err(|e| format!("Ошибка добавления разворота: {}", e))?;

        // Получаем ID созданной записи
        let id = self.conn.last_insert_rowid();

        Ok(Spread {
            id,
            ..spread
        })
    }

    /// Обновляет статус разворота
    ///
    /// # Аргументы
    /// * `spread_id` — ID разворота
    /// * `status` — новый статус
    ///
    /// # Возвращает
    /// `Result<(), String>` — успех или описание ошибки
    pub fn update_spread_status(&self, spread_id: i64, status: SpreadStatus) -> Result<(), String> {
        let now = chrono_now();

        let rows_affected = self
            .conn
            .execute(
                "UPDATE spreads SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status.as_str(), now, spread_id],
            )
            .map_err(|e| format!("Ошибка обновления статуса: {}", e))?;

        if rows_affected == 0 {
            return Err(format!("Разворот с ID {} не найден", spread_id));
        }

        Ok(())
    }

    /// Обновляет пути к страницам разворота
    ///
    /// # Аргументы
    /// * `spread_id` — ID разворота
    /// * `left_path` — новый путь к левой странице
    /// * `right_path` — новый путь к правой странице
    ///
    /// # Возвращает
    /// `Result<(), String>` — успех или описание ошибки
    pub fn update_spread_paths(
        &self,
        spread_id: i64,
        left_path: &str,
        right_path: &str,
    ) -> Result<(), String> {
        let now = chrono_now();

        let rows_affected = self
            .conn
            .execute(
                "UPDATE spreads SET left_path = ?1, right_path = ?2, updated_at = ?3 WHERE id = ?4",
                params![left_path, right_path, now, spread_id],
            )
            .map_err(|e| format!("Ошибка обновления путей: {}", e))?;

        if rows_affected == 0 {
            return Err(format!("Разворот с ID {} не найден", spread_id));
        }

        Ok(())
    }

    /// Обновляет вершины разворота
    ///
    /// # Аргументы
    /// * `spread_id` — ID разворота
    /// * `left_vertices` — JSON-строка с вершинами левой страницы
    /// * `right_vertices` — JSON-строка с вершинами правой страницы
    ///
    /// # Возвращает
    /// `Result<(), String>` — успех или описание ошибки
    pub fn update_spread_vertices(
        &mut self,
        spread_id: i64,
        left_vertices: &str,
        right_vertices: &str,
    ) -> Result<(), String> {
        let now = chrono_now();

        // §1.3: Атомарная транзакция — комплексное обновление вершин + timestamp
        let tx = self.conn.transaction().map_err(|e| format!("Ошибка транзакции: {}", e))?;

        let rows_affected = tx
            .execute(
                "UPDATE spreads SET left_vertices = ?1, right_vertices = ?2, updated_at = ?3 WHERE id = ?4",
                params![left_vertices, right_vertices, now, spread_id],
            )
            .map_err(|e| format!("Ошибка обновления вершин: {}", e))?;

        if rows_affected == 0 {
            return Err(format!("Разворот с ID {} не найден", spread_id));
        }

        tx.commit().map_err(|e| format!("Ошибка commit: {}", e))?;
        Ok(())
    }

    /// Обновляет статус книги
    ///
    /// # Аргументы
    /// * `book_uuid` — UUID книги
    /// * `status` — новый статус
    ///
    /// # Возвращает
    /// `Result<(), String>` — успех или описание ошибки
    pub fn update_book_status(&self, book_uuid: &str, status: BookStatus) -> Result<(), String> {
        let now = chrono_now();

        let rows_affected = self
            .conn
            .execute(
                "UPDATE books SET status = ?1, updated_at = ?2 WHERE uuid = ?3",
                params![status.as_str(), now, book_uuid],
            )
            .map_err(|e| format!("Ошибка обновления статуса книги: {}", e))?;

        if rows_affected == 0 {
            return Err(format!("Книга с UUID {} не найдена", book_uuid));
        }

        Ok(())
    }

    /// Обновляет количество страниц книги
    ///
    /// # Аргументы
    /// * `book_uuid` — UUID книги
    /// * `total_pages` — новое количество страниц
    ///
    /// # Возвращает
    /// `Result<(), String>` — успех или описание ошибки
    pub fn update_book_total_pages(&self, book_uuid: &str, total_pages: i64) -> Result<(), String> {
        let now = chrono_now();

        let rows_affected = self
            .conn
            .execute(
                "UPDATE books SET total_pages = ?1, updated_at = ?2 WHERE uuid = ?3",
                params![total_pages, now, book_uuid],
            )
            .map_err(|e| format!("Ошибка обновления количества страниц: {}", e))?;

        if rows_affected == 0 {
            return Err(format!("Книга с UUID {} не найдена", book_uuid));
        }

        Ok(())
    }

    /// Получает книгу по UUID
    ///
    /// # Аргументы
    /// * `book_uuid` — UUID книги
    ///
    /// # Возвращает
    /// `Result<Option<Book>, String>` — книга или None если не найдена
    pub fn get_book(&self, book_uuid: &str) -> Result<Option<Book>, String> {
        let result = self.conn.query_row(
            "SELECT uuid, name, start_date, total_pages, status, created_at, updated_at
             FROM books WHERE uuid = ?1",
            params![book_uuid],
            |row| {
                Ok(Book {
                    uuid: row.get(0)?,
                    name: row.get(1)?,
                    start_date: row.get(2)?,
                    total_pages: row.get(3)?,
                    status: BookStatus::from_str(&row.get::<_, String>(4)?)
                        .unwrap_or(BookStatus::InProgress),
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        );

        match result {
            Ok(book) => Ok(Some(book)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("Ошибка получения книги: {}", e)),
        }
    }

    /// Получает разворот по ID
    ///
    /// # Аргументы
    /// * `spread_id` — ID разворота
    ///
    /// # Возвращает
    /// `Result<Option<Spread>, String>` — разворот или None если не найден
    pub fn get_spread(&self, spread_id: i64) -> Result<Option<Spread>, String> {
        let result = self.conn.query_row(
            "SELECT id, book_uuid, spread_index, left_path, right_path, left_vertices, right_vertices,
                    threshold_k, status, created_at, updated_at
             FROM spreads WHERE id = ?1",
            params![spread_id],
            |row| {
                Ok(Spread {
                    id: row.get(0)?,
                    book_uuid: row.get(1)?,
                    spread_index: row.get(2)?,
                    left_path: row.get(3)?,
                    right_path: row.get(4)?,
                    left_vertices: row.get(5)?,
                    right_vertices: row.get(6)?,
                    threshold_k: row.get(7)?,
                    status: SpreadStatus::from_str(&row.get::<_, String>(8)?)
                        .unwrap_or(SpreadStatus::Pending),
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            },
        );

        match result {
            Ok(spread) => Ok(Some(spread)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("Ошибка получения разворота: {}", e)),
        }
    }

    /// Получает последний незавершённый разворот для книги
    ///
    /// # Аргументы
    /// * `book_uuid` — UUID книги
    ///
    /// # Возвращает
    /// `Result<Option<Spread>, String>` — разворот или None если не найден
    pub fn get_last_spread(&self, book_uuid: &str) -> Result<Option<Spread>, String> {
        let result = self.conn.query_row(
            "SELECT id, book_uuid, spread_index, left_path, right_path, left_vertices, right_vertices,
                    threshold_k, status, created_at, updated_at
             FROM spreads WHERE book_uuid = ?1
             ORDER BY spread_index DESC LIMIT 1",
            params![book_uuid],
            |row| {
                Ok(Spread {
                    id: row.get(0)?,
                    book_uuid: row.get(1)?,
                    spread_index: row.get(2)?,
                    left_path: row.get(3)?,
                    right_path: row.get(4)?,
                    left_vertices: row.get(5)?,
                    right_vertices: row.get(6)?,
                    threshold_k: row.get(7)?,
                    status: SpreadStatus::from_str(&row.get::<_, String>(8)?)
                        .unwrap_or(SpreadStatus::Pending),
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            },
        );

        match result {
            Ok(spread) => Ok(Some(spread)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("Ошибка получения последнего разворота: {}", e)),
        }
    }

    /// Получает список всех книг
    ///
    /// # Возвращает
    /// `Result<Vec<Book>, String>` — список книг или описание ошибки
    pub fn list_books(&self) -> Result<Vec<Book>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT uuid, name, start_date, total_pages, status, created_at, updated_at
                      FROM books ORDER BY created_at DESC")
            .map_err(|e| format!("Ошибка подготовки запроса: {}", e))?;

        let books_iter = stmt
            .query_map([], |row| {
                Ok(Book {
                    uuid: row.get(0)?,
                    name: row.get(1)?,
                    start_date: row.get(2)?,
                    total_pages: row.get(3)?,
                    status: BookStatus::from_str(&row.get::<_, String>(4)?)
                        .unwrap_or(BookStatus::InProgress),
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            })
            .map_err(|e| format!("Ошибка выполнения запроса: {}", e))?;

        let mut books = Vec::new();
        for book in books_iter {
            books.push(book.map_err(|e| format!("Ошибка чтения строки: {}", e))?);
        }

        Ok(books)
    }

    /// Получает список всех разворотов для книги
    ///
    /// # Аргументы
    /// * `book_uuid` — UUID книги
    ///
    /// # Возвращает
    /// `Result<Vec<Spread>, String>` — список разворотов или описание ошибки
    pub fn list_spreads(&self, book_uuid: &str) -> Result<Vec<Spread>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, book_uuid, spread_index, left_path, right_path, left_vertices, right_vertices,
                        threshold_k, status, created_at, updated_at
                 FROM spreads WHERE book_uuid = ?1 ORDER BY spread_index ASC",
            )
            .map_err(|e| format!("Ошибка подготовки запроса: {}", e))?;

        let spreads_iter = stmt
            .query_map(params![book_uuid], |row| {
                Ok(Spread {
                    id: row.get(0)?,
                    book_uuid: row.get(1)?,
                    spread_index: row.get(2)?,
                    left_path: row.get(3)?,
                    right_path: row.get(4)?,
                    left_vertices: row.get(5)?,
                    right_vertices: row.get(6)?,
                    threshold_k: row.get(7)?,
                    status: SpreadStatus::from_str(&row.get::<_, String>(8)?)
                        .unwrap_or(SpreadStatus::Pending),
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            })
            .map_err(|e| format!("Ошибка выполнения запроса: {}", e))?;

        let mut spreads = Vec::new();
        for spread in spreads_iter {
            spreads.push(spread.map_err(|e| format!("Ошибка чтения строки: {}", e))?);
        }

        Ok(spreads)
    }

    /// Получает последний незавершённый UUID книги
    ///
    /// # Возвращает
    /// `Result<Option<String>, String>` — UUID книги или None если не найдена
    pub fn get_in_progress_book(&self) -> Result<Option<String>, String> {
        let result = self.conn.query_row(
            "SELECT uuid FROM books WHERE status = 'in_progress' ORDER BY updated_at DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        );

        match result {
            Ok(uuid) => Ok(Some(uuid)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("Ошибка получения незавершённой книги: {}", e)),
        }
    }

    /// Удаляет книгу и все связанные развороты
    ///
    /// # Аргументы
    /// * `book_uuid` — UUID книги
    ///
    /// # Возвращает
    /// `Result<(), String>` — успех или описание ошибки
    pub fn delete_book(&self, book_uuid: &str) -> Result<(), String> {
        let rows_affected = self
            .conn
            .execute("DELETE FROM books WHERE uuid = ?1", params![book_uuid])
            .map_err(|e| format!("Ошибка удаления книги: {}", e))?;

        if rows_affected == 0 {
            return Err(format!("Книга с UUID {} не найдена", book_uuid));
        }

        Ok(())
    }

    /// Закрывает соединение с БД
    pub fn close(&self) -> Result<(), String> {
        // SQLite автоматически закрывает соединение при уничтожении объекта
        Ok(())
    }

    /// Выполняет произвольную PRAGMA команду
    ///
    /// # Аргументы
    /// * `pragma` — строка PRAGMA (например, "wal_checkpoint(TRUNCATE)")
    ///
    /// # Возвращает
    /// `Result<(), String>` — успех или описание ошибки
    pub fn execute_pragma(&self, pragma: &str) -> Result<(), String> {
        let sql = if pragma.to_uppercase().starts_with("PRAGMA") {
            pragma.to_string()
        } else {
            format!("PRAGMA {}", pragma)
        };
        self.conn
            .execute_batch(&sql)
            .map_err(|e| format!("Ошибка выполнения PRAGMA '{}': {}", pragma, e))
    }

    /// Возвращает путь к файлу БД
    pub fn db_path(&self) -> &str {
        &self.db_path
    }
}

/// Глобальный экземпляр SessionStore (ленивая инициализация)
static GLOBAL_SESSION_STORE: std::sync::OnceLock<Arc<Mutex<SessionStore>>> = std::sync::OnceLock::new();

/// Получает глобальный экземпляр SessionStore
///
/// # Аргументы
/// * `db_path` — путь к файлу SQLite (используется только при первой инициализации)
///
/// # Возвращает
/// `Arc<Mutex<SessionStore>>` — глобальное хранилище
pub fn global_session_store(db_path: &str) -> Arc<Mutex<SessionStore>> {
    GLOBAL_SESSION_STORE
        .get_or_init(|| {
            let store = SessionStore::new(db_path)
                .unwrap_or_else(|e| {
                    eprintln!("[⚠️ SESSION STORE] Ошибка инициализации: {}", e);
                    SessionStore::new(":memory:")
                        .unwrap_or_else(|e2| panic!("Критическая ошибка: {}", e2))
                });
            Arc::new(Mutex::new(store))
        })
        .clone()
}

/// Возвращает текущую дату/время в формате ISO 8601
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();

    // Простое преобразование Unix timestamp в ISO 8601
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

    #[test]
    fn test_create_and_get_book() {
        let store = SessionStore::new(":memory:").unwrap();
        let book = store.create_book("Тестовая книга").unwrap();

        assert!(!book.uuid.is_empty());
        assert_eq!(book.name, "Тестовая книга");
        assert_eq!(book.status, BookStatus::InProgress);

        let fetched = store.get_book(&book.uuid).unwrap().unwrap();
        assert_eq!(fetched.uuid, book.uuid);
        assert_eq!(fetched.name, book.name);
    }

    #[test]
    fn test_add_and_get_spread() {
        let store = SessionStore::new(":memory:").unwrap();
        let book = store.create_book("Книга для тестов").unwrap();

        let spread = store
            .add_spread(&book.uuid, 0, Some("/tmp/left.tiff"), Some("/tmp/right.tiff"))
            .unwrap();

        assert_eq!(spread.book_uuid, book.uuid);
        assert_eq!(spread.spread_index, 0);
        assert_eq!(spread.status, SpreadStatus::Pending);

        let fetched = store.get_spread(spread.id).unwrap().unwrap();
        assert_eq!(fetched.id, spread.id);
        assert_eq!(fetched.left_path, Some("/tmp/left.tiff".to_string()));
    }

    #[test]
    fn test_update_spread_status() {
        let store = SessionStore::new(":memory:").unwrap();
        let book = store.create_book("Книга").unwrap();
        let spread = store.add_spread(&book.uuid, 0, None, None).unwrap();

        store
            .update_spread_status(spread.id, SpreadStatus::Processing)
            .unwrap();

        let fetched = store.get_spread(spread.id).unwrap().unwrap();
        assert_eq!(fetched.status, SpreadStatus::Processing);

        store
            .update_spread_status(spread.id, SpreadStatus::Completed)
            .unwrap();

        let fetched = store.get_spread(spread.id).unwrap().unwrap();
        assert_eq!(fetched.status, SpreadStatus::Completed);
    }

    #[test]
    fn test_update_book_status() {
        let store = SessionStore::new(":memory:").unwrap();
        let book = store.create_book("Книга").unwrap();

        store
            .update_book_status(&book.uuid, BookStatus::Completed)
            .unwrap();

        let fetched = store.get_book(&book.uuid).unwrap().unwrap();
        assert_eq!(fetched.status, BookStatus::Completed);
    }

    #[test]
    fn test_list_books() {
        let store = SessionStore::new(":memory:").unwrap();
        store.create_book("Книга 1").unwrap();
        store.create_book("Книга 2").unwrap();
        store.create_book("Книга 3").unwrap();

        let books = store.list_books().unwrap();
        assert_eq!(books.len(), 3);
    }

    #[test]
    fn test_list_spreads() {
        let store = SessionStore::new(":memory:").unwrap();
        let book = store.create_book("Книга").unwrap();

        store.add_spread(&book.uuid, 0, None, None).unwrap();
        store.add_spread(&book.uuid, 1, None, None).unwrap();
        store.add_spread(&book.uuid, 2, None, None).unwrap();

        let spreads = store.list_spreads(&book.uuid).unwrap();
        assert_eq!(spreads.len(), 3);
        assert_eq!(spreads[0].spread_index, 0);
        assert_eq!(spreads[1].spread_index, 1);
        assert_eq!(spreads[2].spread_index, 2);
    }

    #[test]
    fn test_get_in_progress_book() {
        let store = SessionStore::new(":memory:").unwrap();
        let book1 = store.create_book("Книга 1").unwrap();
        let book2 = store.create_book("Книга 2").unwrap();

        // Обе книги в статусе in_progress
        let in_progress = store.get_in_progress_book().unwrap();
        assert!(in_progress.is_some());

        // Завершаем первую книгу
        store
            .update_book_status(&book1.uuid, BookStatus::Completed)
            .unwrap();

        // Должна вернуть вторую книгу
        let in_progress = store.get_in_progress_book().unwrap().unwrap();
        assert_eq!(in_progress, book2.uuid);
    }

    #[test]
    fn test_delete_book() {
        let store = SessionStore::new(":memory:").unwrap();
        let book = store.create_book("Книга для удаления").unwrap();
        store.add_spread(&book.uuid, 0, None, None).unwrap();

        store.delete_book(&book.uuid).unwrap();

        let fetched = store.get_book(&book.uuid).unwrap();
        assert!(fetched.is_none());
    }

    #[test]
    fn test_book_status_parsing() {
        assert_eq!(BookStatus::from_str("in_progress"), Some(BookStatus::InProgress));
        assert_eq!(BookStatus::from_str("completed"), Some(BookStatus::Completed));
        assert_eq!(BookStatus::from_str("cancelled"), Some(BookStatus::Cancelled));
        assert_eq!(BookStatus::from_str("paused"), Some(BookStatus::Paused));
        assert_eq!(BookStatus::from_str("invalid"), None);
    }

    #[test]
    fn test_spread_status_parsing() {
        assert_eq!(SpreadStatus::from_str("pending"), Some(SpreadStatus::Pending));
        assert_eq!(SpreadStatus::from_str("processing"), Some(SpreadStatus::Processing));
        assert_eq!(SpreadStatus::from_str("completed"), Some(SpreadStatus::Completed));
        assert_eq!(SpreadStatus::from_str("failed"), Some(SpreadStatus::Failed));
        assert_eq!(SpreadStatus::from_str("invalid"), None);
    }

    #[test]
    fn test_update_spread_paths() {
        let store = SessionStore::new(":memory:").unwrap();
        let book = store.create_book("Книга").unwrap();
        let spread = store.add_spread(&book.uuid, 0, None, None).unwrap();

        store
            .update_spread_paths(spread.id, "/new/left.tiff", "/new/right.tiff")
            .unwrap();

        let fetched = store.get_spread(spread.id).unwrap().unwrap();
        assert_eq!(fetched.left_path, Some("/new/left.tiff".to_string()));
        assert_eq!(fetched.right_path, Some("/new/right.tiff".to_string()));
    }

    #[test]
    fn test_update_spread_vertices() {
        let mut store = SessionStore::new(":memory:").unwrap();
        let book = store.create_book("Книга").unwrap();
        let spread = store.add_spread(&book.uuid, 0, None, None).unwrap();

        let left_vertices = r#"{"p1":{"x":100,"y":200},"p2":{"x":300,"y":200}}"#;
        let right_vertices = r#"{"p1":{"x":400,"y":200},"p2":{"x":600,"y":200}}"#;

        store
            .update_spread_vertices(spread.id, left_vertices, right_vertices)
            .unwrap();

        let fetched = store.get_spread(spread.id).unwrap().unwrap();
        assert_eq!(fetched.left_vertices, Some(left_vertices.to_string()));
        assert_eq!(fetched.right_vertices, Some(right_vertices.to_string()));
    }

    #[test]
    fn test_update_book_total_pages() {
        let store = SessionStore::new(":memory:").unwrap();
        let book = store.create_book("Книга").unwrap();

        store.update_book_total_pages(&book.uuid, 400).unwrap();

        let fetched = store.get_book(&book.uuid).unwrap().unwrap();
        assert_eq!(fetched.total_pages, 400);
    }

    #[test]
    fn test_get_last_spread() {
        let store = SessionStore::new(":memory:").unwrap();
        let book = store.create_book("Книга").unwrap();

        store.add_spread(&book.uuid, 0, None, None).unwrap();
        store.add_spread(&book.uuid, 1, None, None).unwrap();
        store.add_spread(&book.uuid, 2, None, None).unwrap();

        let last = store.get_last_spread(&book.uuid).unwrap().unwrap();
        assert_eq!(last.spread_index, 2);
    }
}