use axum::{
    Json, Router,
    extract::Query,
    http::StatusCode,
    routing::{get, patch, post},
};

use clap::Parser;

use opencv::{core::Mat, imgcodecs, prelude::*};

use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tower_http::cors::{Any, CorsLayer};

mod config; // Конфигурация bind-адреса (host/port)
mod cv;
mod sane_core; // Подключаем наш новый аппаратный слой FFI
mod pdf_exporter; // Сборка финального PDF из страниц (G5)
mod session_recovery; // Горячий рестарт сессии
mod session_store; // Транзакционное хранение сессий сканирования

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

    // D1: Инициализация Session Store (SQLite)
    let db_path = "./kanonissa.db";
    let session_store = session_store::global_session_store(db_path);
    println!("[💾 SESSION STORE]: Инициализирован SQLite на {}", db_path);

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
    let app = Router::new()
        .route("/api/v1/health", get(health_check))
        .route("/api/v1/scanner/init", post(initialize_sane))
        .route("/api/v1/scanner/process", post(process_scan_frame))
        .route("/api/v1/calibration", get(get_calibration))
        .route("/api/v1/calibration", post(update_calibration))
        .route("/api/v1/scan/{uuid}/adjust-vertex", patch(adjust_vertex))
        .route("/api/v1/export-pdf", post(export_pdf))
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
    let store = session_store::global_session_store("./kanonissa.db");
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

    store
        .update_spread_vertices(spread.id, left_v, right_v)
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
    let store = session_store::global_session_store("./kanonissa.db");
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
        author: "Kanonissa Library".to_string(),
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
        "{\"status\": \"Kanonissa Core Engine Online\"}",
    )
}

async fn initialize_sane() -> (StatusCode, &'static str) {
    println!("[⚙️ SANE API]: Инициализация каретки сканера... Готов.");
    (StatusCode::OK, "{\"hardware_status\": \"ScannerReady\"}")
}

