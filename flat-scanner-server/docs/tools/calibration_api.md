# Calibration API (G1)

## Назначение

REST-эндпоинты для чтения и обновления параметров калибровки бинаризации Сауволы в реальном времени (hot-reload). Оператор может менять `k_factor`, `window_size`, `profile` из Flutter UI без перезапуска сервера.

## Архитектура

```
Flutter UI  ──HTTP──▶  Axum Router  ──▶  CalibrationManager (global)
                              │                    │
                              │                    ├── Mutex<CalibrationParams>
                              │                    ├── mtime cache (500ms throttle)
                              │                    └── calibration.json (CWD)
                              │
                              └──▶ process_scan_frame() читает calib.get()
                                   при каждой обработке кадра
```

- `CalibrationManager` — глобальный `OnceLock`-экземпляр в `src/cv/calibration.rs`.
- `get()` перечитывает `calibration.json` при изменении `mtime` (троттлинг 500 мс).
- `save()` записывает JSON и обновляет кэш в памяти.
- `process_scan_frame()` в `main.rs` вызывает `global_calibration().get()` перед каждой обработкой — изменения применяются к следующему кадру.

## API

### `GET /api/v1/calibration`

Возвращает текущие параметры.

**Response 200:**
```json
{
  "k_factor": 0.2,
  "window_size": 15,
  "profile": "text_bw_1bit"
}
```

### `POST /api/v1/calibration`

Обновляет параметры. Валидация: `k_factor ∈ (0, 1)`, `window_size` нечётное ≥ 3.

**Request:**
```json
{
  "k_factor": 0.35,
  "window_size": 25,
  "profile": "illustration_grayscale_8bit"
}
```

**Response 200:** обновлённые параметры (те же поля).

**Response 422:**
```json
{"error": "k_factor must be in (0, 1)"}
```

## Примеры

```bash
# Получить текущие параметры
curl http://127.0.0.1:8080/api/v1/calibration

# Обновить k_factor и window_size
curl -X POST http://127.0.0.1:8080/api/v1/calibration \
  -H 'Content-Type: application/json' \
  -d '{"k_factor": 0.3, "window_size": 21, "profile": "text_bw_1bit"}'
```

## Известные ограничения

- Файл `calibration.json` пишется в CWD процесса (не в `~/.config`). Для systemd — `WorkingDirectory=` в unit-файле.
- Нет авторизации — API доступен всем, кто может достучаться до порта. Для продакшена: bind на `127.0.0.1` или добавить auth-слой.
- `profile` не валидируется на сервере — неизвестные значения падают в `ProcessingProfile::from_str_lenient` (дефолт `TextBw1bit`).