# TECH_SPEC_addon_4.md: Интеграция Axum HTTP API и Маршрутов Управления

## 1. Назначение документа
Документ специфицирует требования к реализации легковесного HTTP-интерфейса (Axum Web Layer) для инициализации процесса сканирования текущей страницы и передачи параметров от Flutter-клиента.

## 2. Спецификация Эндпоинта (HTTP POST `/api/v1/scan`)

### 2.1. Контракт Запроса (JSON Payload)
Фронтенд передаёт структуру идентификатора книги и номер текущей целевой страницы.

```json
{
  "book_id": "9a7b1c3d-e5f6-4a3b-8c2d-1e0f2a3b4c5d",
  "page_number": 14
}
```

### 2.2. Поведение Эндпоинта и Жизненный Цикл
1. Axum-поток принимает запрос, десериализует JSON в структуру `ScanRequest`.
2. Создаётся экземпляр безопасного Си-дескриптора `SaneScanner` (TECH_SPEC_addon_2.md). Если физический сканер занят или отключён, эндпоинт мгновенно возвращает статус `HTTP 503 Service Unavailable` с описанием ошибки в JSON, не блокируя сервер.
3. Вызывается асинхронный метод `PageProcessor::process_page` (TECH_SPEC_addon_3.md).
4. Веб-сервер возвращает `HTTP 202 Accepted` сразу, как только задача уходит в обработку, либо `HTTP 200 OK` по факту мгновенного завершения (Warp + Fast Binarization).

---

## 3. Архитектурная Реализация (Код Axum Ручки)

Модель обязана интегрировать следующий обработчик в `src/routes.rs` (или `src/main.rs`):

```rust
use axum::{Extension, Json, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct ScanRequest {
    book_id: String,
    page_number: i32,
}

/// Асинхронный обработчик команды сканирования с изоляцией паник
pub async fn handle_scan(
    Extension(processor): Extension<Arc<PageProcessor>>,
    Json(payload): Json<ScanRequest>,
) -> impl IntoResponse {
    // 1. Попытка инициализации Си-интерфейса сканера
    let scanner = match SaneScanner::new() {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({ "status": "ERROR", "message": format!("Сканер недоступен: {}", e) }))
            ).into_response();
        }
    };

    // 2. Запуск сквозного скоростного пайплайна (Захват -> Валидация -> Warp -> Броня SQLite)
    match processor.process_page(payload.book_id, payload.page_number, scanner).await {
        Ok(_) => {
            (
                StatusCode::OK,
                Json(serde_json::json!({ "status": "SUCCESS", "message": "Страница успешно обработана и закоммичена в WAL" }))
            ).into_response()
        }
        Err(digitization_err) => {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "status": "FAILED", "message": digitization_err.to_string() }))
            ).into_response()
        }
    }
}
```

## 4. Конфигурация Маршрутизатора (Router Setup)
Зарегистрировать ручку в основном роутере Axum приложения:
```rust
let app = Router::new()
    .route("/api/v1/scan", post(handle_scan))
    .layer(Extension(arc_page_processor));
```
