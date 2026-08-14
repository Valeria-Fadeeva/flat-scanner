//! Экспорт бинаризированных страниц в CCITT Group 4 TIFF через OpenCV
//!
//! Использует встроенный TIFF-энкодер OpenCV с параметром IMWRITE_TIFF_COMPRESSION=8
//! для CCITT Group 4 (T.6) сжатия без потерь.

use opencv::core::{Mat, Vector};
use opencv::imgcodecs::{imwrite, IMWRITE_TIFF_COMPRESSION, IMWRITE_TIFF_COMPRESSION_CCITT_T6};

/// Кодирует монохромное изображение в CCITT Group 4 TIFF и сохраняет в файл.
///
/// # Аргументы
/// * `src` — Входное 8-битное одноканальное изображение (0=чёрный текст, 255=белый фон)
/// * `path` — Путь к выходному файлу
///
/// # Возвращает
/// * `Ok(usize)` — Размер файла в байтах
/// * `Err(String)` — Сообщение об ошибке
pub fn encode_ccitt_g4_to_file(src: &Mat, path: &str) -> Result<usize, String> {
    // Параметры для CCITT Group 4 сжатия
    let mut params = Vector::<i32>::new();
    params.push(IMWRITE_TIFF_COMPRESSION);
    params.push(IMWRITE_TIFF_COMPRESSION_CCITT_T6);

    // Сохранение через OpenCV
    imwrite(path, src, &params).map_err(|e| format!("Ошибка сохранения TIFF: {}", e))?;

    // Получение размера файла
    let metadata = std::fs::metadata(path).map_err(|e| format!("Ошибка получения метаданных: {}", e))?;
    Ok(metadata.len() as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opencv::core::{Mat, MatFlags, Size};

    #[test]
    fn test_encode_ccitt_g4() {
        // Создание тестового изображения 100x100
        let mut src = Mat::zeros::<u8>(Size::new(100, 100), MatFlags::CV_8UC1).unwrap();
        // Добавление "текста" (чёрные пиксели)
        for i in 10..20 {
            for j in 10..20 {
                src.at_mut::<u8>((i, j)).unwrap();
            }
        }

        // Кодирование
        let size = encode_ccitt_g4_to_file(&src, "/tmp/test_ccitt_g4.tiff").unwrap();
        assert!(size > 0);
        assert!(size < 100 * 100); // Должно быть меньше оригинала
    }
}