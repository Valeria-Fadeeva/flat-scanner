//! Экспорт бинаризированных страниц в CCITT Group 4 TIFF через FFI к libtiff.
//!
//! Крейт `tiff` (0.11) не поддерживает CCITT Group 4, а встроенный TIFF-энкодер
//! OpenCV требует 1-битный вход, но принимает 8-битный — отсюда ошибка
//! "Bits/sample must be 1 for Group 3/4 encoding/decoding".
//!
//! Решение: прямой FFI к системной libtiff (4.7+), которая корректно кодирует
//! CCITT Group 4 (T.6) из 1-битных данных.

use opencv::{core, imgproc, prelude::*, core::Mat};

// ─── Константы libtiff ───────────────────────────────────────────────────────
const TIFFTAG_IMAGEWIDTH: u32 = 256;
const TIFFTAG_IMAGELENGTH: u32 = 257;
const TIFFTAG_BITSPERSAMPLE: u32 = 258;
const TIFFTAG_COMPRESSION: u32 = 259;
const TIFFTAG_PHOTOMETRIC: u32 = 262;
const TIFFTAG_FILLORDER: u32 = 266;

const COMPRESSION_CCITTFAX4: u16 = 4; // CCITT Group 4
const PHOTOMETRIC_MINISBLACK: u16 = 1; // 0 = чёрный (текст)
const FILLORDER_MSB2LSB: u16 = 1; // MSB-first

// ─── FFI-объявления libtiff ──────────────────────────────────────────────────
use std::ffi::c_uint;

unsafe extern "C" {
    fn TIFFOpen(filename: *const std::ffi::c_char, mode: *const std::ffi::c_char) -> *mut std::ffi::c_void;
    fn TIFFSetField(tif: *mut std::ffi::c_void, tag: u32, ...) -> i32;
    fn TIFFWriteScanline(tif: *mut std::ffi::c_void, buf: *const std::ffi::c_void, row: u32, width: u32) -> i32;
    fn TIFFClose(tif: *mut std::ffi::c_void);
}

/// Преобразование изображения в grayscale.
fn to_gray(src: &Mat) -> Result<Mat, String> {
    if src.channels() > 1 {
        let mut gray = Mat::default();
        imgproc::cvt_color(
            src,
            &mut gray,
            imgproc::COLOR_BGR2GRAY,
            0,
            core::AlgorithmHint::ALGO_HINT_APPROX,
        )
        .map_err(|e| e.to_string())?;
        Ok(gray)
    } else {
        Ok(src.clone())
    }
}

/// Кодирует монохромное изображение в CCITT Group 4 TIFF и сохраняет в файл.
///
/// # Аргументы
/// * `src` — Входное изображение (grayscale или BGR). Тёмный текст, светлый фон.
/// * `path` — Путь к выходному файлу
///
/// # Возвращает
/// * `Ok(usize)` — Размер файла в байтах
/// * `Err(String)` — Сообщение об ошибке
pub fn encode_ccitt_g4_to_file(src: &Mat, path: &str) -> Result<usize, String> {
    let rows = src.rows() as usize;
    let cols = src.cols() as usize;
    if rows == 0 || cols == 0 {
        return Err("Пустое изображение".to_string());
    }

    // 1. Grayscale
    let gray = to_gray(src)?;

    // 2. Бинаризация: текст (тёмный) = 0, фон (светлый) = 255
    let mut binary = Mat::default();
    imgproc::threshold(&gray, &mut binary, 127.5, 255.0, imgproc::THRESH_BINARY)
        .map_err(|e| e.to_string())?;

    // 3. Упаковка битов MSB-first (8 пикселей в байт)
    let row_bytes = (cols + 7) / 8;
    let mut packed = vec![0u8; rows * row_bytes];
    unsafe {
        let data = std::slice::from_raw_parts(binary.data() as *const u8, rows * cols);
        for y in 0..rows {
            for x in 0..cols {
                if data[y * cols + x] != 0 {
                    packed[y * row_bytes + x / 8] |= 0x80u8 >> (x % 8);
                }
            }
        }
    }

    // 4. Открытие TIFF через libtiff
    let c_path = std::ffi::CString::new(path)
        .map_err(|e| format!("Некорректный путь: {}", e))?;
    let c_mode = std::ffi::CString::new("w").unwrap();

    let tif = unsafe { TIFFOpen(c_path.as_ptr(), c_mode.as_ptr()) };
    if tif.is_null() {
        return Err(format!("Не удалось открыть TIFF для записи: {}", path));
    }

    // 5. Установка тегов
    unsafe {
        let ok = |r: i32| r != 0;

        if !ok(TIFFSetField(tif, TIFFTAG_IMAGEWIDTH, cols as c_uint)) {
            TIFFClose(tif);
            return Err("Не удалось установить ImageWidth".to_string());
        }
        if !ok(TIFFSetField(tif, TIFFTAG_IMAGELENGTH, rows as c_uint)) {
            TIFFClose(tif);
            return Err("Не удалось установить ImageLength".to_string());
        }
        if !ok(TIFFSetField(tif, TIFFTAG_BITSPERSAMPLE, 1u32)) {
            TIFFClose(tif);
            return Err("Не удалось установить BitsPerSample=1".to_string());
        }
        if !ok(TIFFSetField(tif, TIFFTAG_COMPRESSION, COMPRESSION_CCITTFAX4 as c_uint)) {
            TIFFClose(tif);
            return Err("Не удалось установить Compression=CCITT Group 4".to_string());
        }
        if !ok(TIFFSetField(tif, TIFFTAG_PHOTOMETRIC, PHOTOMETRIC_MINISBLACK as c_uint)) {
            TIFFClose(tif);
            return Err("Не удалось установить Photometric=MinIsBlack".to_string());
        }
        if !ok(TIFFSetField(tif, TIFFTAG_FILLORDER, FILLORDER_MSB2LSB as c_uint)) {
            TIFFClose(tif);
            return Err("Не удалось установить FillOrder=MSB2LSB".to_string());
        }

        // 6. Запись scanlines
        for y in 0..rows {
            let row_ptr = packed.as_ptr().add(y * row_bytes) as *const std::ffi::c_void;
            let r = TIFFWriteScanline(tif, row_ptr, y as u32, row_bytes as u32);
            if r < 0 {
                TIFFClose(tif);
                return Err(format!("Ошибка записи scanline {}", y));
            }
        }

        // 7. Закрытие
        TIFFClose(tif);
    }

    let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
    Ok(metadata.len() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_ccitt_g4() {
        // Тестовое изображение 100x100: белый фон + чёрный квадрат (текст)
        let mut src = Mat::ones(100, 100, core::CV_8UC1)
            .unwrap()
            .to_mat()
            .unwrap();
        unsafe {
            let data = std::slice::from_raw_parts_mut(src.data_mut() as *mut u8, 100 * 100);
            for y in 20..40 {
                for x in 20..40 {
                    data[y * 100 + x] = 0;
                }
            }
        }

        let size = encode_ccitt_g4_to_file(&src, "/tmp/test_ccitt_g4.tiff").unwrap();
        assert!(size > 0);
    }

    #[test]
    fn test_encode_ccitt_g4_all_white() {
        let src = Mat::ones(50, 50, core::CV_8UC1)
            .unwrap()
            .to_mat()
            .unwrap();
        let size = encode_ccitt_g4_to_file(&src, "/tmp/test_ccitt_g4_white.tiff").unwrap();
        assert!(size > 0);
    }
}