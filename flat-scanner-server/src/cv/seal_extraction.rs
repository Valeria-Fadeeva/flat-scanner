//! G3 (M5): Сохранение печатей и штампов.
//!
//! Библиотечные штампы и печати — это насыщенные цветовые области, которые
//! Sauvola-бинаризация стирает как «фон». Чернила бывают красными, синими,
//! голубыми и фиолетовыми (зависит от краски и десятилетия), поэтому
//! детекция идёт по **насыщенности (S-канал HSV)**: бумага всегда
//! низконасыщенная, а любая цветная краска даёт высокую насыщенность
//! независимо от оттенка. Алгоритм очищает маску от шума бумаги и налагает
//! её поверх бинаризированного текстового слоя, чтобы печать сохранилась
//! в финальном 1-битном растре.
//!
//! См. TECH_SPEC.md §5.4.

use opencv::{
    core::{self, BORDER_DEFAULT, Mat, Point, Size, Vector},
    imgproc,
    prelude::*,
};

/// Минимальная доля пикселей кадра, при которой область считается печатью.
/// Ниже этого порога маска считается шумом бумаги и возвращается пустой.
const MIN_SEAL_AREA_RATIO: f64 = 0.0005;

/// Извлекает бинарную маску печати/штампа из цветного изображения.
///
/// # Аргументы
/// * `src` — исходное изображение (BGR 3-канальное). Для grayscale-входа
///   возвращается пустая маска (печать не детектируется без цвета).
///
/// # Возвращает
/// Маска CV_8UC1 (0/255), где 255 — пиксель печати. Пустая маска (0 строк),
/// если печать не обнаружена или вход нецветной.
pub fn extract_seal_mask(src: &Mat) -> Result<Mat, String> {
    // Без цветового канала печать не детектируется.
    if src.channels() != 3 {
        return Ok(Mat::default());
    }

    // 1. Переход в HSV.
    let mut hsv = Mat::default();
    imgproc::cvt_color(
        src,
        &mut hsv,
        imgproc::COLOR_BGR2HSV,
        0,
        core::AlgorithmHint::ALGO_HINT_APPROX,
    )
    .map_err(|e| e.to_string())?;

    // 2. Извлечение канала S — насыщенность (индекс 1). Белая/серая бумага
    //    имеет S≈0, любая цветная краска (красная, синяя, голубая,
    //    фиолетовая) — высокую S независимо от оттенка.
    let mut channels = Vector::<Mat>::new();
    core::split(&hsv, &mut channels).map_err(|e| e.to_string())?;
    if channels.len() != 3 {
        return Err("HSV: ожидалось 3 канала".to_string());
    }
    let saturation = channels.get(1).map_err(|e| e.to_string())?;

    // 3. Порог Otsu: разделяет печать и бумагу по модальности насыщенности.
    let mut thresholded = Mat::default();
    imgproc::threshold(
        &saturation,
        &mut thresholded,
        0.0,
        255.0,
        imgproc::THRESH_BINARY | imgproc::THRESH_OTSU,
    )
    .map_err(|e| e.to_string())?;

    // 5. Очистка от шума бумаги: открытие убирает мелкие точки,
    //    закрытие заполняет провалы внутри печати.
    let kernel = imgproc::get_structuring_element(
        imgproc::MORPH_ELLIPSE,
        Size::new(5, 5),
        Point::new(0, 0),
    )
    .map_err(|e| e.to_string())?;
    let mut cleaned = Mat::default();
    imgproc::morphology_ex(
        &thresholded,
        &mut cleaned,
        imgproc::MORPH_OPEN,
        &kernel,
        Point::new(-1, -1),
        2,
        BORDER_DEFAULT,
        core::Scalar::all(0.0),
    )
    .map_err(|e| e.to_string())?;
    let mut closed = Mat::default();
    imgproc::morphology_ex(
        &cleaned,
        &mut closed,
        imgproc::MORPH_CLOSE,
        &kernel,
        Point::new(-1, -1),
        2,
        BORDER_DEFAULT,
        core::Scalar::all(0.0),
    )
    .map_err(|e| e.to_string())?;
    let cleaned = closed;

    // 6. Порог площади: если «печать» занимает ничтожную долю кадра —
    //    это шум бумаги, возвращаем пустую маску.
    let nonzero = count_nonzero_u8(&cleaned);
    let total = (cleaned.rows() as f64) * (cleaned.cols() as f64);
    if total <= 0.0 || (nonzero as f64) / total < MIN_SEAL_AREA_RATIO {
        return Ok(Mat::default());
    }

    Ok(cleaned)
}

/// Подсчёт ненулевых пикселей в CV_8UC1-матрице (обход через raw-слайс).
fn count_nonzero_u8(mat: &Mat) -> usize {
    let total = (mat.rows() * mat.cols()) as usize;
    unsafe {
        let data = std::slice::from_raw_parts(mat.data() as *const u8, total);
        data.iter().filter(|&&v| v != 0).count()
    }
}

