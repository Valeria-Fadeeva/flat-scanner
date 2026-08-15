//! M8: Калибровка бинаризации Сауволы с hot-reload.
//!
//! Параметры (`k_factor`, `window_size`) читаются из файла `calibration.json`
//! в корне проекта. Файл может быть изменён во время работы (из Flutter UI
//! или вручную) — модуль отслеживает `mtime` и перечитывает параметры
//! без перезапуска процесса.
//!
//! Формат `calibration.json`:
//! ```json
//! {
//!   "k_factor": 0.2,
//!   "window_size": 15,
//!   "profile": "text_bw_1bit"
//! }
//! ```

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::profile_filtering::ProcessingProfile;

/// Путь к файлу калибровки (относительно CWD).
const CALIBRATION_FILE: &str = "calibration.json";

/// Минимальный интервал между перечитываниями файла (защита от трешхолда mtime).
const RELOAD_INTERVAL: Duration = Duration::from_millis(500);

/// Параметры калибровки бинаризации.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationParams {
    /// Коэффициент Сауволы (обычно 0.1–0.5).
    #[serde(default = "default_k_factor")]
    pub k_factor: f32,
    /// Размер окна Сауволы в пикселях (нечётное, обычно 11–51).
    #[serde(default = "default_window_size")]
    pub window_size: i32,
    /// Профиль обработки по умолчанию.
    #[serde(default)]
    pub profile: String,
}

fn default_k_factor() -> f32 {
    0.2
}

fn default_window_size() -> i32 {
    15
}

impl Default for CalibrationParams {
    fn default() -> Self {
        Self {
            k_factor: default_k_factor(),
            window_size: default_window_size(),
            profile: "text_bw_1bit".to_string(),
        }
    }
}

impl CalibrationParams {
    /// Профиль обработки из строки конфигурации.
    pub fn processing_profile(&self) -> ProcessingProfile {
        ProcessingProfile::from_str_lenient(&self.profile)
    }
}

/// Глобальный менеджер калибровки с hot-reload.
///
/// Обёртка над `Mutex<CalibrationParams>` + кэш `mtime` файла.
/// `get()` перечитывает файл, если он изменился с последнего обращения
/// (с учётом `RELOAD_INTERVAL`).
#[derive(Debug)]
pub struct CalibrationManager {
    path: String,
    params: Mutex<CalibrationParams>,
    last_mtime: Mutex<Option<u128>>,
    last_check: Mutex<std::time::Instant>,
}

impl CalibrationManager {
    /// Создаёт менеджер, загружая параметры из файла (или дефолты).
    pub fn new() -> Self {
        let path = CALIBRATION_FILE.to_string();
        let params = Self::load_from_file(&path).unwrap_or_default();
        let mtime = Self::file_mtime(&path);
        Self {
            path,
            params: Mutex::new(params),
            last_mtime: Mutex::new(mtime),
            last_check: Mutex::new(std::time::Instant::now()),
        }
    }

    /// Возвращает актуальные параметры, перечитывая файл при изменении.
    pub fn get(&self) -> CalibrationParams {
        // Троттлинг: не проверяем mtime чаще, чем раз в RELOAD_INTERVAL
        {
            let mut last_check = self.last_check.lock().unwrap();
            if last_check.elapsed() < RELOAD_INTERVAL {
                return self.params.lock().unwrap().clone();
            }
            *last_check = std::time::Instant::now();
        }

        let current_mtime = Self::file_mtime(&self.path);
        let mut last_mtime = self.last_mtime.lock().unwrap();

        if current_mtime != *last_mtime {
            *last_mtime = current_mtime;
            if let Some(mtime) = current_mtime {
                let _ = mtime; // файл существует
                if let Ok(new_params) = Self::load_from_file(&self.path) {
                    *self.params.lock().unwrap() = new_params;
                }
            }
        }

        self.params.lock().unwrap().clone()
    }

    /// Принудительно перечитывает файл (обход кэша mtime).
    /// Публичный API: будет вызываться из Flutter UI (endpoint /api/v1/calibration).
    #[allow(dead_code)]
    pub fn reload(&self) -> CalibrationParams {
        let new_params = Self::load_from_file(&self.path).unwrap_or_default();
        *self.params.lock().unwrap() = new_params.clone();
        *self.last_mtime.lock().unwrap() = Self::file_mtime(&self.path);
        new_params
    }

    /// Сохраняет параметры в файл (используется Flutter UI для калибровки).
    /// Публичный API: будет вызываться из Flutter UI (endpoint /api/v1/calibration).
    #[allow(dead_code)]
    pub fn save(&self, params: &CalibrationParams) -> Result<(), String> {
        let json = serde_json::to_string_pretty(params).map_err(|e| e.to_string())?;
        fs::write(&self.path, json).map_err(|e| e.to_string())?;
        *self.params.lock().unwrap() = params.clone();
        *self.last_mtime.lock().unwrap() = Self::file_mtime(&self.path);
        Ok(())
    }

    fn load_from_file(path: &str) -> Result<CalibrationParams, String> {
        let content = fs::read_to_string(path).map_err(|e| e.to_string())?;
        let params: CalibrationParams = serde_json::from_str(&content).map_err(|e| e.to_string())?;
        Ok(params)
    }

    fn file_mtime(path: &str) -> Option<u128> {
        let meta = fs::metadata(Path::new(path)).ok()?;
        let modified = meta.modified().ok()?;
        let duration = modified.duration_since(UNIX_EPOCH).ok()?;
        Some(duration.as_millis())
    }
}

impl Default for CalibrationManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Глобальный экземпляр менеджера калибровки (ленивая инициализация).
pub fn global_calibration() -> &'static CalibrationManager {
    use std::sync::OnceLock;
    static INSTANCE: OnceLock<CalibrationManager> = OnceLock::new();
    INSTANCE.get_or_init(CalibrationManager::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_params() {
        let params = CalibrationParams::default();
        assert_eq!(params.k_factor, 0.2);
        assert_eq!(params.window_size, 15);
        assert_eq!(params.processing_profile(), ProcessingProfile::TextBw1bit);
    }

    #[test]
    fn test_profile_parsing() {
        let params = CalibrationParams {
            profile: "illustration_grayscale_8bit".to_string(),
            ..Default::default()
        };
        assert_eq!(
            params.processing_profile(),
            ProcessingProfile::IllustrationGrayscale8bit
        );
    }

    #[test]
    fn test_save_and_reload() {
        let manager = CalibrationManager::new();
        let mut params = CalibrationParams::default();
        params.k_factor = 0.35;
        params.window_size = 25;

        manager.save(&params).unwrap();
        let reloaded = manager.reload();
        assert_eq!(reloaded.k_factor, 0.35);
        assert_eq!(reloaded.window_size, 25);

        // Убираем тестовый файл
        let _ = fs::remove_file(CALIBRATION_FILE);
    }

    #[test]
    fn test_json_deserialization() {
        let json = r#"{"k_factor": 0.25, "window_size": 21, "profile": "color_rgb_24bit"}"#;
        let params: CalibrationParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.k_factor, 0.25);
        assert_eq!(params.window_size, 21);
        assert_eq!(params.processing_profile(), ProcessingProfile::ColorRgb24bit);
    }
}