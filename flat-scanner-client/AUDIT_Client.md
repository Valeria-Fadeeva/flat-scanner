# AUDIT_Client.md — Аудит клиента `flat-scanner-client`

**Дата аудита:** 19 августа 2026 г.  
**Объект:** `flat-scanner-client/` (Flutter Desktop, Dart)  
**Референсы:** TECH_SPEC.md, TECH_SPEC_addon_1-4.md, TODO.md, CHANGELOG.md

---

## 1. Соответствие архитектуре (TECH_SPEC.md §7)

### 1.1. ScannerBLoC ✅
- Реализованы все 4 состояния: `ScannerInitial`, `Scanning`, `ScanSuccess`, `ScanError`.
- Реализованы события: `StartScan`, `ResetScan`.
- Состояния/событи — `Equatable`-совместимы (корректный `props`).
- **Замечание:** Имена состояний в коде (`Scanning`/`ScanSuccess`/`ScanError`) отличаются от TECH_SPEC (§7.1), где указаны `ScannerScanning`/`ScannerSuccess`/`ScannerError`. Это не ошибка, но расхождение с документацией.

### 1.2. ApiService ✅
- Все эндпоинты из TECH_SPEC реализованы: `health`, `initScanner`, `processScan`.
- Все G1-G6 эндпоинты реализованы: `/api/v1/calibration` (GET/POST), `/api/v1/scan/<uuid>/adjust-vertex` (PATCH), `/api/v1/export-pdf`, `/api/v1/import-pdf`, `/api/v1/replace-pdf-page`, `/api/v1/insert-pdf-page`, `/api/v1/clean-pdf-page`.
- `ApiException` корректно реализован с `statusCode` и `message`.

### 1.3. Модель данных ✅
- `ScanProfile` enum корректно мапит wire-строки (`text_bw_1bit`, `illustration_grayscale_8bit`, `color_rgb_24bit`).
- `PageVertex`, `PageVertices`, `ScanResponse` — все `Equatable`-совместимы.
- **Замечение:** `PageVertices.fromJson` ожидает `json['vertices']` как `Map<String, dynamic>`, но в `ScanResponse.fromJson` передаётся `json['vertices'] as Map<String, dynamic>`. Если сервер возвращает массив (как в JSON-контракте TECH_SPEC §7.4: `"vertices": {"p1": {...}, ...}`), то `PageVertices` ожидает список. Это **потенциальная несовместимость** с сервером.

---

## 2. Баги и синтаксические ошибки

### 2.1. `api_service.dart` — синтаксическая ошибка в `importPdf` (КРИТИЧЕСКАЯ)
**Файл:** `lib/data/api_service.dart`, строка 153.
```dart
'output_dir': ?outputDir,  // ❌ Синтаксическая ошибка: оператор ? не применим к значению
```
**Исправление:**
```dart
if (outputDir != null) 'output_dir': outputDir,  // ✅
```

### 2.2. `api_service.dart` — синтаксическая ошибка в `replacePdfPage` (КРИТИЧЕСКАЯ)
**Файл:** `lib/data/api_service.dart`, строка 180.
```dart
'output_pdf': ?outputPdf,  // ❌
```

### 2.3. `api_service.dart` — синтаксическая ошибка в `insertPdfPage` (КРИТИЧЕСКАЯ)
**Файл:** `lib/data/api_service.dart`, строка 207.
```dart
'output_pdf': ?outputPdf,  // ❌
```

**Итого:** 3 критических синтаксических ошибки, блокирующих `flutter analyze` и `flutter build`.

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
- `ApiException` перехватывается и преобразуется в `ScanError`.
- Общий `catch` также перехватывается.
- **Замечание:** Сообщение ошибки содержит `$e` (исключение Dart), которое может быть длинным. Рекомендуется `e.toString()` или извлечение `ApiException.message`.

### 3.4. `VertexEditor._commitVertex` — обработка ошибок ✅
- `ApiException` перехватывается и показывается через `SnackBar`.
- Проверка `mounted` перед `ScaffoldMessenger` — корректна.

---

## 4. Потенциальные утечки ресурсов

### 4.1. `ApiService.dispose()` ✅
Метод `dispose()` вызывается где-то? В `main.dart` `ApiService` создаётся через `RepositoryProvider` без явного `dispose`. Это **потенциальная утечка** HTTP-клиента.

