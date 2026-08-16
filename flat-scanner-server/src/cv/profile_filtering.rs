//! E2: Multi-profile фильтрация страниц.
//!
//! Три профиля обработки:
//!   - `Text_BW_1bit` — Sauvola-бинаризация + инверсия (белая бумага, чёрный текст).
//!     Готово к экспорту в CCITT Group 4 TIFF через `ccitt_encoder`.
//!   - `Illustration_Grayscale_8bit` — гамма-коррекция + CLAHE-контраст для иллюстраций.
//!   - `Color_RGB_24bit` — оригинальная палитра без изменений (цветные вставки).
//!
//! Профиль передаётся из Flutter UI через Axum API (поле `profile` в запросе).

use opencv::{
    core::{self, Mat},
    imgproc,
    prelude::*,
};
use serde::{Deserialize, Serialize};

use super::binarization::apply_sauvola_threshold;
use super::seal_extraction::{extract_seal_mask, overlay_seal_on_text};

/// Профиль обработки страницы.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessingProfile {
    /// Текст: бинарный 1-бит (Sauvola + инверсия). Экспорт в CCITT G4.
    TextBw1bit,
    /// Иллюстрация: grayscale 8-бит (гамма + контраст).
    IllustrationGrayscale8bit,
    /// Цвет: RGB 24-бит, оригинальная палитра.
    ColorRgb24bit,
}

impl Default for ProcessingProfile {
    fn default() -> Self {
        ProcessingProfile::TextBw1bit
    }
}

impl ProcessingProfile {
    /// Разбор строки из API (snake_case) в профиль.
    pub fn from_str_lenient(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "text_bw_1bit" | "text" | "bw" | "1bit" => ProcessingProfile::TextBw1bit,
            "illustration_grayscale_8bit" | "illustration" | "gray" | "8bit" => {
                ProcessingProfile::IllustrationGrayscale8bit
            }
            "color_rgb_24bit" | "color" | "rgb" | "24bit" => ProcessingProfile::ColorRgb24bit,
            _ => ProcessingProfile::default(),
        }
    }
}

/// Применяет профиль обработки к изображению.
///
/// # Аргументы
/// * `src` — исходное изображение (BGR или grayscale)
/// * `profile` — профиль обработки
/// * `k_factor` — коэффициент Сауволы (для `TextBw1bit`)
/// * `window_size` — размер окна Сауволы (для `TextBw1bit`)
///
/// # Возвращает
/// Обработанное изображение:
///   - `TextBw1bit` → CV_8UC1, 0/255 (белая бумага, чёрный текст)
///   - `IllustrationGrayscale8bit` → CV_8UC1, гамма + CLAHE
///   - `ColorRgb24bit` → CV_8UC3, без изменений
pub fn apply_profile(
    src: &Mat,
    profile: ProcessingProfile,
    k_factor: f32,
    window_size: i32,
) -> Result<Mat, String> {
    match profile {
        ProcessingProfile::TextBw1bit => {
            let binary = apply_sauvola_threshold(src, k_factor, window_size)?;
            // Инверсия: бумага белая, текст чёрный
            let mut inverted = Mat::default();
            core::bitwise_not(&binary, &mut inverted, &Mat::default())
                .map_err(|e| e.to_string())?;

            // G3 (M5): сохранение печатей/штампов. Извлекаем маску
            // синей/красной печати из исходного цветного кадра и налагаем
            // её поверх текстового слоя, чтобы чернила не были стёрты
            // Sauvola-бинаризацией. Для grayscale-входа маска пустая —
            // слой возвращается без изменений.
            let seal_mask = extract_seal_mask(src)?;
            overlay_seal_on_text(&inverted, &seal_mask)
        }
        ProcessingProfile::IllustrationGrayscale8bit => {
            // Grayscale
            let mut gray = Mat::default();
            if src.channels() > 1 {
                imgproc::cvt_color(
                    src,
                    &mut gray,
                    imgproc::COLOR_BGR2GRAY,
                    0,
                    core::AlgorithmHint::ALGO_HINT_APPROX,
                )
                .map_err(|e| e.to_string())?;
            } else {
                gray = src.clone();
            }

            // Гамма-коррекция (gamma=1.2 — лёгкое осветление теней)
            let mut gamma_corrected = Mat::default();
            apply_gamma(&gray, &mut gamma_corrected, 1.2)?;

            // CLAHE: локальный контраст без выбивания фона
            let mut clahe = imgproc::create_clahe(2.0, core::Size::new(8, 8))
                .map_err(|e| e.to_string())?;
            let mut result = Mat::default();
            clahe
                .apply(&gamma_corrected, &mut result)
                .map_err(|e| e.to_string())?;
            Ok(result)
        }
        ProcessingProfile::ColorRgb24bit => {
            // Оригинальная палитра: гарантируем BGR 3-канальность
            if src.channels() == 1 {
                let mut bgr = Mat::default();
                imgproc::cvt_color(
                    src,
                    &mut bgr,
                    imgproc::COLOR_GRAY2BGR,
                    0,
                    core::AlgorithmHint::ALGO_HINT_APPROX,
                )
                .map_err(|e| e.to_string())?;
                Ok(bgr)
            } else {
                Ok(src.clone())
            }
        }
    }
}

