import 'dart:io';

import 'package:flutter/material.dart';

/// Служба адаптации темы под системную тему KDE (Breeze).
///
/// Flutter Linux рендерит через GTK-бэкенд и не читает KDE-тему напрямую.
/// Эта служба парсит `~/.config/kdeglobals` (секции `[ColorButton]`,
/// `[KDE][Contrast]`) и строит Material 3 [ColorScheme] из палитры пользователя.
///
/// Fallback: если `kdeglobals` отсутствует (не KDE) — дефолтный Material 3.
class ThemeService {
  /// Определяет, включена ли тёмная тема в системе.
  ///
  /// Читает `~/.config/kdeglobals` → `[KDE][Contrast] ColorScheme=Dark`.
  /// Если файла нет — возвращает `false` (светлая).
  bool get isDarkMode {
    final kdeglobals = _readKdeGlobals();
    if (kdeglobals == null) return false;

    final contrast = kdeglobals['KDE][Contrast'];
    final scheme = contrast?['ColorScheme'];
    return scheme == 'Dark';
  }

  /// Строит Material 3 [ColorScheme] из палитры Breeze.
  ColorScheme buildScheme({required bool dark}) {
    final colors = _readKdeGlobals();

    // Ключевые цвета Breeze из [ColorButton]
    final window = _parseColor(colors?['ColorButton']?['Window'],
        dark ? const Color(0xFF2A2E32) : const Color(0xFFFAFAFA));
    final text = _parseColor(colors?['ColorButton']?['Text'],
        dark ? const Color(0xFFEFF0F1) : const Color(0xFF232629));
    final highlight = _parseColor(colors?['ColorButton']?['Highlight'],
        dark ? const Color(0xFF2A82DA) : const Color(0xFF308CC6));
    final button = _parseColor(colors?['ColorButton']?['Button'],
        dark ? const Color(0xFF31363B) : const Color(0xFFEBEBEB));

    final brightness = dark ? Brightness.dark : Brightness.light;

    return ColorScheme.fromSeed(
      seedColor: highlight,
      brightness: brightness,
      surface: window,
      onSurface: text,
      primary: highlight,
      onPrimary: Colors.white,
      secondary: button,
    );
  }

  /// Возвращает [ThemeData] для светлой и тёмной темы.
  ThemeData buildThemeData({required bool dark}) {
    final scheme = buildScheme(dark: dark);
    return ThemeData(
      useMaterial3: true,
      colorScheme: scheme,
      visualDensity: VisualDensity.adaptivePlatformDensity,
    );
  }

  // --- Внутренние помощники ---

  /// Читает `~/.config/kdeglobals` в виде вложенного словаря
  /// `section -> key -> value`.
  Map<String, Map<String, String>>? _readKdeGlobals() {
    final home = Platform.environment['HOME'];
    if (home == null) return null;

    final file = File('$home/.config/kdeglobals');
    if (!file.existsSync()) return null;

    try {
      final lines = file.readAsLinesSync();
      final result = <String, Map<String, String>>{};
      var currentSection = '';

      for (final raw in lines) {
        final line = raw.trim();
        if (line.isEmpty || line.startsWith('#') || line.startsWith(';')) {
          continue;
        }
        if (line.startsWith('[') && line.endsWith(']')) {
          currentSection = line.substring(1, line.length - 1);
          result[currentSection] = {};
        } else if (line.contains('=') && currentSection.isNotEmpty) {
          final idx = line.indexOf('=');
          final key = line.substring(0, idx).trim();
          final value = line.substring(idx + 1).trim();
          result[currentSection]![key] = value;
        }
      }
      return result;
    } catch (_) {
      return null;
    }
  }

  /// Парсит цвет в формате `#RRGGBB` или `#AARRGGBB`.
  Color _parseColor(String? hex, Color fallback) {
    if (hex == null) return fallback;
    var value = hex.replaceAll('#', '');
    if (value.length == 6) value = 'FF$value';
    if (value.length != 8) return fallback;

    final int parsed;
    try {
      parsed = int.parse(value, radix: 16);
    } catch (_) {
      return fallback;
    }
    return Color(parsed);
  }
}