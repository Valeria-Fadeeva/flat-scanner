# Flat Scanner Client

Десктопный клиент для сканирования книг на плоском сканере (Melawvy OS).
Написан на Flutter (Linux x64), общается с сервером `flat_scanner_server`
по HTTP.

## Возможности

- Выбор профиля обработки: текст 1-бит, иллюстрация 8-бит, цвет 24-бит.
- Запуск захвата и обработки разворота одной кнопкой.
- Отображение вершин страницы (4 угла) и времени обработки.
- Опциональный полноэкранный режим (кнопка в AppBar).
- Адаптация темы под системную тему KDE (Breeze) + Material 3.

## Архитектура

```
lib/
├── main.dart                     # Точка входа, window_manager, MultiBlocProvider
├── data/
│   ├── api_service.dart          # HTTP-клиент к flat_scanner_server
│   ├── models.dart               # ScanProfile, ScanResponse, PageVertex
│   └── theme_service.dart        # Парсинг kdeglobals → Material 3 ColorScheme
├── domain/
│   └── scanner_bloc.dart         # BLoC: StartScan → Scanning → Success/Error
└── presentation/
    └── scan_editor_page.dart     # UI: профиль, кнопка, результат, fullscreen
```

## Сборка

Требуется Flutter SDK и GTK-зависимости:

```bash
sudo apt install libgtk-3-dev libnotify-dev libuuid-dev libxkbcommon-dev
flutter pub get
flutter build linux --release
```

Бинарник: `build/linux/x64/release/bundle/flat_scanner_client`.

## Установка (Arch / Melawvy)

```bash
makepkg -si
```

## Настройка

Адрес сервера задаётся в `lib/data/api_service.dart`
(по умолчанию `http://127.0.0.1:8080`).

## Известные ограничения

- Клиент не хранит историю сессий локально (это делает сервер).
- Превью изображения разворота не отображается — только вершины и метрики.