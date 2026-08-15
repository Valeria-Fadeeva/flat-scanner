# Flat Scanner Server (Kanonissa Core Engine)

Двухрежимное ядро оцифровки книг без OCR. Захватывает кадр с планшетного сканера (SANE),
детектирует вершины страниц, выполняет перспективную коррекцию и деварпинг корешка,
сегментирует разворот на левую/правую страницы, выравнивает скос и бинаризует
(Сауволла) с сохранением в CCITT Group 4 TIFF (1-бит) или PNG (grayscale/color).

## Стек

- **Rust** (edition 2024)
- **Axum** — асинхронный HTTP-шлюз (Tokio)
- **OpenCV 4.x** — компьютерное зрение (детекция, warping, сегментация)
- **SANE** — захват кадра со сканера
- **SQLite (rusqlite)** — транзакционное хранение сессий сканирования
- **tiff** — CCITT Group 4 сжатие

## Зависимости (Arch Linux)

```bash
sudo pacman -S opencv sane libtiff sqlite
```

## Запуск

### Web-режим (по умолчанию)

```bash
cargo run
```

Сервер слушает на `127.0.0.1:54321` (по умолчанию).

### CLI-режим (без веб-сервера)

```bash
cargo run -- --cli --output-dir ./split --k-factor 0.2
# или с готовым файлом разворота:
cargo run -- --cli --input-file spread.png
```

## Конфигурация bind-адреса

Приоритет источников: **CLI-флаг > config.toml > дефолт**.

### CLI-флаги

```bash
cargo run -- --host 0.0.0.0 --port 8080
```

### Файл конфигурации `config.toml`

```toml
[server]
host = "127.0.0.1"   # или "0.0.0.0" для доступа по сети
port = 54321
```

Путь к файлу (по порядку поиска):
1. `$FLAT_SCANNER_CONFIG` (переменная окружения, для systemd)
2. `~/.config/flat-scanner-server/config.toml`
3. `/etc/flat-scanner-server/config.toml` (системный, устанавливается PKGBUILD)

> ⚠️ При `host = "0.0.0.0"` сервер доступен по сети — убедитесь, что это намеренно.

## API

| Метод | Путь | Описание |
|-------|------|----------|
| GET | `/api/v1/health` | Проверка доступности движка |
| POST | `/api/v1/scanner/init` | Инициализация каретки сканера |
| POST | `/api/v1/scanner/process` | Захват + обработка разворота |

### Пример запроса `/api/v1/scanner/process`

```json
{
  "uuid": "a1b2c3d4",
  "threshold_preset": 0,
  "profile": "text_bw_1bit"
}
```

Профили обработки (`profile`):
- `text_bw_1bit` — текст, 1-бит, CCITT G4 TIFF
- `illustration_grayscale_8bit` — иллюстрации, 8-бит, PNG
- `color_rgb_24bit` — цвет, 24-бит, PNG

## systemd-сервис

Устанавливается PKGBUILD в `/usr/lib/systemd/system/flat-scanner-server.service`.

```bash
sudo systemctl enable --now flat-scanner-server
sudo systemctl status flat-scanner-server
journalctl -u flat-scanner-server -f
```

Сервис:
- `Restart=on-failure` — автоперезапуск (важно для hot-restart сессии)
- `WorkingDirectory=/var/lib/flat-scanner` — где лежат `kanonissa.db`, `calibration.json`, `split/`
- `Environment=FLAT_SCANNER_CONFIG=/etc/flat-scanner-server/config.toml`

## Сборка в PKGBUILD (Arch Linux)

```bash
makepkg -si
```

Устанавливает:
- `/usr/bin/flat_scanner_server` — бинарник
- `/usr/lib/systemd/system/flat-scanner-server.service` — systemd-юнит
- `/etc/flat-scanner-server/config.toml` — конфигурация
- `/var/lib/flat-scanner/` — каталог работы
- `/usr/share/doc/flat-scanner-server/README.md` — документация

## Структура

```
flat-scanner-server/
├── src/
│   ├── main.rs              # CLI-парсер + Axum-роутеры + конвейеры
│   ├── config.rs            # Конфигурация bind-адреса (host/port)
│   ├── sane_core.rs         # FFI-слой SANE (захват кадра)
│   ├── session_store.rs     # SQLite-хранилище сессий
│   ├── session_recovery.rs  # Горячий рестарт сессии
│   └── cv/                  # Компьютерное зрение (OpenCV)
│       ├── binarization.rs  # Сауволла
│       ├── calibration.rs   # Hot-reload калибровки
│       ├── ccitt_encoder.rs # CCITT G4 TIFF
│       ├── profile_filtering.rs # Multi-profile обработка
│       ├── segmentation.rs  # Сегментация разворота
│       └── warping.rs       # Перспективная коррекция + деварпинг
├── flat-scanner-server.service  # systemd-юнит
├── config.example.toml          # Пример конфигурации
├── PKGBUILD                     # Сборка для Arch Linux
└── Cargo.toml
```

## Тесты

```bash
cargo test