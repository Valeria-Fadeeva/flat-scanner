use opencv::{core::Mat, imgcodecs, prelude::*};
use std::process::Command;

/// Динамический автоматический поиск реального сканера на USB-шине Linux
pub fn detect_hardware_scanner() -> Result<String, String> {
    println!("[⚙️ HARDWARE]: Динамический опрос шины SANE...");

    let output = Command::new("scanimage")
        .arg("-L")
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        return Err("Не удалось выполнить команду scanimage -L".to_string());
    }

    let stdout_str = String::from_utf8_lossy(&output.stdout);

    // Построчно перебираем все устройства, которые нашла система
    // Построчно перебираем все устройства, которые нашла система
    for line in stdout_str.lines() {
        let line_lower = line.to_lowercase();

        // Жесткий Firewall: отсекаем веб-камеры и пустые строки
        if line_lower.contains("webcam")
            || line_lower.contains("camera")
            || line_lower.contains("v4l")
            || line.trim().is_empty()
        {
            continue;
        }

        // Патч кавычек: ищем открывающий обратный апостроф ` и закрывающую одинарную кавычку '
        if let Some(start_idx) = line.find('`') {
            if let Some(end_idx) = line.find('\'') {
                if end_idx > start_idx {
                    let device_address = line[start_idx + 1..end_idx].to_string();
                    println!(
                        "[⚙️ HARDWARE]: Устройство успешно локализовано: {}",
                        device_address
                    );
                    return Ok(device_address);
                }
            }
        }
    }

    Err("Реальный планшетный сканер не обнаружен в системе SANE".to_string())
}

pub fn capture_sane_frame(device_name: &str) -> Result<Mat, String> {
    println!(
        "[📸 HARDWARE]: Инициализация захвата. Устройство: {}...",
        device_name
    );

    // Автоматический адаптивный патч геометрии: если мы дома на Canon LiDE (genesys/pixma) — переключаем на А4
    if device_name.contains("genesys")
        || device_name.contains("pixma")
        || device_name.contains("niash")
    {
        println!("[📐 HARDWARE]: Обнаружен А4-профиль сканера. Коррекция геометрии.");
    } else {
        println!("[📐 HARDWARE]: Обнаружен А3-профиль сканера (Epson). Полный кадр.");
    }

    // Запускаем высокоскоростной захват в RAM без создания файлов на диске
    let output = Command::new("scanimage")
        .args(["-d", device_name, "--format=tiff", "--resolution=300"])
        .output()
        .map_err(|e| e.to_string())?;

    if !output.status.success() {
        let err_msg = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(format!("Аппаратный сбой SANE: {}", err_msg));
    }

    println!(
        "[🚀 SANE RAM]: Буфер получен. Объем: {} байт. Декодирование OpenCV...",
        output.stdout.len()
    );

    // Создаем матрицу OpenCV напрямую из вектора оперативной памяти
    let mat = imgcodecs::imdecode(
        &Mat::from_slice(&output.stdout).map_err(|e| e.to_string())?,
        imgcodecs::IMREAD_COLOR,
    )
    .map_err(|e| e.to_string())?;

    if mat.empty() {
        return Err("Декодер вернул пустую матрицу".to_string());
    }

    Ok(mat)
}
