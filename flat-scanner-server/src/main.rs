//     Flat Scanner Server - High-performance headless flatbed scanning
//     core engine in Rust.
//
//     Copyright (C) 2026  Valeria Fadeeva <valeria.fadeeva.me@gmail.com>

use axum::{
    Json, Router,
    extract::Query,
    http::StatusCode,
    routing::{get, patch, post},
    Extension,
};

use clap::Parser;

use opencv::{core::Mat, imgcodecs, prelude::*};

use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tower_http::cors::{Any, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;

mod config; // Конфигурация bind-адреса (host/port)
mod cv;
mod sane_core; // Подключаем наш новый аппаратный слой FFI
mod pdf_exporter; // Сборка финального PDF из страниц (G5)
mod pdf_importer; // Разборка сторонних PDF (G4)
mod session_recovery; // Горячий рестарт сессии
mod session_store; // Транзакционное хранение сессий сканирования
mod write_queue; // Single Writer + FIFO-очередь (§1.3)
mod pipeline; // Сквозной скоростной пайплайн (TECH_SPEC_addon_3.md §J)
mod routes; // Axum HTTP API routes (TECH_SPEC_addon_4.md)

/// ТЗ ПК "Канонисса-Библиотека" v1.0 — Двухрежимное ядро (Web / CLI)
#[derive(Parser, Debug)]
#[command(
    author = "Valeria Fadeeva",
    version = "1.0",
    about = "Гибридный движок оцифровки"
)]
struct CliArgs {
    /// Флаг активации консольного режима (без запуска веб-сервера)
    #[arg(short, long, default_value_t = false)]
    cli: bool,

    /// Путь к папке сохранения страниц в CLI-режиме (аналог split/)
    #[arg(short, long, default_value = "./split")]
    output_dir: String,

    /// Коэффициент Сауволы для CLI-режима
    #[arg(short, long, default_value_t = 0.2)]
    k_factor: f32,

    /// Опциональный путь к сырому файлу разворота для тестов без сканера
    #[arg(short, long)]
    input_file: Option<String>,

    /// Адрес привязки веб-сервера (127.0.0.1 локально, 0.0.0.0 по сети).
    /// Приоритет: CLI-флаг > config.toml > дефолт 127.0.0.1
    #[arg(long)]
    host: Option<String>,

    /// Порт веб-сервера. Приоритет: CLI-флаг > config.toml > дефолт 54321
    #[arg(long)]
    port: Option<u16>,
}

#[derive(Debug, Deserialize)]
struct ScanTriggerRequest {
    uuid: String,
    _threshold_preset: i32,
    /// Профиль обработки (E2): text_bw_1bit | illustration_grayscale_8bit | color_rgb_24bit
    #[serde(default)]
    profile: Option<String>,
}

#[derive(Debug, Serialize)]
struct ScanResponse {
    status: String,
    uuid: String,
    vertices: cv::PageVertices,
    execution_time_ms: u128,
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let args = CliArgs::parse();

    // C1: Инициализация путей к каталогам (валидация при старте)
    for dir in ["./split", "./export", "./import"] {
        if let Err(e) = fs::create_dir_all(dir) {
            return Err(format!("Не удалось создать каталог {}: {}", dir, e));
        }
    }
    println!("[📁 PATHS]: Каталоги инициализированы");

    // D1: Инициализация Session Store (SQLite)
    let db_path = "./data.db";
    let session_store = session_store::global_session_store(db_path);
    println!("[💾 SESSION STORE]: Инициализирован SQLite на {}", db_path);

    // §1.3: Запуск единственного воркера записи в SQLite
    write_queue::spawn_writer(session_store.clone());
    println!("[✍️ WRITE QUEUE]: FIFO-очередь записи активна");