/// Гамма-коррекция: dst = 255 * (src/255)^(1/gamma).
fn apply_gamma(src: &Mat, dst: &mut Mat, gamma: f32) -> Result<(), String> {
    let rows = src.rows();
    let cols = src.cols();
    let total = (rows * cols) as usize;

    // Таблица LUT 256 значений
    let inv_gamma = 1.0 / gamma;
    let mut lut = vec![0u8; 256];
    for i in 0..256 {
        let normalized = i as f32 / 255.0;
        let corrected = normalized.powf(inv_gamma) * 255.0;
        lut[i] = corrected.clamp(0.0, 255.0) as u8;
    }

    unsafe {
        let src_data = std::slice::from_raw_parts(src.data() as *const u8, total);
        let mut dst_mat = Mat::zeros(rows, cols, core::CV_8UC1)
            .map_err(|e| e.to_string())?
            .to_mat()
            .map_err(|e| e.to_string())?;
        let dst_data = std::slice::from_raw_parts_mut(dst_mat.data_mut() as *mut u8, total);
        for i in 0..total {
            dst_data[i] = lut[src_data[i] as usize];
        }
        dst_mat.clone_into(dst);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Тестовый градиент: левая половина тёмная, правая светлая.
    fn make_gradient(rows: i32, cols: i32) -> Mat {
        let mut mat = Mat::zeros(rows, cols, core::CV_8UC1)
            .unwrap()
            .to_mat()
            .unwrap();
        unsafe {
            let data = std::slice::from_raw_parts_mut(mat.data_mut() as *mut u8, (rows * cols) as usize);
            for y in 0..rows as usize {
                for x in 0..cols as usize {
                    data[y * cols as usize + x] = (x as f32 / cols as f32 * 255.0) as u8;
                }
            }
        }
        mat
    }

    #[test]
    fn test_profile_text_bw() {
        let src = make_gradient(100, 200);
        let result = apply_profile(&src, ProcessingProfile::TextBw1bit, 0.2, 15).unwrap();
        assert_eq!(result.channels(), 1);
        // Бинарный результат: только 0 и 255
        unsafe {
            let data = std::slice::from_raw_parts(result.data() as *const u8, 100 * 200);
            assert!(data.iter().all(|&v| v == 0 || v == 255));
        }
    }

    #[test]
    fn test_profile_grayscale() {
        let src = make_gradient(100, 200);
        let result = apply_profile(&src, ProcessingProfile::IllustrationGrayscale8bit, 0.2, 15).unwrap();
        assert_eq!(result.channels(), 1);
        assert_eq!(result.rows(), 100);
        assert_eq!(result.cols(), 200);
    }

    #[test]
    fn test_profile_color() {
        let src = make_gradient(100, 200);
        let result = apply_profile(&src, ProcessingProfile::ColorRgb24bit, 0.2, 15).unwrap();
        assert_eq!(result.channels(), 3);
    }

    #[test]
    fn test_profile_from_str() {
        assert_eq!(
            ProcessingProfile::from_str_lenient("text_bw_1bit"),
            ProcessingProfile::TextBw1bit
        );
        assert_eq!(
            ProcessingProfile::from_str_lenient("illustration_grayscale_8bit"),
            ProcessingProfile::IllustrationGrayscale8bit
        );
        assert_eq!(
            ProcessingProfile::from_str_lenient("color_rgb_24bit"),
            ProcessingProfile::ColorRgb24bit
        );
        assert_eq!(
            ProcessingProfile::from_str_lenient("unknown"),
            ProcessingProfile::TextBw1bit
        );
    }
}