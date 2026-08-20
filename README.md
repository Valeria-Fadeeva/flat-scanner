# Flat Scanner

High-performance headless flatbed scanning core engine for digitizing archival and library books (including publications over 75 years old that have entered the public domain) on A3 flatbed scanners (EPSON GT-20000 / Canon LiDE) with open lid.

> **Русская версия:** [README.ru.md](README.ru.md)

## Architecture

```
┌─────────────────────────────────────────┐
│ Flutter Desktop Client (Dart)           │
│ • GUI scanning control                  │
│ • CustomPainter interactive grid        │
│ • BLoC reactive state model             │
│ • Drag-and-Drop vertex adjustment       │
└──────────────┬──────────────────────────┘
               │ HTTP REST API (localhost:54321)
               ▼
┌─────────────────────────────────────────┐
│ Rust Core Engine                        │
│ • Axum Web Server (REST API)            │
│ • SANE Layer (scanimage integration)    │
│ • Computer Vision Pipeline (OpenCV)     │
│ • Session Store (SQLite, WAL)           │
│ • PDF Export/Import (lopdf)             │
└─────────────────────────────────────────┘
```

## Runtime Metrics

| Metric | Value |
|--------|-------|
| Throughput | ≥ 165 pages/shift (~5h 20min operator time) |
| Pipeline speed | ≤ 150 ms per spread (capture → detection → crop → binarization → save) |
| Page size | ~80–120 KB/page (CCITT Group 4 monochrome A4/A3) |
| 400-page book | ≤ 40 MB final PDF |

## Configuration

Server and client read the **same** configuration file following XDG Base Directory Specification.

**Configuration path (priority order):**
1. `$FLAT_SCANNER_CONFIG` (environment variable, for systemd)
2. `~/.config/flat-scanner/config.toml`
3. `/etc/flat-scanner/config.toml` (system, installed by PKGBUILD)

### config.toml structure

```toml
[server]
# Bind address:
#   "127.0.0.1" — local machine only (secure by default)
#   "0.0.0.0"   — network access (for remote Flutter client)
host = "127.0.0.1"

# HTTP gateway port
port = 54321

[paths]
# Base directory for all data (~ supported)
base_dir = "~/.local/share/flat-scanner"

# Subdirectories relative to base_dir
raw_dir = "raw"
processed_dir = "processed"
export_dir = "export"
import_dir = "import"

# Database file (relative to base_dir)
database = "data.db"
```

### Directory map

| Data type | Path | Description |
|-----------|------|-------------|
| Configuration | `~/.config/flat-scanner/config.toml` | Unified config for server and client |
| Raw scans | `~/.local/share/flat-scanner/raw/` | Original TIFF from scanner (optional) |
| Processed | `~/.local/share/flat-scanner/processed/` | CCITT G4 / PNG pages |
| PDF export | `~/.local/share/flat-scanner/export/` | Final PDFs |
| PDF import | `~/.local/share/flat-scanner/import/` | Temporary import files |
| Database | `~/.local/share/flat-scanner/data.db` | SQLite sessions |

## Installation (Arch Linux)

### Server

```bash
makepkg -si flat-scanner-server/PKGBUILD
systemctl enable --now flat-scanner-server
```

### Client

```bash
makepkg -si flat-scanner-client/PKGBUILD
```

### System dependencies

```bash
sudo pacman -S opencv sane-backends libtiff poppler-utils
```

## API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/api/v1/health` | Health check |
| POST | `/api/v1/scanner/init` | Initialize scanner |
| POST | `/api/v1/scanner/process` | Capture + process spread |
| GET/POST | `/api/v1/calibration` | Get/update calibration params |
| PATCH | `/api/v1/scan/{uuid}/adjust-vertex` | Adjust page vertex |
| POST | `/api/v1/export-pdf` | Export final PDF |
| POST | `/api/v1/import-pdf` | Import external PDF |
| POST | `/api/v1/replace-pdf-page` | Replace PDF page |
| POST | `/api/v1/insert-pdf-page` | Insert PDF page |
| POST | `/api/v1/clean-pdf-page` | Clean PDF page from noise |

## Development

### Server (Rust)

```bash
cd flat-scanner-server
cargo build --release
cargo test
```

### Client (Flutter)

```bash
cd flat-scanner-client
flutter pub get
flutter run -d linux
```

## License

AGPL-3.0-only