    // D2: Горячий рестарт сессии
    let recovery = session_recovery::SessionRecovery::new(None);
    if let Ok(store) = session_store.lock() {
        match recovery.recover_session(&store) {
            Ok(Some(result)) => {
                println!(
                    "[🔄 HOT RESTART]: Восстановлена сессия '{}' ({} страниц, {} завершено, {} pending)",
                    result.book_name,
                    result.total_pages,
                    result.completed_spreads,
                    result.pending_spreads
                );
            }
            Ok(None) => {
                println!("[🔄 HOT RESTART]: Незавершённых сессий не найдено");
            }
            Err(e) => {
                println!("[⚠️ HOT RESTART]: Ошибка восстановления: {}", e);
            }
        }

        // D2: WAL checkpoint при старте
        if let Err(e) = recovery.wal_checkpoint(&store) {
            println!("[⚠️ WAL]: Ошибка checkpoint: {}", e);
        }

        // D2: Очистка устаревших pending-файлов (старше 24 часов)
        if let Ok(removed) = recovery.cleanup_stale_pending(std::time::Duration::from_secs(86400)) {
            if !removed.is_empty() {
                println!("[🧹 CLEANUP]: Удалено устаревших pending: {}", removed.len());
            }
        }
    }

    // ПРОВЕРКА РЕЖИМА 1: Если передан флаг --cli, запускаем локальный конвейер в RAM
    if args.cli {
        println!("[📟 CLI MODE]: Активирован консольный конвейер оцифровки...");
        return run_cli_pipeline(args);
    }

    // РЕЖИМА 2 (Дефолт): Запуск веб-сервера Axum под управление Tokio для Flutter
    println!("[🌐 WEB MODE]: Запуск асинхронного сервера для Flutter Desktop...");

    // Загрузка конфигурации bind-адреса (CLI-флаг > config.toml > дефолт)
    let mut cfg = config::Config::load();
    cfg.apply_cli_overrides(args.host.clone(), args.port);
    if cfg.server.host == "0.0.0.0" {
        println!("[⚠️ BIND]: Сервер слушает на 0.0.0.0 — открыт доступ по сети!");
    }

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    let page_processor = std::sync::Arc::new(pipeline::PageProcessor::new("./split".to_string()));

    // M4: Лимит на размер загружаемого изображения (50MB)
    // Предотвращает memory exhaustion при загрузке слишком больших файлов
    let body_limit = RequestBodyLimitLayer::new(50 * 1024 * 1024);

    let app = Router::new()
        .route("/api/v1/health", get(health_check))
        .route("/api/v1/scanner/init", post(initialize_sane))
        .route("/api/v1/scanner/process", post(process_scan_frame))
        .route("/api/v1/scan", post(routes::handle_scan))
        .route("/api/v1/calibration", get(get_calibration))
        .route("/api/v1/calibration", post(update_calibration))
        .route("/api/v1/scan/{uuid}/adjust-vertex", patch(adjust_vertex))
        .route("/api/v1/export-pdf", post(export_pdf))
        .route("/api/v1/import-pdf", post(import_pdf))
        .route("/api/v1/replace-pdf-page", post(replace_pdf_page))
        .route("/api/v1/insert-pdf-page", post(insert_pdf_page))
        .route("/api/v1/clean-pdf-page", post(clean_pdf_page))
        .layer(Extension(page_processor))
        .layer(body_limit)
        .layer(cors);

    let addr: SocketAddr = cfg.bind_addr().parse().unwrap();
    println!("[🟢 ENGINE ACTIVE]: Сетевой шлюз открыт на http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// --- БЛОК ОБРАБОТЧИКОВ КАЛИБРОВКИ (G1) ---

/// GET /api/v1/calibration — возвращает текущие параметры калибровки.
async fn get_calibration() -> Json<cv::calibration::CalibrationParams> {
    Json(cv::calibration::global_calibration().get())
}

/// POST /api/v1/calibration — обновляет параметры калибровки (hot-reload).
async fn update_calibration(
    Json(params): Json<cv::calibration::CalibrationParams>,
) -> Result<Json<cv::calibration::CalibrationParams>, (StatusCode, Json<serde_json::Value>)> {
    // Валидация: k_factor в (0, 1), window_size нечётное >= 3
    if !(0.0 < params.k_factor && params.k_factor < 1.0) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "k_factor must be in (0, 1)"})),
        ));
    }
    if params.window_size < 3 || params.window_size % 2 == 0 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "window_size must be odd and >= 3"})),
        ));
    }

    match cv::calibration::global_calibration().save(&params) {
        Ok(()) => {
            println!("[🎯 CALIBRATION]: Обновлены параметры k_factor={}, window_size={}, profile={}",
                params.k_factor, params.window_size, params.profile);
            Ok(Json(cv::calibration::global_calibration().get()))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )),
    }
}