**Рекомендация:** Добавить `Dispose` в `main.dart`:
```dart
RepositoryProvider(
  create: (_) => ApiService(),
  dispose: (_, controller) => controller.dispose(),
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
- `args: ^2.7.0` — не используется в коде (возможно, оставлен от предыдущей версии).

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

### 6.1. `PKGBUILD` — пути (СРЕДНЯЯ)
**Файл:** `PKGBUILD`, строка 31 и 44.
```bash
cd "${srcdir}/flat-scanner-${pkgver}/flat-scanner-client-flutter"
```
**Проблема:** В рабочем каталоге проект называется `flat-scanner-client`, а не `flat-scanner-client-flutter`. Это приведёт к ошибке сборки, если PKGBUILD используется с актуальной структурой.

### 6.2. `PKGBUILD` — лицензия (СРЕДНЯЯ)
**Файл:** `PKGBUILD`, строка 10.
```bash
license=('MIT')
```
**Расхождение:** TECH_SPEC.md указывает AGPL-3.0-only для всех компонентов. `LICENSE` файл в корне проекта — AGPL-3.0-only. PKGBUILD указывает MIT.

### 6.3. `PKGBUILD` — `depends` (НИЗКАЯ)
Отсутствует `sane-backends` как runtime-зависимость (хотя она используется сервером, а не клиентом — допустимо).

### 6.4. `.desktop` файл ✅
- Корректный формат XDG Desktop Entry.
- `Exec=flat_scanner_client` — соответствует имени бинарника.
- `Icon=flat-scanner-client` — требует наличия иконки в системе.

---

## 7. UI/UX

### 7.1. `ScanEditorPage` — навигация к PDF-экрану ✅
Кнопка в AppBar корректно навигрует на `PdfImportPage`.

### 7.2. `ScanEditorPage` — экспорт PDF ✅
- Кнопка заблокирована (`_lastUuid == null || _exporting`).
- Корректная обработка `mounted` после асинхронных операций.

### 7.3. `VertexEditor` — масштабирование координат ✅
- `_toLocal`/`_grabRadius` — корректная логика масштабирования.
- Защита от вырожденного контура (строки 76-77).

### 7.4. `ThemeService` — KDE Breeze ✅
- Парсинг `kdeglobals` — корректен.
- Fallback на Material 3 при отсутствии KDE — корректен.
- **Замечание:** `_parseColor` поддерживает только `#RRGGBB`/`#AARRGGBB`. Некоторые KDE-темы используют RGB без `#`-префикса.

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
1. **3 синтаксические ошибки** в `api_service.dart` (строки 153, 180, 207): `?outputDir`/`?outputPdf` — оператор `?` не применим к не-null-значению. Необходимо использовать условную спред-синтаксию: `if (outputDir != null) 'output_dir': outputDir,`.

### Средние
2. **Расхождение PKGBUILD пути:** `flat-scanner-client-flutter` → `flat-scanner-client`.
3. **Расхождение PKGBUILD лицензии:** `MIT` → `AGPL-3.0-only`.
4. **Расхождение имён состояний BLoC:** `Scanning`/`ScanSuccess`/`ScanError` vs `ScannerScanning`/`ScannerSuccess`/`ScannerError` в TECH_SPEC §7.1.
5. **Возможная несовместимость `PageVertices`:** `fromJson` ожидает `Map`, но сервер может возвращать `List`.
6. **Утечка `ApiService.dispose()`:** нет `dispose` в `RepositoryProvider`.

### Низкие
7. `args: ^2.7.0` — зависимость не используется в коде.
8. `_parseColor` не поддерживает RGB без `#`-префикса.

---

## 10. Рекомендации

### Немедленно
- [ ] Исправить 3 синтаксические ошибки в `api_service.dart` (строки 153, 180, 207).
- [ ] Добавить `dispose: (_, controller) => controller.dispose()` в `RepositoryProvider` в `main.dart`.

### Перед релизом
- [ ] Исправить путь в `PKGBUILD` (`flat-scanner-client-flutter` → `flat-scanner-client`).
- [ ] Исправить лицензию в `PKGBUILD` на `('AGPL-3.0-only')`.
- [ ] Привести имена состояний BLoC в соответствие с TECH_SPEC или обновить TECH_SPEC.
- [ ] Проверить формат `vertices` от сервера (Map vs List) и привести `PageVertices.fromJson` к единому формату.
- [ ] Удалить неиспользуемую зависимость `args` из `pubspec.yaml`.