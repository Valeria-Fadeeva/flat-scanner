import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';

import '../data/api_service.dart';

/// G4: Экран разборки стороннего PDF.
///
/// Позволяет:
/// - растеризовать страницы PDF в PNG (`importPdf`);
/// - заменить дефектную страницу (`replacePdfPage`);
/// - вставить страницу (`insertPdfPage`);
/// - очистить страницу от шума (`cleanPdfPage`).
///
/// Все пути указываются на стороне сервера (headless-режим).
class PdfImportPage extends StatefulWidget {
  const PdfImportPage({super.key});

  @override
  State<PdfImportPage> createState() => _PdfImportPageState();
}

class _PdfImportPageState extends State<PdfImportPage> {
  final TextEditingController _pdfPathController = TextEditingController();
  final TextEditingController _imagePathController = TextEditingController();

  int _dpi = 300;
  bool _busy = false;

  /// Пути к растеризованным PNG-страницам (после импорта).
  List<String> _pages = const [];

  /// Текущий профиль очистки.
  final String _cleanProfile = 'text_bw_1bit';

  @override
  void dispose() {
    _pdfPathController.dispose();
    _imagePathController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Scaffold(
      appBar: AppBar(
        title: const Text('Flat Scanner — разборка PDF (G4)'),
      ),
      body: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Путь к PDF
            Text('Путь к PDF-файлу (на сервере)',
                style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            TextField(
              controller: _pdfPathController,
              decoration: const InputDecoration(
                hintText: '/home/user/books/input.pdf',
                border: OutlineInputBorder(),
              ),
            ),
            const SizedBox(height: 16),

            // DPI
            Text('Разрешение растеризации (DPI)',
                style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            DropdownButton<int>(
              value: _dpi,
              items: const [150, 200, 300, 400, 600]
                  .map((d) => DropdownMenuItem(value: d, child: Text('$d DPI')))
                  .toList(),
              onChanged: (v) => setState(() => _dpi = v ?? 300),
            ),
            const SizedBox(height: 16),

            // Кнопка импорта
            FilledButton.icon(
              onPressed: _busy ? null : _importPdf,
              icon: _busy
                  ? const SizedBox(
                      width: 16,
                      height: 16,
                      child: CircularProgressIndicator(strokeWidth: 2))
                  : const Icon(Icons.file_download),
              label: const Text('Импортировать страницы'),
            ),
            const SizedBox(height: 24),

            // Список страниц
            if (_pages.isNotEmpty) ...[
              Text('Страницы (${_pages.length})',
                  style: theme.textTheme.titleMedium),
              const SizedBox(height: 8),
              Expanded(
                child: ListView.builder(
                  itemCount: _pages.length,
                  itemBuilder: (context, i) => _PageRow(
                    index: i,
                    path: _pages[i],
                    pdfPath: _pdfPathController.text.trim(),
                    imagePathController: _imagePathController,
                    cleanProfile: _cleanProfile,
                    busy: _busy,
                    onReplace: () => _replacePage(i),
                    onInsert: () => _insertPage(i),
                    onClean: () => _cleanPage(i),
                  ),
                ),
              ),
            ] else
              Expanded(
                child: Center(
                  child: Text(
                    'Импортируйте PDF, чтобы увидеть страницы.',
                    style: theme.textTheme.bodyMedium,
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }

  Future<void> _importPdf() async {
    final pdfPath = _pdfPathController.text.trim();
    if (pdfPath.isEmpty) {
      _snack('Укажите путь к PDF-файлу.');
      return;
    }
    setState(() => _busy = true);
    try {
      final api = context.read<ApiService>();
      final res = await api.importPdf(inputPdf: pdfPath, dpi: _dpi);
      if (!mounted) return;
      setState(() => _pages = res.pages);
      _snack('Импортировано ${res.pageCount} страниц.');
    } catch (e) {
      _snack('Ошибка импорта: $e');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _replacePage(int index) async {
    final pdfPath = _pdfPathController.text.trim();
    final imagePath = _imagePathController.text.trim();
    if (imagePath.isEmpty) {
      _snack('Укажите путь к PNG-замене.');
      return;
    }
    setState(() => _busy = true);
    try {
      final api = context.read<ApiService>();
      final res = await api.replacePdfPage(
        inputPdf: pdfPath,
        pageIndex: index,
        replacementImage: imagePath,
      );
      _snack('Страница $index заменена: ${res.path}');
    } catch (e) {
      _snack('Ошибка замены: $e');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _insertPage(int index) async {
    final pdfPath = _pdfPathController.text.trim();
    final imagePath = _imagePathController.text.trim();
    if (imagePath.isEmpty) {
      _snack('Укажите путь к PNG для вставки.');
      return;
    }
    setState(() => _busy = true);
    try {
      final api = context.read<ApiService>();
      final res = await api.insertPdfPage(
        inputPdf: pdfPath,
        afterIndex: index,
        imagePath: imagePath,
      );
      _snack('Страница вставлена после $index: ${res.path}');
    } catch (e) {
      _snack('Ошибка вставки: $e');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _cleanPage(int index) async {
    final imagePath = _pages[index];
    setState(() => _busy = true);
    try {
      final api = context.read<ApiService>();
      final res = await api.cleanPdfPage(
        imagePath: imagePath,
        profile: _cleanProfile,
      );
      if (!mounted) return;
      setState(() {
        _pages[index] = res.path;
      });
      _snack('Страница $index очищена: ${res.path}');
    } catch (e) {
      _snack('Ошибка очистки: $e');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  void _snack(String msg) {
    if (!mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(msg)));
  }
}

/// Строка страницы: путь + действия (заменить / вставить / очистить).
class _PageRow extends StatelessWidget {
  final int index;
  final String path;
  final String pdfPath;
  final TextEditingController imagePathController;
  final String cleanProfile;
  final bool busy;
  final VoidCallback onReplace;
  final VoidCallback onInsert;
  final VoidCallback onClean;

  const _PageRow({
    required this.index,
    required this.path,
    required this.pdfPath,
    required this.imagePathController,
    required this.cleanProfile,
    required this.busy,
    required this.onReplace,
    required this.onInsert,
    required this.onClean,
  });

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      margin: const EdgeInsets.symmetric(vertical: 4),
      child: Padding(
        padding: const EdgeInsets.all(12),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(Icons.description, size: 18, color: theme.colorScheme.primary),
                const SizedBox(width: 8),
                Expanded(
                  child: Text(
                    'Стр. ${index + 1}: $path',
                    style: theme.textTheme.bodyMedium,
                    overflow: TextOverflow.ellipsis,
                  ),
                ),
              ],
            ),
            const SizedBox(height: 8),
            Row(
              children: [
                Expanded(
                  child: TextField(
                    controller: imagePathController,
                    decoration: const InputDecoration(
                      isDense: true,
                      hintText: 'Путь к PNG (замена/вставка)',
                      border: OutlineInputBorder(),
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                IconButton(
                  tooltip: 'Заменить страницу',
                  icon: const Icon(Icons.drive_file_rename_outline),
                  onPressed: busy ? null : onReplace,
                ),
                IconButton(
                  tooltip: 'Вставить страницу после этой',
                  icon: const Icon(Icons.playlist_add),
                  onPressed: busy ? null : onInsert,
                ),
                IconButton(
                  tooltip: 'Очистить от шума ($cleanProfile)',
                  icon: const Icon(Icons.cleaning_services),
                  onPressed: busy ? null : onClean,
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}