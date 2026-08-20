import 'package:equatable/equatable.dart';

/// Вершина страницы (4 точки) — ответ сервера.
class PageVertex extends Equatable {
  final double x;
  final double y;

  const PageVertex({required this.x, required this.y});

  factory PageVertex.fromJson(Map<String, dynamic> json) => PageVertex(
        x: (json['x'] as num).toDouble(),
        y: (json['y'] as num).toDouble(),
      );

  @override
  List<Object?> get props => [x, y];
}

/// Вершины разворота (4 угла).
class PageVertices extends Equatable {
  final List<PageVertex> vertices;

  const PageVertices({required this.vertices});

  factory PageVertices.fromJson(Map<String, dynamic> json) => PageVertices(
        vertices: [
          PageVertex.fromJson(json['p1'] as Map<String, dynamic>),
          PageVertex.fromJson(json['p2'] as Map<String, dynamic>),
          PageVertex.fromJson(json['p3'] as Map<String, dynamic>),
          PageVertex.fromJson(json['p4'] as Map<String, dynamic>),
        ],
      );

  @override
  List<Object?> get props => [vertices];
}

/// Профиль обработки страницы.
enum ScanProfile {
  textBw1bit('text_bw_1bit'),
  illustrationGrayscale8bit('illustration_grayscale_8bit'),
  colorRgb24bit('color_rgb_24bit');

  const ScanProfile(this.wire);
  final String wire;

  static ScanProfile fromWire(String? wire) {
    return ScanProfile.values.firstWhere(
      (p) => p.wire == wire,
      orElse: () => ScanProfile.textBw1bit,
    );
  }
}

/// Ответ сервера на `/api/v1/scanner/process`.
class ScanResponse extends Equatable {
  final String status;
  final String uuid;
  final PageVertices vertices;
  final int executionTimeMs;

  const ScanResponse({
    required this.status,
    required this.uuid,
    required this.vertices,
    required this.executionTimeMs,
  });

  factory ScanResponse.fromJson(Map<String, dynamic> json) => ScanResponse(
        status: json['status'] as String,
        uuid: json['uuid'] as String,
        vertices: PageVertices.fromJson(json['vertices'] as Map<String, dynamic>),
        executionTimeMs: (json['execution_time_ms'] as num).toInt(),
      );

  @override
  List<Object?> get props => [status, uuid, vertices, executionTimeMs];
}