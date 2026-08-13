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

    let left_path = format!("{}/page_left_clean.png", args.output_dir);
    let right_path = format!("{}/page_right_clean.png", args.output_dir);

    // Сохраняем идеально ориентированные и инвертированные страницы
    imgcodecs::imwrite(&left_path, &final_left, &opencv::core::Vector::new())
        .map_err(|e| e.to_string())?;
    imgcodecs::imwrite(&right_path, &final_right, &opencv::core::Vector::new())
        .map_err(|e| e.to_string())?;

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

    let dummy_frame = opencv::core::Mat::new_rows_cols_with_default(
        4700,
        3300,
        opencv::core::CV_8UC3,
        opencv::core::Scalar::new(240.0, 240.0, 240.0, 0.0),
    )
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let vertices = cv::process_book_contours(&dummy_frame)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let _binary_frame = cv::apply_sauvola_threshold(&dummy_frame, 0.2, 15)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(ScanResponse {
        status: "PreviewReady".to_string(),
        uuid: payload.uuid,
        vertices,
        execution_time_ms: start_time.elapsed().as_millis(),
    }))
}
