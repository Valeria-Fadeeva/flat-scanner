//! Сквозной скоростной пайплайн оцифровки (TECH_SPEC_addon_3.md §J).
//!
//! Инкапсулирует полный CV-конвейер: захват → rotate → вершины → rectify+dewarp
//! → сегментация → skew → profile → save. Минимальное удержание сканера через RAII.

use std::time::Instant;

use opencv::{core::Mat, imgcodecs, imgproc, prelude::*};

use crate::cv;
use crate::sane_core;

/// Быстрая бинаризация: adaptive threshold (ADAPTIVE_THRESH_MEAN_C, blockSize=11, C=2.0).
/// Используется для мгновенного предпросмотра до полного конвейера.
pub fn fast_binarize(src: &Mat) -> Result<Mat, String> {
    let mut gray = Mat::default();
    if src.channels() == 3 {
        imgproc::cvt_color(
            src,
            &mut gray,
            imgproc::COLOR_BGR2GRAY,
            0,
            opencv::core::AlgorithmHint::ALGO_HINT_DEFAULT,
        )
        .map_err(|e| format!("cvt_color: {}", e))?;
    } else {
        gray = src.clone();
    }

    let mut dst = Mat::default();
    imgproc::adaptive_threshold(
        &gray,
        &mut dst,
        255.0,
        imgproc::ADAPTIVE_THRESH_MEAN_C,
        imgproc::THRESH_BINARY,
        11,
        2.0,
    )
    .map_err(|e| format!("adaptive_threshold: {}", e))?;

    Ok(dst)
}

/// Результат обработки страницы
#[derive(Debug)]
pub struct PageResult {
    pub left_path: String,
    pub right_path: String,
    pub vertices: cv::PageVertices,
    pub execution_time_ms: u128,
}

/// Обработчик страницы: инкапсулирует полный CV-конвейер.
pub struct PageProcessor {
    output_dir: String,
}

impl PageProcessor {
    pub fn new(output_dir: String) -> Self {
        Self { output_dir }
    }

    /// Полный конвейер обработки разворота.
    /// Вызывается из `spawn_blocking` — блокирующий.
    pub fn process_page(
        &self,
        uuid: &str,
        profile_override: Option<&str>,
        device_name: &str,
    ) -> Result<PageResult, String> {
        let start_time = Instant::now();

        // 1. Захват кадра (RAII: SaneScanner освобождает сканер при выходе из scope)
        let captured_frame = sane_core::capture_sane_frame(device_name)
            .map_err(|e| format!("capture_sane_frame: {}", e))?;

        if captured_frame.empty() {
            return Err("Получен пустой буфер кадра от сканера".to_string());
        }

        // 2. Разворот кадра А3 на 90° по часовой стрелке
        let mut rotated_frame = Mat::default();
        opencv::core::rotate(
            &captured_frame,
            &mut rotated_frame,
            opencv::core::ROTATE_90_CLOCKWISE,
        )
        .map_err(|e| format!("rotate: {}", e))?;

        // 3. Детекция вершин страницы
        let vertices = cv::process_book_contours(&rotated_frame)
            .map_err(|e| format!("process_book_contours: {}", e))?;

        println!(
            "[📐 PIPELINE]: Вершины восстановлены для UUID {}: {:?}",
            uuid, vertices
        );

        // 4. Полная коррекция: перспективная трансформация + деварпинг корешка
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

        // 5. Сегментация разворота на левую и правую страницы
        let (left_page, right_page) = cv::segment_pages(&corrected_page)
            .unwrap_or_else(|e| {
                println!("[⚠️ SEGMENT] Ошибка сегментации: {}. Использую исходный кадр.", e);
                (corrected_page.clone(), corrected_page.clone())
            });

        // 6. Детекция и выравнивание скоса
        let left_skew = cv::detect_skew_angle(&left_page).unwrap_or(0.0);
        println!("[📐 SKEW LEFT] Угол скоса левой страницы: {:.2}°", left_skew);
        let left_aligned = cv::rotate_image(&left_page, -left_skew).unwrap_or(left_page.clone());

        let right_skew = cv::detect_skew_angle(&right_page).unwrap_or(0.0);
        println!("[📐 SKEW RIGHT] Угол скоса правой страницы: {:.2}°", right_skew);
        let right_aligned = cv::rotate_image(&right_page, -right_skew).unwrap_or(right_page.clone());

        // 7. Hot-reload калибровки + multi-profile обработка
        let calib = cv::calibration::global_calibration().get();
        let profile = match profile_override {
            Some(p) => cv::ProcessingProfile::from_str_lenient(p),
            None => calib.processing_profile(),
        };
        println!(
            "[⚙️ CALIB] k={}, window={}, profile={:?}",
            calib.k_factor, calib.window_size, profile
        );

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

        // 8. Сохранение страниц: CCITT G4 TIFF для 1-бит, PNG для grayscale/color
        let output_dir = &self.output_dir;
        if !std::path::Path::new(output_dir).exists() {
            std::fs::create_dir_all(output_dir).ok();
        }

        let (left_path, right_path) = if profile == cv::ProcessingProfile::TextBw1bit {
            (
                format!("{}/page_{}_left.tiff", output_dir, uuid),
                format!("{}/page_{}_right.tiff", output_dir, uuid),
            )
        } else {
            (
                format!("{}/page_{}_left.png", output_dir, uuid),
                format!("{}/page_{}_right.png", output_dir, uuid),
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
                .map_err(|e| format!("imwrite left: {}", e))?;
            imgcodecs::imwrite(&right_path, &final_right, &params)
                .map_err(|e| format!("imwrite right: {}", e))?;
            println!("[💾 PNG] Сохранено: {} и {}", left_path, right_path);
        }

        println!(
            "[✅ PIPELINE SUCCESS] Разворот обработан: {} и {}",
            left_path, right_path
        );

        Ok(PageResult {
            left_path,
            right_path,
            vertices,
            execution_time_ms: start_time.elapsed().as_millis(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fast_binarize_small_image() {
        let src = Mat::zeros(32, 32, opencv::core::CV_8UC3)
            .unwrap()
            .to_mat()
            .unwrap();
        let result = fast_binarize(&src);
        assert!(result.is_ok(), "fast_binarize не должен падать на валидном изображении");
        let dst = result.unwrap();
        assert_eq!(dst.rows(), 32);
        assert_eq!(dst.cols(), 32);
    }

    #[test]
    fn test_page_processor_new() {
        let pp = PageProcessor::new("./test_output".to_string());
        assert_eq!(pp.output_dir, "./test_output");
    }
}