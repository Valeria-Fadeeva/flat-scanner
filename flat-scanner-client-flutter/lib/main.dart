//     Flat Scanner Client - Native Linux desktop UI built with Flutter
//     for the flat-scanner ecosystem.
//
//       Copyright (C) 2026  Valeria Fadeeva <valeria.fadeeva.me@gmail.com>

import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:window_manager/window_manager.dart';

import 'data/api_service.dart';
import 'data/theme_service.dart';
import 'domain/scanner_bloc.dart';
import 'presentation/scan_editor_page.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();

  // Инициализация window_manager (для fullscreen и управления окном)
  await windowManager.ensureInitialized();

  const windowOptions = WindowOptions(
    size: Size(1024, 768),
    minimumSize: Size(800, 600),
    center: true,
    title: 'Flat Scanner',
  );
  windowManager.waitUntilReadyToShow(windowOptions, () async {
    await windowManager.show();
    await windowManager.focus();
  });

  runApp(const FlatScannerApp());
}

/// Корневой виджет приложения.
class FlatScannerApp extends StatelessWidget {
  const FlatScannerApp({super.key});

  @override
  Widget build(BuildContext context) {
    final themeService = ThemeService();
    final isDark = themeService.isDarkMode;

    return MultiBlocProvider(
      providers: [
        RepositoryProvider<ApiService>(
          create: (_) => ApiService(),
        ),
        BlocProvider(
          create: (context) => ScannerBloc(context.read<ApiService>()),
        ),
      ],
      child: MaterialApp(
        title: 'Flat Scanner',
        debugShowCheckedModeBanner: false,
        theme: themeService.buildThemeData(dark: false),
        darkTheme: themeService.buildThemeData(dark: true),
        themeMode: isDark ? ThemeMode.dark : ThemeMode.light,
        home: const ScanEditorPage(),
      ),
    );
  }
}