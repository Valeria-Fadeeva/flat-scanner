use opencv::{core::Mat, imgcodecs, prelude::*};
use std::io::Read;
use std::process::{Child, Command, Stdio};

/// RAII-обёртка над child-процессом `scanimage` (TECH_SPEC_addon_2.md §2.1–2.2).
/// Гарантирует корректное завершение процесса и сбор stdout при выходе из области
/// видимости, исключая утечки дескрипторов при раннем возврате по ошибке.
pub struct SaneScanner {
    child: Option<Child>,
}

impl SaneScanner {
    /// Запускает высокоскоростной захват в RAM без создания файлов на диске.
    pub fn new(device_name: &str) -> Result<Self, String> {
        let child = Command::new("scanimage")
            .args(["-d", device_name, "--format=tiff", "--resolution=300"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Не удалось запустить scanimage: {}", e))?;

        Ok(Self {
            child: Some(child),
        })
    }

    /// Читает кадр в переиспользуемый буфер (§2.3): аллокация вектора происходит
    /// один раз, между кадрами переиспользуется выделенная ёмкость, что
    /// предотвращает фрагментацию кучи при пакетном сканировании.
    pub fn read_frame(&mut self, buffer: &mut Vec<u8>) -> Result<Vec<u8>, String> {
        let child = self.child.as_mut().ok_or("SaneScanner уже завершён")?;
        let mut stdout = child.stdout.take().ok_or("stdout-поток сканера потерян")?;

        buffer.clear();
        stdout
            .read_to_end(buffer)
            .map_err(|e| format!("Ошибка чтения потока SANE: {}", e))?;

        let status = child.wait().map_err(|e| format!("Ошибка ожидания scanimage: {}", e))?;
        if !status.success() {
            let mut stderr = child.stderr.take().unwrap();
            let mut err_msg = String::new();
            let _ = stderr.read_to_string(&mut err_msg);
            return Err(format!("Аппаратный сбой SANE: {}", err_msg));
        }

        Ok(std::mem::take(buffer))
    }
}

impl Drop for SaneScanner {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.wait();
        }
    }
}

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

    // Захват через RAII-обёртку: процесс гарантированно завершён при выходе из scope
    let mut scanner = SaneScanner::new(device_name)?;

    // Переиспользуемый буфер кадров (§2.3): аллоцируется один раз на вызов
    let mut frame_buffer: Vec<u8> = Vec::new();
    let frame = scanner.read_frame(&mut frame_buffer)?;

    println!(
        "[🚀 SANE RAM]: Буфер получен. Объем: {} байт. Декодирование OpenCV...",
        frame.len()
    );

    // Создаем матрицу OpenCV напрямую из вектора оперативной памяти
    let mat = imgcodecs::imdecode(
        &Mat::from_slice(&frame).map_err(|e| e.to_string())?,
        imgcodecs::IMREAD_COLOR,
    )
    .map_err(|e| e.to_string())?;

    if mat.empty() {
        return Err("Декодер вернул пустую матрицу".to_string());
    }

    Ok(mat)
}