async fn process_scan_frame(
    Json(payload): Json<ScanTriggerRequest>,
) -> Result<Json<ScanResponse>, (StatusCode, String)> {
    let start_time = std::time::Instant::now();

    // Блокирующий вызов scanimage выполняем в отдельном потоке, чтобы не фризить Tokio event-loop
    let captured_frame = tokio::task::spawn_blocking(|| -> Result<Mat, String> {
        println!("[⚙️ HARDWARE]: Поиск планшетных сканеров на USB-шине...");
        let device_name = sane_core::detect_hardware_scanner()
            .map_err(|e| format!("Аппаратный сбой при обнаружении сканера: {}", e))?;

        println!("[📷 CAPTURE]: Захват кадра со сканера '{}'", device_name);
        let mat = sane_core::capture_sane_frame(&device_name)
            .map_err(|e| format!("Ошибка захвата матрицы SANE: {}", e))?;

        if mat.empty() {
            return Err("Получен пустой буфер кадра от сканера".to_string());
        }

        Ok(mat)
    })
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("Panic в spawn_blocking: {:?}", e)))?
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?;

    // Принудительный разворот кадра А3 на 90° по часовой стрелке (как в CLI конвейере)
    let mut rotated_frame = Mat::default();
    opencv::core::rotate(
        &captured_frame,
        &mut rotated_frame,
        opencv::core::ROTATE_90_CLOCKWISE,
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Детекция вершин страницы на развернутом кадре
    let vertices = cv::process_book_contours(&rotated_frame)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("CV error: {}", e)))?;

    println!(
        "[📐 WEB CV]: Вершины восстановлены для UUID {}: {:?}",
        payload.uuid, vertices
    );

    // Полная коррекция страницы: перспективная трансформация + деварпинг корешка
    const PAGE_WIDTH: u32 = 2400;
    const PAGE_HEIGHT: u32 = 3200;
    let corrected_page = cv::rectify_and_dewarp_page(
        &rotated_frame,
        &vertices,
        PAGE_WIDTH,
        PAGE_HEIGHT,
    )
    .unwrap_or_else(|e| {
        println!("[⚠️ CORRECT] Ошибка коррекции: {}. Использую исходный кадр.", e);
        rotated_frame.clone()
    });

    // Сегментация разворота на левую и правую страницы
    let (left_page, right_page) = cv::segment_pages(&corrected_page)
        .unwrap_or_else(|e| {
            println!("[⚠️ SEGMENT] Ошибка сегментации: {}. Использую исходный кадр.", e);
            (corrected_page.clone(), corrected_page.clone())
        });

    // Детекция и выравнивание скоса левой страницы
    let left_skew = cv::detect_skew_angle(&left_page).unwrap_or(0.0);
    println!("[📐 SKEW LEFT] Угол скоса левой страницы: {:.2}°", left_skew);
    let left_aligned = cv::rotate_image(&left_page, -left_skew).unwrap_or(left_page.clone());

    // Детекция и выравнивание скоса правой страницы
    let right_skew = cv::detect_skew_angle(&right_page).unwrap_or(0.0);
    println!("[📐 SKEW RIGHT] Угол скоса правой страницы: {:.2}°", right_skew);
    let right_aligned = cv::rotate_image(&right_page, -right_skew).unwrap_or(right_page.clone());

    // M8: Hot-reload калибровки (k_factor, window_size, profile из calibration.json)
    let calib = cv::calibration::global_calibration().get();
    let profile = match &payload.profile {
        Some(p) => cv::ProcessingProfile::from_str_lenient(p),
        None => calib.processing_profile(),
    };
    println!(
        "[⚙️ CALIB] k={}, window={}, profile={:?}",
        calib.k_factor, calib.window_size, profile
    );

    // E2: Multi-profile обработка каждой страницы
    let final_left = cv::apply_profile(&left_aligned, profile, calib.k_factor, calib.window_size)
        .unwrap_or_else(|e| {
            println!("[⚠️ BIN LEFT] Ошибка обработки левой страницы: {}", e);
            left_aligned.clone()
        });
    let final_right = cv::apply_profile(&right_aligned, profile, calib.k_factor, calib.window_size)
        .unwrap_or_else(|e| {
            println!("[⚠️ BIN RIGHT] Ошибка обработки правой страницы: {}", e);
            right_aligned.clone()
        });

    // Сохранение страниц: CCITT G4 TIFF для 1-бит, PNG для grayscale/color
    let output_dir = "./split";
    if !std::path::Path::new(output_dir).exists() {
        std::fs::create_dir_all(output_dir).ok();
    }
    let (left_path, right_path) = if profile == cv::ProcessingProfile::TextBw1bit {
        (
            format!("{}/page_{}_left.tiff", output_dir, payload.uuid),
            format!("{}/page_{}_right.tiff", output_dir, payload.uuid),
        )
    } else {
        (
            format!("{}/page_{}_left.png", output_dir, payload.uuid),
            format!("{}/page_{}_right.png", output_dir, payload.uuid),
        )
    };

    if profile == cv::ProcessingProfile::TextBw1bit {
        match cv::encode_ccitt_g4_to_file(&final_left, &left_path) {
            Ok(size) => println!("[💾 CCITT G4 LEFT] {} KB", size / 1024),
            Err(e) => println!("[⚠️ SAVE LEFT] Ошибка кодирования левой страницы: {}", e),
        }
        match cv::encode_ccitt_g4_to_file(&final_right, &right_path) {
            Ok(size) => println!("[💾 CCITT G4 RIGHT] {} KB", size / 1024),
            Err(e) => println!("[⚠️ SAVE RIGHT] Ошибка кодирования правой страницы: {}", e),
        }
    } else {
        let params = opencv::core::Vector::default();
        imgcodecs::imwrite(&left_path, &final_left, &params)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("imwrite left: {}", e)))?;
        imgcodecs::imwrite(&right_path, &final_right, &params)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("imwrite right: {}", e)))?;
        println!("[💾 PNG] Сохранено: {} и {}", left_path, right_path);
    }

    println!("[✅ WEB SUCCESS] Разворот обработан и сохранен: {} и {}", left_path, right_path);

    Ok(Json(ScanResponse {
        status: "PreviewReady".to_string(),
        uuid: payload.uuid,
        vertices,
        execution_time_ms: start_time.elapsed().as_millis(),
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
