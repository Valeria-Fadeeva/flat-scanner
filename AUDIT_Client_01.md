# AUDIT_Client.md — Аудит клиента `flat-scanner-client`

**Дата аудита:** 19 августа 2026 г.  
**Дата исправлений:** 20 августа 2026 г.  
**Объект:** `flat-scanner-client/` (Flutter Desktop, Dart)  
**Референсы:** TECH_SPEC.md, TECH_SPEC_addon_1-4.md, TODO.md, CHANGELOG.md

---

## 1. Соответствие архитектуре (TECH_SPEC.md §7)

### 1.1. ScannerBLoC ✅
- Реализованы все 4 состояния: `ScannerInitial`, `ScannerScanning`, `ScannerSuccess`, `ScannerError`.
- Реализованы события: `StartScan`, `ResetScan`.
- Состояния/событи — `Equatable`-совместимы (корректный `props`).
- ✅ **Исправлено:** Имена состояний приведены в соответствие с TECH_SPEC (§7.1): `ScannerScanning`/`ScannerSuccess`/`ScannerError`.

### 1.2. ApiService ✅
- Все эндпоинты из TECH_SPEC реализованы: `health`, `initScanner`, `processScan`.
- Все G1-G6 эндпоинты реализованы: `/api/v1/calibration` (GET/POST), `/api/v1/scan/<uuid>/adjust-vertex` (PATCH), `/api/v1/export-pdf`, `/api/v1/import-pdf`, `/api/v1/replace-pdf-page`, `/api/v1/insert-pdf-page`, `/api/v1/clean-pdf-page`.
- `ApiException` корректно реализован с `statusCode` и `message`.

### 1.3. Модель данных ✅
- `ScanProfile` enum корректно мапит wire-строки (`text_bw_1bit`, `illustration_grayscale_8bit`, `color_rgb_24bit`).
- `PageVertex`, `PageVertices`, `ScanResponse` — все `Equatable`-совместимы.
- ✅ **Исправлено:** `PageVertices.fromJson` теперь корректно парсит формат `Map` с ключами `p1`–`p4`, соответствующий серверу.

---

## 2. Баги и синтаксические ошибки

### 2.1. `api_service.dart` — синтаксическая ошибка в `importPdf` (КРИТИЧЕСКАЯ)
**Файл:** `lib/data/api_service.dart`, строка 153.
```dart
'output_dir': ?outputDir,  // ❌ Было: Синтаксическая ошибка
```
✅ **Исправлено:**
```dart
if (outputDir != null) 'output_dir': outputDir,  // ✅
```

### 2.2. `api_service.dart` — синтаксическая ошибка в `replacePdfPage` (КРИТИЧЕСКАЯ)
**Файл:** `lib/data/api_service.dart`, строка 180.
```dart
'output_pdf': ?outputPdf,  // ❌ Было: Синтаксическая ошибка
```
✅ **Исправлено:**
```dart
if (outputPdf != null) 'output_pdf': outputPdf,  // ✅
```

### 2.3. `api_service.dart` — синтаксическая ошибка в `insertPdfPage` (КРИТИЧЕСКАЯ)
**Файл:** `lib/data/api_service.dart`, строка 207.
```dart
'output_pdf': ?outputPdf,  // ❌ Было: Синтаксическая ошибка
```
✅ **Исправлено:**
```dart
if (outputPdf != null) 'output_pdf': outputPdf,  // ✅
```

**Итого:** ✅ Все 3 критические синтаксические ошибки исправлены.

---

## 3. Безопасность и обработка ошибок

### 3.1. `ApiService.health()` — silent fail ✅
```dart
Future<bool> health() async {
  try {
    final res = await _client.get(Uri.parse('$_baseUrl/api/v1/health'));
    return res.statusCode == 200;
  } catch (_) {
    return false;
  }
}
```
Корректная обработка: при ошибке сети возвращается `false`, а не выбрасывается исключение.

### 3.2. `ApiService` — все остальные методы выбрасывают `ApiException` ✅
Каждый метод с HTTP-запросом проверяет `statusCode != 200` и выбрасывает `ApiException`.

### 3.3. `ScannerBloc` — обработка ошибок ✅
- `ApiException` перехватывается и преобразуется в `ScannerError`.
- Общий `catch` также перехватывается.
- **Замечание:** Сообщение ошибки содержит `$e` (исключение Dart), которое может быть длинным. Рекомендуется `e.toString()` или извлечение `ApiException.message`.

### 3.4. `VertexEditor._commitVertex` — обработка ошибок ✅
- `ApiException` перехватывается и показывается через `SnackBar`.
- Проверка `mounted` перед `ScaffoldMessenger` — корректна.

---

## 4. Потенциальные утечки ресурсов

### 4.1. `ApiService.dispose()` ✅
✅ **Исправлено:** Добавлен `dispose` в `RepositoryProvider<ApiService>` в `main.dart`:
```dart
RepositoryProvider<ApiService>(
  create: (_) => ApiService(),
  dispose: (api) => api.dispose(),
)
```

### 4.2. `PdfImportPage` — `TextEditingController` ✅
Контроллеры корректно диспозятся в `dispose()`.

---

## 5. Конфигурация и зависимости

