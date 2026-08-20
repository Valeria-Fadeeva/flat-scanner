//! Конфигурация сервера Flat Scanner.
//!
//! Приоритет источников:
//! 1. CLI-флаги (`--host`, `--port`) — имеют высший приоритет.
//! 2. Файл конфигурации `config.toml` (см. [`config_path`]).
//! 3. Дефолтные значения (`127.0.0.1:54321`).
//!
//! Файл конфигурации ищется в порядке:
//! - `$FLAT_SCANNER_CONFIG` (переменная окружения, для systemd);
//! - `~/.config/flat-scanner-server/config.toml`;
//! - `/etc/flat-scanner-server/config.toml` (системный, для PKGBUILD).

use serde::Deserialize;
use std::path::PathBuf;

/// Дефолтный адрес привязки (только локальная машина).
pub const DEFAULT_HOST: &str = "127.0.0.1";
/// Дефолтный порт HTTP-шлюза.
pub const DEFAULT_PORT: u16 = 54321;

/// Структура файла конфигурации `config.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Секция сетевого сервера.
    pub server: ServerConfig,
    /// Секция путей хранения данных.
    pub paths: PathsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig::default(),
            paths: PathsConfig::default(),
        }
    }
}

/// Параметры сетевого шлюза Axum.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Адрес привязки: `127.0.0.1` (локально) или `0.0.0.0` (по сети).
    pub host: String,
    /// Порт HTTP-шлюза.
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: DEFAULT_HOST.to_string(),
            port: DEFAULT_PORT,
        }
    }
}

/// Параметры путей хранения данных (XDG Base Directory Specification).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct PathsConfig {
    /// Базовый каталог для всех данных (поддерживается ~).
    pub base_dir: String,
    /// Подкаталог для сырых сканов.
    pub raw_dir: String,
    /// Подкаталог для обработанных страниц.
    pub processed_dir: String,
    /// Подкаталог для экспорта PDF.
    pub export_dir: String,
    /// Подкаталог для импорта PDF.
    pub import_dir: String,
    /// Файл базы данных.
    pub database: String,
}

impl Default for PathsConfig {
    fn default() -> Self {
        Self {
            base_dir: "~/.local/share/flat-scanner".to_string(),
            raw_dir: "raw".to_string(),
            processed_dir: "processed".to_string(),
            export_dir: "export".to_string(),
            import_dir: "import".to_string(),
            database: "data.db".to_string(),
        }
    }
}

impl PathsConfig {
    /// Резолвит ~ в $HOME.
    fn resolve_home(path: &str) -> PathBuf {
        if let Some(rest) = path.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest);
            }
        }
        PathBuf::from(path)
    }

    /// Возвращает абсолютный путь к базовому каталогу.
    pub fn base_path(&self) -> PathBuf {
        Self::resolve_home(&self.base_dir)
    }

    /// Возвращает абсолютный путь к каталогу сырых сканов.
    pub fn raw_path(&self) -> PathBuf {
        self.base_path().join(&self.raw_dir)
    }

    /// Возвращает абсолютный путь к каталогу обработанных страниц.
    pub fn processed_path(&self) -> PathBuf {
        self.base_path().join(&self.processed_dir)
    }

    /// Возвращает абсолютный путь к каталогу экспорта PDF.
    pub fn export_path(&self) -> PathBuf {
        self.base_path().join(&self.export_dir)
    }

    /// Возвращает абсолютный путь к каталогу импорта PDF.
    pub fn import_path(&self) -> PathBuf {
        self.base_path().join(&self.import_dir)
    }

    /// Возвращает абсолютный путь к базе данных.
    pub fn database_path(&self) -> PathBuf {
        self.base_path().join(&self.database)
    }

    /// Создаёт все каталоги из конфигурации.
    pub fn create_directories(&self) -> Result<(), String> {
        let dirs = [
            self.base_path(),
            self.raw_path(),
            self.processed_path(),
            self.export_path(),
            self.import_path(),
        ];
        for dir in dirs {
            std::fs::create_dir_all(&dir)
                .map_err(|e| format!("Не удалось создать каталог {}: {}", dir.display(), e))?;
        }
        Ok(())
    }
}