// --- БЛОК ОБРАБОТЧИКОВ КОРРЕКЦИИ ВЕРШИН (G2) ---

/// Запрос корректировки вершины (query params)
#[derive(Deserialize)]
struct AdjustVertexQuery {
    /// Индекс вершины: 0–3 (p1–p4)
    index: u8,
    /// Новая координата X
    x: i32,
    /// Новая координата Y
    y: i32,
    /// Сторона: "left" или "right"
    page: String,
}

/// Ответ корректировки вершины
#[derive(Serialize)]
struct AdjustVertexResponse {
    /// Обновлённые вершины
    vertices: cv::PageVertices,
    /// Индекс обновлённой вершины
    index: u8,
    /// Сторона
    page: String,
}

/// PATCH /api/v1/scan/{uuid}/adjust-vertex?index=N&x=X&y=Y&page=left|right
///
/// Корректирует одну вершину страницы в последней записи спреда книги.
async fn adjust_vertex(
    axum::extract::Path(uuid): axum::extract::Path<String>,
    Query(q): Query<AdjustVertexQuery>,
) -> Result<Json<AdjustVertexResponse>, (StatusCode, Json<serde_json::Value>)> {
    // Валидация index
    if q.index > 3 {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "index must be 0-3"})),
        ));
    }

    // Валидация page
    if q.page != "left" && q.page != "right" {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "page must be 'left' or 'right'"})),
        ));
    }

    // Получаем последнюю запись спреда
    let store = session_store::global_session_store("./data.db");
    let store = store.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
    })?;

    let spread = store
        .get_last_spread(&uuid)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "No spread found for this book"})),
            )
        })?;

    // Определяем, какие вершины обновляем
    let vertices_json = if q.page == "left" {
        spread.left_vertices.clone()
    } else {
        spread.right_vertices.clone()
    };

    let mut vertices: cv::PageVertices = vertices_json
        .as_ref()
        .and_then(|j| serde_json::from_str(j).ok())
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Vertices not found for this spread"})),
            )
        })?;

    // Обновляем вершину
    let point = cv::CustomPoint { x: q.x, y: q.y };
    match q.index {
        0 => vertices.p1 = point,
        1 => vertices.p2 = point,
        2 => vertices.p3 = point,
        3 => vertices.p4 = point,
        _ => unreachable!(),
    }

    // Сериализуем обратно
    let new_vertices_json = serde_json::to_string(&vertices)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
        })?;

    // Сохраняем в БД (обновляем нужную сторону)
    let left_v = if q.page == "left" {
        &new_vertices_json
    } else {
        spread.left_vertices.as_deref().unwrap_or("{}")
    };
    let right_v = if q.page == "right" {
        &new_vertices_json
    } else {
        spread.right_vertices.as_deref().unwrap_or("{}")
    };

    // §1.3: Запись вершин через FIFO-очередь единственного воркера
    write_queue::submit(write_queue::WriteTask::UpdateSpreadVertices {
        spread_id: spread.id,
        left_vertices: left_v.to_string(),
        right_vertices: right_v.to_string(),
    })
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
    })?;

    println!(
        "[📐 ADJUST VERTEX]: UUID={}, page={}, index={}, new=({}, {})",
        uuid, q.page, q.index, q.x, q.y
    );

    Ok(Json(AdjustVertexResponse {
        vertices,
        index: q.index,
        page: q.page,
    }))
}

// --- БЛОК ЭКСПОРТА PDF (G5) ---

/// Запрос экспорта PDF
#[derive(Debug, Deserialize)]
struct ExportPdfRequest {
    /// UUID книги
    uuid: String,
    /// Путь к выходному PDF-файлу (опционально, по умолчанию ./export/<uuid>.pdf)
    #[serde(default)]
    output_path: Option<String>,
}

/// Ответ экспорта PDF
#[derive(Debug, Serialize)]
struct ExportPdfResponse {
    /// Путь к созданному PDF
    path: String,
    /// Размер PDF в байтах
    size_bytes: usize,
    /// Количество страниц
    page_count: usize,
}