### 5.1. `pubspec.yaml` — зависимости ✅
- `flutter_bloc: ^9.1.1` — актуальная версия.
- `http: ^1.6.0` — актуальная версия.
- `equatable: ^2.1.0` — актуальная версия.
- `window_manager: ^0.5.2` — актуальная версия.
- `gtk: ^2.2.0` — для Linux десктоп.
- ✅ **Исправлено:** Удалена неиспользуемая зависимость `args`.

### 5.2. `pubspec.yaml` — SDK constraint ✅
- `sdk: ^3.13.0` — соответствует требованиям.

### 5.3. `pubspec.yaml` — `publish_to: 'none'` ✅
Пакет не публикуется на pub.dev (как и указано в TECH_SPEC).

### 5.4. `pubspec.yaml` — отсутствие assets секции ✅
Приложение не использует статические ассеты (картинки, шрифты).

### 5.5. `analysis_options.yaml` — Flutter Lints ✅
Используется `flutter_lints: ^6.0.0`.

---

## 6. PKGBUILD и дистрибуция

### 6.1. `PKGBUILD` — `depends` (НИЗКАЯ)
Отсутствует `sane-backends` как runtime-зависимость (хотя она используется сервером, а не клиентом — допустимо).

### 6.2. `.desktop` файл ✅
- Корректный формат XDG Desktop Entry.
- `Exec=flat_scanner_client` — соответствует имени бинарника.
- `Icon=flat-scanner-client` — требует наличия иконки в системе.

### 6.3. `PKGBUILD` — путь к проекту ✅
✅ **Корректный:** Путь в `build()` соответствует: `flat-scanner-client`.

---

## 7. UI/UX

### 7.1. `ScanEditorPage` — навигация к PDF-экрану ✅
Кнопка в AppBar корректно навигирует на `PdfImportPage`.

### 7.2. `ScanEditorPage` — экспорт PDF ✅
- Кнопка заблокирована (`_lastUuid == null || _exporting`).
- Корректная обработка `mounted` после асинхронных операций.

### 7.3. `VertexEditor` — масштабирование координат ✅
- `_toLocal`/`_grabRadius` — корректная логика масштабирования.
- Защита от вырожденного контура (строки 76-77).

### 7.4. `ThemeService` — KDE Breeze ✅
- Парсинг `kdeglobals` — корректен.
- Fallback на Material 3 при отсутствии KDE — корректен.
- ✅ **Исправлено:** `_parseColor` теперь поддерживает RGB без `#`-префикса.

---

## 8. Соответствие TECH_SPEC_addon (Flutter)

| Требование | Статус | Комментарий |
|-----------|--------|-------------|
| C1: Генерация проекта Flutter | ✅ | `flat-scanner-client/` существует |
| C2: ScannerBLoC | ✅ | Все состояния/события реализованы |
| C3: ScanEditorPage | ✅ | Профиль, кнопка, вершины, fullscreen |
| C4: CustomPainter Drag-and-Drop | ✅ | `vertex_editor.dart` |
| G1: `/api/v1/calibration` | ✅ | `getCalibration`/`updateCalibration` |
| G2: `/api/v1/scan/<uuid>/adjust-vertex` | ✅ | `adjustVertex` |
| G4: PDF import | ✅ | `pdf_import_page.dart` + все методы |
| G5: PDF export | ✅ | `exportPdf` в `api_service.dart` |
| G6: CustomPainter | ✅ | `VertexEditor` |

---

## 9. Итоговая сводка

### Критические (блокируют сборку)
✅ **Все исправлены:**
1. **3 синтаксические ошибки** в `api_service.dart` (строки 153, 180, 207) — исправлены.

### Средние
✅ **Все исправлены:**
2. **Расхождение имён состояний BLoC** — исправлено: `ScannerScanning`/`ScannerSuccess`/`ScannerError`.
3. **Несовместимость `PageVertices`** — исправлено: `fromJson` теперь парсит `Map` с `p1`–`p4`.
4. **Утечка `ApiService.dispose()`** — исправлено: добавлен `dispose` в `RepositoryProvider`.
5. **PKGBUILD путь корректный** — `flat-scanner-client`.

### Низкие
✅ **Все исправлены:**
6. `args: ^2.7.0` — удалена неиспользуемая зависимость.
7. `_parseColor` — добавлена поддержка RGB без `#`-префикса.

---

## 10. Рекомендации

### Немедленно
- [x] Исправить 3 синтаксические ошибки в `api_service.dart` (строки 153, 180, 207).
- [x] Добавить `dispose: (api) => api.dispose()` в `RepositoryProvider` в `main.dart`.

### Перед релизом
- [x] Привести имена состояний BLoC в соответствие с TECH_SPEC.
- [x] Привести `PageVertices.fromJson` к формату `Map` с `p1`–`p4`.
- [x] Удалить неиспользуемую зависимость `args` из `pubspec.yaml`.
- [x] Исправить путь в `PKGBUILD`.
- [x] Улучшить `_parseColor` для RGB без `#`.

---

## 11. Результат проверки

**`flutter analyze`:**
- 0 ошибок
- 3 info-level lint warnings (use_null_aware_elements) — не блокируют сборку

**Статус:** ✅ Все критические и средние проблемы исправлены. Код готов к сборке.