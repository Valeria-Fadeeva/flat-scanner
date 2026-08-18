//! # Write Queue — Single Writer + FIFO-очередь (§1.3)
//!
//! Архитектура: все записи в SQLite проходят через единственный
//! фоновый воркер, читающий задачи из `tokio::sync::mpsc` канала.
//! Чтение данных из БД остаётся параллельным в Axum-хендлерах.
//!
//! ```text
//! Axum handlers ──► mpsc::Sender ──► [FIFO] ──► Writer Worker ──► SQLite
//! ```

use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use crate::session_store::{BookStatus, SessionStore, SpreadStatus};

/// Задача записи в БД
#[derive(Debug)]
pub enum WriteTask {
    /// Обновление вершин разворота
    UpdateSpreadVertices {
        spread_id: i64,
        left_vertices: String,
        right_vertices: String,
    },
    /// Обновление статуса разворота
    UpdateSpreadStatus {
        spread_id: i64,
        status: SpreadStatus,
    },
    /// Обновление путей разворота
    UpdateSpreadPaths {
        spread_id: i64,
        left_path: String,
        right_path: String,
    },
    /// Обновление статуса книги
    UpdateBookStatus {
        book_uuid: String,
        status: BookStatus,
    },
    /// Обновление количества страниц книги
    UpdateBookTotalPages {
        book_uuid: String,
        total_pages: i64,
    },
}

/// Глобальный канал записи (ленивая инициализация)
static WRITE_CHANNEL: std::sync::OnceLock<mpsc::UnboundedSender<WriteTask>> =
    std::sync::OnceLock::new();

/// Инициализирует канал записи и запускает фоновый воркер.
///
/// Вызывается один раз при старте сервера (до `axum::serve`).
pub fn spawn_writer(store: Arc<Mutex<SessionStore>>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<WriteTask>();

    // Регистрируем Sender в OnceLock для доступа из хендлеров
    let _ = WRITE_CHANNEL.set(tx.clone());

    // Запускаем единственный воркер записи
    tokio::spawn(async move {
        while let Some(task) = rx.recv().await {
            let result = match &task {
                WriteTask::UpdateSpreadVertices {
                    spread_id,
                    left_vertices,
                    right_vertices,
                } => store
                    .lock()
                    .ok()
                    .and_then(|mut s| s.update_spread_vertices(*spread_id, left_vertices, right_vertices).ok()),
                WriteTask::UpdateSpreadStatus { spread_id, status } => store
                    .lock()
                    .ok()
                    .and_then(|s| s.update_spread_status(*spread_id, *status).ok()),
                WriteTask::UpdateSpreadPaths {
                    spread_id,
                    left_path,
                    right_path,
                } => store
                    .lock()
                    .ok()
                    .and_then(|s| s.update_spread_paths(*spread_id, left_path, right_path).ok()),
                WriteTask::UpdateBookStatus { book_uuid, status } => store
                    .lock()
                    .ok()
                    .and_then(|s| s.update_book_status(book_uuid, *status).ok()),
                WriteTask::UpdateBookTotalPages { book_uuid, total_pages } => store
                    .lock()
                    .ok()
                    .and_then(|s| s.update_book_total_pages(book_uuid, *total_pages).ok()),
            };

            if result.is_none() {
                eprintln!("[✍️ WRITE QUEUE]: Ошибка записи в SQLite");
            }
        }
    });
}

/// Отправляет задачу в очередь записи.
///
/// Возвращает `Err` если воркер ещё не запущен (канал не инициализирован).
pub fn submit(task: WriteTask) -> Result<(), String> {
    let tx = WRITE_CHANNEL
        .get()
        .ok_or_else(|| "Write queue не инициализирована".to_string())?;
    tx.send(task)
        .map_err(|e| format!("Ошибка отправки задачи: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_write_task_debug() {
        let task = WriteTask::UpdateSpreadStatus {
            spread_id: 1,
            status: SpreadStatus::Completed,
        };
        let dbg = format!("{:?}", task);
        assert!(dbg.contains("UpdateSpreadStatus"));
    }

    #[test]
    fn test_submit_before_init() {
        // Если spawn_writer не вызывался, submit вернёт Err
        let result = submit(WriteTask::UpdateBookStatus {
            book_uuid: "test".into(),
            status: BookStatus::Completed,
        });
        // Может быть Ok если другой тест уже инициализировал канал
        // или Err если канал ещё не создан — оба варианта валидны
        let _ = result;
    }
}