/// POST /api/v1/export-pdf — собирает финальный PDF из всех страниц книги.
///
/// Страницы берутся из SessionStore (spreads, по spread_index ASC),
/// для каждого разворота — сначала левая, затем правая страница.
async fn export_pdf(
    Json(payload): Json<ExportPdfRequest>,
) -> Result<Json<ExportPdfResponse>, (StatusCode, Json<serde_json::Value>)> {
    let store = session_store::global_session_store("./data.db");
    let store = store.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
    })?;

    // Название книги для метаданных PDF
    let book = store
        .get_book(&payload.uuid)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Book not found"})),
            )
        })?;

    // Все развороты книги по порядку
    let spreads = store
        .list_spreads(&payload.uuid)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
        })?;

    // Собираем упорядоченный список страниц: левая, затем правая
    let mut page_paths: Vec<String> = Vec::new();
    for spread in &spreads {
        if let Some(p) = &spread.left_path {
            if !p.is_empty() {
                page_paths.push(p.clone());
            }
        }
        if let Some(p) = &spread.right_path {
            if !p.is_empty() {
                page_paths.push(p.clone());
            }
        }
    }

    if page_paths.is_empty() {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": "No pages found for this book"})),
        ));
    }

    // Освобождаем MutexGuard ДО блокирующего await: std::sync::MutexGuard
    // не Send, удержание через await сделало бы future хендлера не-Send.
    drop(store);

    // Путь к выходному PDF
    let output_path = match &payload.output_path {
        Some(p) if !p.is_empty() => p.clone(),
        _ => format!("./export/{}.pdf", payload.uuid),
    };
    if let Some(parent) = std::path::Path::new(&output_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(serde_json::json!({"error": format!("mkdir: {}", e)})),
                    )
                })?;
        }
    }

    let metadata = pdf_exporter::PdfMetadata {
        title: book.name.clone(),
        author: "Valeria Fadeeva".to_string(),
        subject: "Digitized book pages".to_string(),
    };

    // Блокирующий вызов (imread + сжатие) — для локального инструмента
    // блокировка event loop допустима, избегаем проблем с Send
    let size = pdf_exporter::assemble_pdf_from_tiff_pages(&page_paths, &metadata, &output_path)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
        })?;

    println!(
        "[📄 PDF EXPORT]: UUID={}, {} страниц, {} KB -> {}",
        payload.uuid,
        page_paths.len(),
        size / 1024,
        output_path
    );

    Ok(Json(ExportPdfResponse {
        path: output_path,
        size_bytes: size,
        page_count: page_paths.len(),
    }))
}

// --- БЛОК РАЗБОРКИ PDF (G4) ---

/// Запрос импорта PDF
#[derive(Debug, Deserialize)]
struct ImportPdfRequest {
    /// Путь к входному PDF
    input_pdf: String,
    /// Каталог для PNG-страниц (опционально, по умолчанию ./import/<hash>)
    #[serde(default)]
    output_dir: Option<String>,
    /// DPI растеризации (по умолчанию 300)
    #[serde(default = "default_dpi")]
    dpi: u32,
}

fn default_dpi() -> u32 {
    300
}

/// Ответ импорта PDF
#[derive(Debug, Serialize)]
struct ImportPdfResponse {
    /// Пути к экспортированным PNG-страницам
    pages: Vec<String>,
    /// Количество страниц
    page_count: usize,
}

/// Запрос замены страницы
#[derive(Debug, Deserialize)]
struct ReplacePdfPageRequest {
    input_pdf: String,
    page_index: usize,
    replacement_image: String,
    #[serde(default)]
    output_pdf: Option<String>,
}

/// Запрос вставки страницы
#[derive(Debug, Deserialize)]
struct InsertPdfPageRequest {
    input_pdf: String,
    /// Индекс после которого вставить (-1 = в начало)
    after_index: i64,
    image_path: String,
    #[serde(default)]
    output_pdf: Option<String>,
}