/// Накладывает маску печати поверх бинаризированного текстового слоя.
///
/// Пиксели печати принудительно ставятся в чёрный (0), чтобы чернила
/// сохранились в финальном 1-битном растре и не были стёрты
/// Sauvola-бинаризацией. Результат остаётся CV_8UC1 (0/255) — совместим
/// с экспортом в CCITT Group 4.
///
/// # Аргументы
/// * `text_layer` — бинаризированный текстовый слой (белая бумага = 255, чёрный текст = 0)
/// * `seal_mask` — маска печати из `extract_seal_mask` (пустая → слой без изменений)
pub fn overlay_seal_on_text(text_layer: &Mat, seal_mask: &Mat) -> Result<Mat, String> {
    if seal_mask.empty() || seal_mask.rows() != text_layer.rows() || seal_mask.cols() != text_layer.cols() {
        return Ok(text_layer.clone());
    }

    // Конвенция слоя: тёмный текст = 0, светлая бумага = 255 (MINISBLACK).
    // Пиксели печати принудительно ставим в чёрный (0), остальные сохраняют
    // текстовый слой: result = text_layer & (~seal_mask).
    let mut inv_mask = Mat::default();
    core::bitwise_not(seal_mask, &mut inv_mask, &Mat::default())
        .map_err(|e| e.to_string())?;
    let mut result = Mat::default();
    core::bitwise_and(text_layer, &inv_mask, &mut result, &Mat::default())
        .map_err(|e| e.to_string())?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Создаёт BGR-изображение: белая бумага + цветное пятно (печать).
    /// `bgr` — цвет чернил в порядке (B, G, R).
    fn make_page_with_seal(rows: i32, cols: i32, bgr: (u8, u8, u8)) -> Mat {
        let mut mat = Mat::zeros(rows, cols, core::CV_8UC3)
            .unwrap()
            .to_mat()
            .unwrap();
        unsafe {
            let data = std::slice::from_raw_parts_mut(mat.data_mut() as *mut u8, (rows * cols * 3) as usize);
            // Белая бумага (BGR = 255,255,255)
            for i in 0..(rows * cols * 3) as usize {
                data[i] = 255;
            }
            // Цветное пятно в центре
            let cy = rows / 2;
            let cx = cols / 2;
            for y in (cy - 10)..(cy + 10) {
                for x in (cx - 10)..(cx + 10) {
                    let idx = ((y as usize) * cols as usize + x as usize) * 3;
                    data[idx] = bgr.0; // B
                    data[idx + 1] = bgr.1; // G
                    data[idx + 2] = bgr.2; // R
                }
            }
        }
        mat
    }

    /// Проверяет, что печать заданного цвета обнаружена в маске.
    fn assert_seal_detected(src: &Mat) {
        let mask = extract_seal_mask(src).unwrap();
        assert!(!mask.empty(), "печать должна быть обнаружена");
        assert_eq!(mask.channels(), 1);
        assert!(count_nonzero_u8(&mask) > 0, "в маске должны быть пиксели печати");
    }

    #[test]
    fn test_extract_seal_mask_detects_red_seal() {
        // Красное пятно (BGR = 0,0,255)
        assert_seal_detected(&make_page_with_seal(200, 200, (0, 0, 255)));
    }

    #[test]
    fn test_extract_seal_mask_detects_blue_seal() {
        // Синее пятно (BGR = 255,0,0) — преобладающий цвет чернил
        assert_seal_detected(&make_page_with_seal(200, 200, (255, 0, 0)));
    }

    #[test]
    fn test_extract_seal_mask_detects_purple_seal() {
        // Фиолетовое пятно (BGR = 255,0,255)
        assert_seal_detected(&make_page_with_seal(200, 200, (255, 0, 255)));
    }

    #[test]
    fn test_extract_seal_mask_grayscale_returns_empty() {
        let gray = Mat::zeros(100, 100, core::CV_8UC1)
            .unwrap()
            .to_mat()
            .unwrap();
        let mask = extract_seal_mask(&gray).unwrap();
        assert!(mask.empty(), "grayscale-вход не содержит печати");
    }

    /// Создаёт CV_8UC1-матрицу, заполненную значением `value`.
    fn make_filled(rows: i32, cols: i32, value: u8) -> Mat {
        let mut mat = Mat::zeros(rows, cols, core::CV_8UC1)
            .unwrap()
            .to_mat()
            .unwrap();
        unsafe {
            let data = std::slice::from_raw_parts_mut(mat.data_mut() as *mut u8, (rows * cols) as usize);
            for i in 0..(rows * cols) as usize {
                data[i] = value;
            }
        }
        mat
    }

    #[test]
    fn test_overlay_seal_sets_black() {
        // Текстовый слой: всё белое (255)
        let text = make_filled(50, 50, 255);
        // Маска печати: один пиксель
        let mut seal = Mat::zeros(50, 50, core::CV_8UC1)
            .unwrap()
            .to_mat()
            .unwrap();
        unsafe {
            let data = std::slice::from_raw_parts_mut(seal.data_mut() as *mut u8, 50 * 50);
            data[25 * 50 + 25] = 255;
        }
        let result = overlay_seal_on_text(&text, &seal).unwrap();
        unsafe {
            let data = std::slice::from_raw_parts(result.data() as *const u8, 50 * 50);
            // Пиксель печати принудительно чёрный (0), остальные — белый (255)
            assert_eq!(data[25 * 50 + 25], 0);
            assert_eq!(data[0], 255);
        }
    }

    #[test]
    fn test_overlay_seal_empty_mask_noop() {
        let text = make_filled(50, 50, 255);
        let result = overlay_seal_on_text(&text, &Mat::default()).unwrap();
        assert_eq!(result.rows(), 50);
        assert_eq!(result.cols(), 50);
    }
}