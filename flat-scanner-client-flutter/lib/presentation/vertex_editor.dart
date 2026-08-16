import 'package:flutter/material.dart';

import '../data/api_service.dart';
import '../data/models.dart';

/// G6: Интерактивный редактор вершин страницы (drag-and-drop).
///
/// Рисует контур страницы (4 вершины) через [CustomPainter] и позволяет
/// перетаскивать каждую вершину мышью. При отпускании вершины координаты
/// отправляются на сервер через [ApiService.adjustVertex] (G2).
///
/// Координаты сервера — в пикселях исходного кадра; виджет масштабирует их
/// в область отрисовки и обратно.
class VertexEditor extends StatefulWidget {
  /// UUID книги (для PATCH-запроса).
  final String uuid;

  /// Текущие вершины страницы.
  final PageVertices vertices;

  /// Сторона: 'left' или 'right'.
  final String page;

  /// HTTP-клиент для отправки корректировки.
  final ApiService api;

  /// Вызывается после успешного обновления вершины на сервере.
  final void Function(PageVertex updated, int index)? onChanged;

  const VertexEditor({
    super.key,
    required this.uuid,
    required this.vertices,
    required this.page,
    required this.api,
    this.onChanged,
  });

  @override
  State<VertexEditor> createState() => _VertexEditorState();
}

class _VertexEditorState extends State<VertexEditor> {
  /// Индекс перетаскиваемой вершины (null — нет активного drag).
  int? _dragIndex;

  /// Локальный кэш вершин (обновляется при drag, чтобы не дёргать сеть на каждый пиксель).
  late List<PageVertex> _vertices;

  /// Границы контента в пикселях сервера (для масштабирования).
  double _minX = 0, _minY = 0, _maxX = 1, _maxY = 1;

  @override
  void initState() {
    super.initState();
    _vertices = List<PageVertex>.from(widget.vertices.vertices);
    _computeBounds();
  }

  @override
  void didUpdateWidget(VertexEditor oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.vertices != widget.vertices) {
      _vertices = List<PageVertex>.from(widget.vertices.vertices);
      _computeBounds();
    }
  }

  void _computeBounds() {
    if (_vertices.isEmpty) return;
    _minX = _vertices.map((v) => v.x).reduce((a, b) => a < b ? a : b);
    _minY = _vertices.map((v) => v.y).reduce((a, b) => a < b ? a : b);
    _maxX = _vertices.map((v) => v.x).reduce((a, b) => a > b ? a : b);
    _maxY = _vertices.map((v) => v.y).reduce((a, b) => a > b ? a : b);
    // Защита от вырожденного контура
    if (_maxX <= _minX) _maxX = _minX + 1;
    if (_maxY <= _minY) _maxY = _minY + 1;
  }

  /// Пиксель сервера → локальная координата в пикселях виджета.
  Offset _toLocal(PageVertex v, Size size) {
    final sx = size.width / (_maxX - _minX);
    final sy = size.height / (_maxY - _minY);
    return Offset((v.x - _minX) * sx, (v.y - _minY) * sy);
  }

  /// Локальная координата в пикселях виджета → пиксель сервера.
  PageVertex _toServer(Offset local, Size size) {
    final sx = (_maxX - _minX) / size.width;
    final sy = (_maxY - _minY) / size.height;
    return PageVertex(
      x: _minX + local.dx * sx,
      y: _minY + local.dy * sy,
    );
  }

  Future<void> _commitVertex(int index, PageVertex v) async {
    try {
      await widget.api.adjustVertex(
        uuid: widget.uuid,
        index: index,
        x: v.x.round(),
        y: v.y.round(),
        page: widget.page,
      );
      widget.onChanged?.call(v, index);
    } on ApiException catch (e) {
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          SnackBar(content: Text('Ошибка корректировки вершины: ${e.message}')),
        );
      }
    }
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, constraints) {
        final size = constraints.biggest;
        return GestureDetector(
          onPanStart: (d) => _onPanStart(d.localPosition, size),
          onPanUpdate: (d) => _onPanUpdate(d.localPosition, size),
          onPanEnd: (d) => _onPanEnd(d.localPosition, size),
          child: CustomPaint(
            size: size,
            painter: _VertexPainter(
              vertices: _vertices,
              toLocal: (v) => _toLocal(v, size),
              dragIndex: _dragIndex,
            ),
          ),
        );
      },
    );
  }

  void _onPanStart(Offset local, Size size) {
    final idx = _nearestVertex(local, size);
    if (idx != null) {
      setState(() => _dragIndex = idx);
    }
  }

  void _onPanUpdate(Offset local, Size size) {
    if (_dragIndex == null) return;
    final v = _toServer(local, size);
    setState(() => _vertices[_dragIndex!] = v);
  }

  void _onPanEnd(Offset local, Size size) {
    final idx = _dragIndex;
    setState(() => _dragIndex = null);
    if (idx == null) return;
    final v = _toServer(local, size);
    _commitVertex(idx, v);
  }

  /// Индекс ближайшей вершины в радиусе захвата (24px), иначе null.
  int? _nearestVertex(Offset local, Size size) {
    const grabRadius = 24.0;
    int? best;
    double bestDist = grabRadius;
    for (var i = 0; i < _vertices.length; i++) {
      final d = (_toLocal(_vertices[i], size) - local).distance;
      if (d <= bestDist) {
        bestDist = d;
        best = i;
      }
    }
    return best;
  }
}

/// Отрисовка контура страницы и маркеров вершин.
class _VertexPainter extends CustomPainter {
  final List<PageVertex> vertices;
  final Offset Function(PageVertex) toLocal;
  final int? dragIndex;

  _VertexPainter({
    required this.vertices,
    required this.toLocal,
    required this.dragIndex,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (vertices.length < 2) return;

    final points = vertices.map(toLocal).toList();

    // Контур страницы
    final path = Path()
      ..moveTo(points.first.dx, points.first.dy);
    for (final p in points.skip(1)) {
      path.lineTo(p.dx, p.dy);
    }
    path.close();

    final fill = Paint()
      ..color = Colors.blue.withValues(alpha: 0.12)
      ..style = PaintingStyle.fill;
    canvas.drawPath(path, fill);

    final stroke = Paint()
      ..color = Colors.blue
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2;
    canvas.drawPath(path, stroke);

    // Маркеры вершин
    for (var i = 0; i < points.length; i++) {
      final active = i == dragIndex;
      final radius = active ? 12.0 : 8.0;
      final marker = Paint()
        ..color = active ? Colors.orange : Colors.blue
        ..style = PaintingStyle.fill;
      canvas.drawCircle(points[i], radius, marker);

      final ring = Paint()
        ..color = Colors.white
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2;
      canvas.drawCircle(points[i], radius, ring);
    }
  }

  @override
  bool shouldRepaint(_VertexPainter oldDelegate) =>
      oldDelegate.vertices != vertices || oldDelegate.dragIndex != dragIndex;
}