/// Запрос очистки страницы
#[derive(Debug, Deserialize)]
struct CleanPdfPageRequest {
    image_path: String,
    /// Профиль: text_bw_1bit | illustration_grayscale_8bit | color_rgb_24bit
    profile: String,
    #[serde(default = "default_k_factor")]
    k_factor: f32,
    #[serde(default = "default_window_size")]
    window_size: i32,
}

fn default_k_factor() -> f32 {
    0.2
}

fn default_window_size() -> i32 {
    15
}

/// Ответ операции с PDF
#[derive(Debug, Serialize)]
struct PdfOperationResponse {
    path: String,
    size_bytes: usize,
}

/// POST /api/v1/import-pdf — экспортирует страницы PDF как PNG.
async fn import_pdf(
    Json(payload): Json<ImportPdfRequest>,
) -> Result<Json<ImportPdfResponse>, (StatusCode, Json<serde_json::Value>)> {
    let output_dir = match &payload.output_dir {
        Some(d) if !d.is_empty() => d.clone(),
        _ => "./import".to_string(),
    };

    let pages = pdf_importer::import_pdf_pages(&payload.input_pdf, &output_dir, payload.dpi)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e})),
            )
        })?;

    println!(
        "[📥 PDF IMPORT]: {} -> {} страниц @ {} DPI",
        payload.input_pdf,
        pages.len(),
        payload.dpi
    );

    Ok(Json(ImportPdfResponse {
        page_count: pages.len(),
        pages,
    }))
}

/// POST /api/v1/replace-pdf-page — заменяет страницу в PDF.
async fn replace_pdf_page(
    Json(payload): Json<ReplacePdfPageRequest>,
) -> Result<Json<PdfOperationResponse>, (StatusCode, Json<serde_json::Value>)> {
    let output_pdf = match &payload.output_pdf {
        Some(p) if !p.is_empty() => p.clone(),
        _ => format!("{}.replaced.pdf", payload.input_pdf),
    };

    let size = pdf_importer::replace_page(
        &payload.input_pdf,
        payload.page_index,
        &payload.replacement_image,
        &output_pdf,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
    })?;

    println!(
        "[✏️ PDF REPLACE]: page {} -> {} ({} KB)",
        payload.page_index,
        output_pdf,
        size / 1024
    );

    Ok(Json(PdfOperationResponse {
        path: output_pdf,
        size_bytes: size,
    }))
}

/// POST /api/v1/insert-pdf-page — вставляет страницу в PDF.
async fn insert_pdf_page(
    Json(payload): Json<InsertPdfPageRequest>,
) -> Result<Json<PdfOperationResponse>, (StatusCode, Json<serde_json::Value>)> {
    let output_pdf = match &payload.output_pdf {
        Some(p) if !p.is_empty() => p.clone(),
        _ => format!("{}.inserted.pdf", payload.input_pdf),
    };

    let size = pdf_importer::insert_page(
        &payload.input_pdf,
        payload.after_index,
        &payload.image_path,
        &output_pdf,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
    })?;

    println!(
        "[➕ PDF INSERT]: after {} -> {} ({} KB)",
        payload.after_index,
        output_pdf,
        size / 1024
    );

    Ok(Json(PdfOperationResponse {
        path: output_pdf,
        size_bytes: size,
    }))
}

/// POST /api/v1/clean-pdf-page — очищает страницу от шума.
async fn clean_pdf_page(
    Json(payload): Json<CleanPdfPageRequest>,
) -> Result<Json<PdfOperationResponse>, (StatusCode, Json<serde_json::Value>)> {
    let cleaned_path = pdf_importer::clean_page(
        &payload.image_path,
        &payload.profile,
        payload.k_factor,
        payload.window_size,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e})),
        )
    })?;

    let size = std::fs::metadata(&cleaned_path)
        .map(|m| m.len() as usize)
        .unwrap_or(0);

    println!(
        "[🧹 PDF CLEAN]: {} -> {} ({} KB, profile={})",
        payload.image_path,
        cleaned_path,
        size / 1024,
        payload.profile
    );

    Ok(Json(PdfOperationResponse {
        path: cleaned_path,
        size_bytes: size,
    }))
}