impl Config {
    /// Возвращает адрес привязки `host:port` как строку.
    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.server.host, self.server.port)
    }

    /// Загружает конфигурацию из файла, если он существует.
    /// При отсутствии файла или ошибке парсинга возвращает дефолты.
    pub fn load() -> Self {
        if let Some(path) = config_path() {
            if path.exists() {
                match std::fs::read_to_string(&path) {
                    Ok(contents) => match toml::from_str::<Config>(&contents) {
                        Ok(cfg) => {
                            println!("[⚙️ CONFIG]: Загружена конфигурация из {}", path.display());
                            return cfg;
                        }
                        Err(e) => {
                            println!(
                                "[⚠️ CONFIG]: Ошибка парсинга {}: {}. Использую дефолты.",
                                path.display(),
                                e
                            );
                        }
                    },
                    Err(e) => {
                        println!(
                            "[⚠️ CONFIG]: Ошибка чтения {}: {}. Использую дефолты.",
                            path.display(),
                            e
                        );
                    }
                }
            }
        }
        Config::default()
    }

    /// Применяет CLI-флаги поверх загруженной конфигурации.
    /// `None` означает «не задан, оставить значение из файла/дефолта».
    pub fn apply_cli_overrides(&mut self, host: Option<String>, port: Option<u16>) {
        if let Some(h) = host {
            self.server.host = h;
        }
        if let Some(p) = port {
            self.server.port = p;
        }
    }
}

/// Определяет путь к файлу конфигурации.
fn config_path() -> Option<PathBuf> {
    // 1. Переменная окружения (systemd / PKGBUILD)
    if let Ok(custom) = std::env::var("FLAT_SCANNER_CONFIG") {
        return Some(PathBuf::from(custom));
    }

    // 2. Пользовательский каталог ~/.config/flat-scanner/config.toml
    if let Some(config_dir) = dirs::config_dir() {
        let user_path = config_dir.join("flat-scanner").join("config.toml");
        if user_path.exists() {
            return Some(user_path);
        }
    }

    // 3. Системный каталог /etc/flat-scanner/config.toml
    let system_path = PathBuf::from("/etc/flat-scanner/config.toml");
    if system_path.exists() {
        return Some(system_path);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_local() {
        let cfg = Config::default();
        assert_eq!(cfg.server.host, "127.0.0.1");
        assert_eq!(cfg.server.port, 54321);
        assert_eq!(cfg.bind_addr(), "127.0.0.1:54321");
    }

    #[test]
    fn parse_toml_config() {
        let toml_str = r#"
            [server]
            host = "0.0.0.0"
            port = 8080

            [paths]
            base_dir = "~/.local/share/flat-scanner"
            raw_dir = "raw"
            processed_dir = "processed"
            export_dir = "export"
            import_dir = "import"
            database = "data.db"
        "#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.server.host, "0.0.0.0");
        assert_eq!(cfg.server.port, 8080);
        assert_eq!(cfg.paths.base_dir, "~/.local/share/flat-scanner");
        assert_eq!(cfg.paths.database, "data.db");
    }

    #[test]
    fn paths_config_defaults() {
        let paths = PathsConfig::default();
        assert_eq!(paths.base_dir, "~/.local/share/flat-scanner");
        assert_eq!(paths.raw_dir, "raw");
        assert_eq!(paths.processed_dir, "processed");
        assert_eq!(paths.export_dir, "export");
        assert_eq!(paths.import_dir, "import");
        assert_eq!(paths.database, "data.db");
    }

    #[test]
    fn cli_overrides_win() {
        let mut cfg = Config::default();
        cfg.apply_cli_overrides(Some("0.0.0.0".to_string()), Some(9999));
        assert_eq!(cfg.bind_addr(), "0.0.0.0:9999");
    }
}