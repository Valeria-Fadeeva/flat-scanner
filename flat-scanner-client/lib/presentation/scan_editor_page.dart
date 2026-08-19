import 'package:flutter/material.dart';
import 'package:flutter_bloc/flutter_bloc.dart';
import 'package:window_manager/window_manager.dart';

import '../data/api_service.dart';
import '../data/models.dart';
import '../domain/scanner_bloc.dart';
import 'pdf_import_page.dart';
import 'vertex_editor.dart';

/// Главный экран редактора сканирования.
///
/// Содержит:
/// - выбор профиля обработки (1-бит / grayscale / color);
/// - кнопку «Сканировать разворот»;
/// - отображение вершин страницы (4 угла) после обработки;
/// - кнопку экспорта финального PDF (G5);
/// - опциональный полноэкранный режим (F11 / кнопка).
class ScanEditorPage extends StatefulWidget {
  const ScanEditorPage({super.key});

  @override
  State<ScanEditorPage> createState() => _ScanEditorPageState();
}

class _ScanEditorPageState extends State<ScanEditorPage> {
  ScanProfile _profile = ScanProfile.textBw1bit;

  /// UUID последней успешно обработанной книги (для экспорта PDF).
  String? _lastUuid;

  /// Флаг: идёт экспорт PDF.
  bool _exporting = false;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);

    return Scaffold(
      appBar: AppBar(
        title: const Text('Flat Scanner — редактор сканирования'),
        actions: [
          IconButton(
            tooltip: 'Разборка стороннего PDF (G4)',
            icon: const Icon(Icons.file_open),
            onPressed: () => Navigator.of(context).push(
              MaterialPageRoute(builder: (_) => const PdfImportPage()),
            ),
          ),
          IconButton(
            tooltip: 'Экспортировать PDF',
            icon: const Icon(Icons.picture_as_pdf),
            onPressed: _lastUuid == null || _exporting ? null : _exportPdf,
          ),
          IconButton(
            tooltip: 'Полноэкранный режим (F11)',
            icon: const Icon(Icons.fullscreen),
            onPressed: _toggleFullscreen,
          ),
        ],
      ),
      body: Padding(
        padding: const EdgeInsets.all(24),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            // Выбор профиля
            Text('Профиль обработки',
                style: theme.textTheme.titleMedium),
            const SizedBox(height: 8),
            SegmentedButton<ScanProfile>(
              segments: const [
                ButtonSegment(
                  value: ScanProfile.textBw1bit,
                  label: Text('Текст 1-бит'),
                  icon: Icon(Icons.document_scanner),
                ),
                ButtonSegment(
                  value: ScanProfile.illustrationGrayscale8bit,
                  label: Text('Иллюстрация 8-бит'),
                  icon: Icon(Icons.image),
                ),
                ButtonSegment(
                  value: ScanProfile.colorRgb24bit,
                  label: Text('Цвет 24-бит'),
                  icon: Icon(Icons.palette),
                ),
              ],
              selected: {_profile},
              onSelectionChanged: (sel) =>
                  setState(() => _profile = sel.first),
            ),
            const SizedBox(height: 24),

            // Кнопка сканирования
            BlocBuilder<ScannerBloc, ScannerState>(
              builder: (context, state) {
                if (state is Scanning) {
                  return const Row(
                    children: [
                      SizedBox(
                        width: 20,
                        height: 20,
                        child: CircularProgressIndicator(strokeWidth: 2),
                      ),
                      SizedBox(width: 12),
                      Text('Идёт захват и обработка разворота…'),
                    ],
                  );
                }
                return FilledButton.icon(
                  onPressed: () => context
                      .read<ScannerBloc>()
                      .add(StartScan(profile: _profile)),
                  icon: const Icon(Icons.photo_camera),
                  label: const Text('Сканировать разворот'),
                );
              },
            ),
            const SizedBox(height: 24),

            // Результат
            Expanded(
              child: BlocBuilder<ScannerBloc, ScannerState>(
                builder: (context, state) {
                  if (state is ScanSuccess) {
                    _lastUuid = state.response.uuid;
                    return _ScanResultCard(response: state.response);
                  }
                  if (state is ScanError) {
                    return _ErrorCard(message: state.message);
                  }
                  return Center(
                    child: Text(
                      'Выберите профиль и нажмите «Сканировать разворот».',
                      style: theme.textTheme.bodyMedium,
                    ),
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }

  Future<void> _toggleFullscreen() async {
    final isFullScreen = await windowManager.isFullScreen();
    await windowManager.setFullScreen(!isFullScreen);
  }

  /// G5: Экспорт финального PDF из всех страниц книги.
  Future<void> _exportPdf() async {
    final uuid = _lastUuid;
    if (uuid == null) return;

    setState(() => _exporting = true);
    try {
      final api = context.read<ApiService>();
      final res = await api.exportPdf(uuid: uuid);
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(
          content: Text(
            'PDF сохранён: ${res.path} (${res.pageCount} стр., '
            '${(res.sizeBytes / 1024).toStringAsFixed(0)} KB)',
          ),
        ),
      );
    } catch (e) {
      if (!mounted) return;
      ScaffoldMessenger.of(context).showSnackBar(
        SnackBar(content: Text('Ошибка экспорта PDF: $e')),
      );
    } finally {
      if (mounted) setState(() => _exporting = false);
    }
  }
}

/// Карточка результата: вершины + время обработки + drag-and-drop редактор.
class _ScanResultCard extends StatefulWidget {
  final ScanResponse response;
  const _ScanResultCard({required this.response});

  @override
  State<_ScanResultCard> createState() => _ScanResultCardState();
}

class _ScanResultCardState extends State<_ScanResultCard> {
  /// Текущие вершины (обновляются при drag-and-drop).
  late PageVertices _vertices;

  @override
  void initState() {
    super.initState();
    _vertices = widget.response.vertices;
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final api = context.read<ApiService>();

    return Card(
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(Icons.check_circle, color: theme.colorScheme.primary),
                const SizedBox(width: 8),
                Text('Разворот обработан',
                    style: theme.textTheme.titleMedium),
              ],
            ),
            const SizedBox(height: 12),
            Text('UUID: ${widget.response.uuid}'),
            Text('Время обработки: ${widget.response.executionTimeMs} мс'),
            const SizedBox(height: 16),

            // G6: Drag-and-Drop редактор вершин
            Text('Корректировка вершин (перетащите маркеры):',
                style: theme.textTheme.titleSmall),
            const SizedBox(height: 8),
            SizedBox(
              height: 200,
              child: Row(
                children: [
                  Expanded(
                    child: VertexEditor(
                      uuid: widget.response.uuid,
                      vertices: _vertices,
                      page: 'left',
                      api: api,
                      onChanged: (v, idx) {
                        setState(() {
                          final list = List<PageVertex>.from(_vertices.vertices);
                          list[idx] = v;
                          _vertices = PageVertices(vertices: list);
                        });
                      },
                    ),
                  ),
                  const SizedBox(width: 16),
                  Expanded(
                    child: VertexEditor(
                      uuid: widget.response.uuid,
                      vertices: _vertices,
                      page: 'right',
                      api: api,
                      onChanged: (v, idx) {
                        setState(() {
                          final list = List<PageVertex>.from(_vertices.vertices);
                          list[idx] = v;
                          _vertices = PageVertices(vertices: list);
                        });
                      },
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(height: 12),

            // Текстовое отображение вершин
            Text('Вершины страницы:', style: theme.textTheme.titleSmall),
            const SizedBox(height: 4),
            ..._vertices.vertices.asMap().entries.map((e) => Padding(
                  padding: const EdgeInsets.only(left: 8),
                  child: Text(
                    'Угол ${e.key + 1}: (${e.value.x.toStringAsFixed(1)}, '
                    '${e.value.y.toStringAsFixed(1)})',
                  ),
                )),
          ],
        ),
      ),
    );
  }
}

/// Карточка ошибки.
class _ErrorCard extends StatelessWidget {
  final String message;
  const _ErrorCard({required this.message});

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Card(
      color: theme.colorScheme.errorContainer,
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Row(
          children: [
            Icon(Icons.error, color: theme.colorScheme.onErrorContainer),
            const SizedBox(width: 12),
            Expanded(
              child: Text(
                message,
                style: TextStyle(color: theme.colorScheme.onErrorContainer),
              ),
            ),
          ],
        ),
      ),
    );
  }
}