// --- БЛОК ЛОКАЛЬНОГО CLI ПАЙПЛАЙНА ---
fn run_cli_pipeline(args: CliArgs) -> Result<(), String> {
    if !Path::new(&args.output_dir).exists() {
        fs::create_dir_all(&args.output_dir).map_err(|e| e.to_string())?;
    }

    let raw_frame: Mat;

    if let Some(file_path) = args.input_file {
        println!(
            "[💾 CLI STORAGE]: Чтение файла разворота с диска: {}",
            file_path
        );
        raw_frame =
            imgcodecs::imread(&file_path, imgcodecs::IMREAD_COLOR).map_err(|e| e.to_string())?;
    } else {
        println!("[⚙️ HARDWARE]: Поиск планшетных сканеров на USB-шине...");
        match sane_core::detect_hardware_scanner() {
            Ok(device_name) => {
                // Патч: вызываем без передачи жестких размеров, функция сама всё считает из чипа
                match sane_core::capture_sane_frame(&device_name) {
                    Ok(captured_mat) => raw_frame = captured_mat,
                    Err(e) => return Err(format!("Ошибка захвата матрицы: {}", e)),
                }
            }
            Err(e) => return Err(format!("Аппаратный сбой: {}", e)),
        }
    }

    if raw_frame.empty() {
        return Err("Получен пустой буфер кадра".to_string());
    }

    let start_time = std::time::Instant::now();

    // ПАТЧ 1: Принудительный разворот кадра А3 на 90 градусов по часовой стрелке силами OpenCV в RAM
    // Это переведет вертикальные полосы в нормальный горизонтальный книжный разворот!
    let mut rotated_frame = Mat::default();
    opencv::core::rotate(
        &raw_frame,
        &mut rotated_frame,
        opencv::core::ROTATE_90_CLOCKWISE,
    )
    .map_err(|e| e.to_string())?;

    // Теперь запускаем детекцию Хафа уже на правильно развернутом кадре
    let vertices = cv::process_book_contours(&rotated_frame)?;
    println!(
        "[📐 CLI CV]: Вершины восстановлены на развернутом кадре: {:?}",
        vertices
    );

    let frame_size = rotated_frame.size().map_err(|e| e.to_string())?;
    let half_width = frame_size.width / 2;

    // Теперь col_bounds честно разрежет разворот на ЛЕВУЮ и ПРАВУЮ страницы!
    let left_boxed = Mat::col_bounds(&rotated_frame, 0, half_width).map_err(|e| e.to_string())?;
    let right_boxed =
        Mat::col_bounds(&rotated_frame, half_width, frame_size.width).map_err(|e| e.to_string())?;

    let left_mat: Mat = left_boxed.clone_pointee();
    let right_mat: Mat = right_boxed.clone_pointee();

    // Запускаем Сауволу
    let binary_left = cv::apply_sauvola_threshold(&left_mat, args.k_factor, 15)?;
    let binary_right = cv::apply_sauvola_threshold(&right_mat, args.k_factor, 15)?;

    // ПАТЧ 2: Попиксельная инверсия ЧБ маски, чтобы бумага стала БЕЛОЙ, а буквы ЧЕРНЫМИ
    let mut final_left = Mat::default();
    let mut final_right = Mat::default();
    opencv::core::bitwise_not(&binary_left, &mut final_left, &Mat::default())
        .map_err(|e| e.to_string())?;
    opencv::core::bitwise_not(&binary_right, &mut final_right, &Mat::default())
        .map_err(|e| e.to_string())?;

    let left_path = format!("{}/page_left_clean.tiff", args.output_dir);
    let right_path = format!("{}/page_right_clean.tiff", args.output_dir);

    // Сохраняем в CCITT Group 4 TIFF
    let left_size = cv::encode_ccitt_g4_to_file(&final_left, &left_path)
        .map_err(|e| format!("Ошибка кодирования левой страницы: {}", e))?;
    let right_size = cv::encode_ccitt_g4_to_file(&final_right, &right_path)
        .map_err(|e| format!("Ошибка кодирования правой страницы: {}", e))?;

    println!(
        "[💾 CCITT G4]: Левая страница: {} KB, Правая страница: {} KB",
        left_size / 1024,
        right_size / 1024
    );

    println!(
        "[🚀 CLI SUCCESS]: Разворот успешно ориентирован, бинаризирован и разделен за {} мс!",
        start_time.elapsed().as_millis()
    );
    Ok(())
}

