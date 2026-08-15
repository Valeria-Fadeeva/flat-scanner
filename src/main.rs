use axum::{
    Json, Router,
    http::StatusCode,
    routing::{get, post},
};

use clap::Parser;

use opencv::{core::Mat, imgcodecs, prelude::*};

use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use tower_http::cors::{Any, CorsLayer};

mod cv;
mod sane_core; // Подключаем наш новый аппаратный слой FFI
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

    // Проверка на незавершённую сессию (hot-restart)
    if let Ok(store) = session_store.lock() {
        if let Ok(Some(uuid)) = store.get_in_progress_book() {
            println!("[🔄 HOT RESTART]: Обнаружена незавершённая сессия: {}", uuid);
        }
    }

    // ПРОВЕРКА РЕЖИМА 1: Если передан флаг --cli, запускаем локальный конвейер в RAM
    if args.cli {
        println!("[📟 CLI MODE]: Активирован консольный конвейер оцифровки...");
        return run_cli_pipeline(args);
    }

    // РЕЖИМА 2 (Дефолт): Запуск веб-сервера Axum под управление Tokio для Flutter
    println!("[🌐 WEB MODE]: Запуск асинхронного сервера для Flutter Desktop...");
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    let app = Router::new()
        .route("/api/v1/health", get(health_check))
        .route("/api/v1/scanner/init", post(initialize_sane))
        .route("/api/v1/scanner/process", post(process_scan_frame))
        .layer(cors);

    let addr: SocketAddr = "127.0.0.1:54321".parse().unwrap();
    println!("[🟢 ENGINE ACTIVE]: Сетевой шлюз открыт на http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
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