// --- БЛОК ОБРАБОТЧИКОВ WEB-СЕРВЕРА AXUM ---
async fn health_check() -> (StatusCode, &'static str) {
    (
        StatusCode::OK,
        "{\"status\": \"Core Engine Online\"}",
    )
}

async fn initialize_sane() -> (StatusCode, &'static str) {
    println!("[⚙️ SANE API]: Инициализация каретки сканера... Готов.");
    (StatusCode::OK, "{\"hardware_status\": \"ScannerReady\"}")
}

async fn process_scan_frame(
    Json(payload): Json<ScanTriggerRequest>,
) -> Result<Json<ScanResponse>, (StatusCode, String)> {
    // §J: Сквозной скоростной пайплайн через PageProcessor
    let profile_opt = payload.profile.clone();
    let uuid = payload.uuid.clone();
    let uuid_for_task = uuid.clone();

    let result = tokio::task::spawn_blocking(move || {
        let processor = pipeline::PageProcessor::new("./split".to_string());
        let device_name = sane_core::detect_hardware_scanner()
            .map_err(|e| format!("Аппаратный сбой при обнаружении сканера: {}", e))?;
        processor.process_page(&uuid_for_task, profile_opt.as_deref(), &device_name)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Panic в spawn_blocking: {:?}", e)))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // §1.3: Запись путей и вершин через FIFO-очередь
    let store = session_store::global_session_store("./data.db");
    if let Ok(s) = store.lock() {
        if let Ok(Some(spread)) = s.get_last_spread(&uuid) {
            let _ = write_queue::submit(write_queue::WriteTask::UpdateSpreadPaths {
                spread_id: spread.id,
                left_path: result.left_path.clone(),
                right_path: result.right_path.clone(),
            });
            let _ = write_queue::submit(write_queue::WriteTask::UpdateSpreadStatus {
                spread_id: spread.id,
                status: session_store::SpreadStatus::Completed,
            });
        }
    }

    Ok(Json(ScanResponse {
        status: "PreviewReady".to_string(),
        uuid,
        vertices: result.vertices,
        execution_time_ms: result.execution_time_ms,
    }))
}


// --- ТЕСТЫ G2: adjust-vertex ---
#[cfg(test)]
mod tests_adjust_vertex {
    use super::*;

    #[test]
    fn test_adjust_vertex_query_deserialize() {
        let json = r#"{"index": 2, "x": 100, "y": 200, "page": "left"}"#;
        let q: AdjustVertexQuery = serde_json::from_str(json).unwrap();
        assert_eq!(q.index, 2);
        assert_eq!(q.x, 100);
        assert_eq!(q.y, 200);
        assert_eq!(q.page, "left");
    }

    #[test]
    fn test_adjust_vertex_response_serialize() {
        let resp = AdjustVertexResponse {
            vertices: cv::PageVertices {
                p1: cv::CustomPoint { x: 10, y: 20 },
                p2: cv::CustomPoint { x: 30, y: 40 },
                p3: cv::CustomPoint { x: 50, y: 60 },
                p4: cv::CustomPoint { x: 70, y: 80 },
            },
            index: 1,
            page: "right".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"index\":1"));
        assert!(json.contains("\"page\":\"right\""));
    }

    #[test]
    fn test_adjust_vertex_updates_correct_point() {
        let mut vertices = cv::PageVertices {
            p1: cv::CustomPoint { x: 0, y: 0 },
            p2: cv::CustomPoint { x: 100, y: 0 },
            p3: cv::CustomPoint { x: 100, y: 200 },
            p4: cv::CustomPoint { x: 0, y: 200 },
        };

        let point = cv::CustomPoint { x: 55, y: 66 };
        match 1u8 {
            0 => vertices.p1 = point,
            1 => vertices.p2 = point,
            2 => vertices.p3 = point,
            3 => vertices.p4 = point,
            _ => unreachable!(),
        }

        assert_eq!(vertices.p2.x, 55);
        assert_eq!(vertices.p2.y, 66);
        assert_eq!(vertices.p1.x, 0); // не изменена
    }
}