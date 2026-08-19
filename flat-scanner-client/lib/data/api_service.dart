import 'dart:convert';

import 'package:http/http.dart' as http;

import 'models.dart';

/// HTTP-клиент для Flat Scanner Server (Axum).
///
/// Адрес сервера настраивается через [host] и [port]
/// (по умолчанию `127.0.0.1:8080`).
class ApiService {
  final String host;
  final int port;
  final http.Client _client;

  ApiService({this.host = '127.0.0.1', this.port = 8080, http.Client? client})
    : _client = client ?? http.Client();

  String get _baseUrl => 'http://$host:$port';

  /// Проверка доступности движка.
  Future<bool> health() async {
    try {
      final res = await _client.get(Uri.parse('$_baseUrl/api/v1/health'));
      return res.statusCode == 200;
    } catch (_) {
      return false;
    }
  }

  /// Инициализация каретки сканера.
  Future<void> initScanner() async {
    final res = await _client.post(
      Uri.parse('$_baseUrl/api/v1/scanner/init'),
      headers: {'Content-Type': 'application/json'},
    );
    if (res.statusCode != 200) {
      throw ApiException(res.statusCode, 'init failed');
    }
  }

  /// Захват + обработка разворота.
  Future<ScanResponse> processScan({
    required String uuid,
    ScanProfile profile = ScanProfile.textBw1bit,
  }) async {
    final body = jsonEncode({
      'uuid': uuid,
      'threshold_preset': 0,
      'profile': profile.wire,
    });

    final res = await _client.post(
      Uri.parse('$_baseUrl/api/v1/scanner/process'),
      headers: {'Content-Type': 'application/json'},
      body: body,
    );

    if (res.statusCode != 200) {
      throw ApiException(res.statusCode, res.body);
    }

    return ScanResponse.fromJson(jsonDecode(res.body) as Map<String, dynamic>);
  }

  /// G1: Получить текущие параметры калибровки.
  Future<CalibrationParams> getCalibration() async {
    final res = await _client.get(Uri.parse('$_baseUrl/api/v1/calibration'));
    if (res.statusCode != 200) {
      throw ApiException(res.statusCode, res.body);
    }
    return CalibrationParams.fromJson(
      jsonDecode(res.body) as Map<String, dynamic>,
    );
  }

  /// G1: Обновить параметры калибровки (hot-reload на сервере).
  Future<CalibrationParams> updateCalibration(CalibrationParams params) async {
    final res = await _client.post(
      Uri.parse('$_baseUrl/api/v1/calibration'),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode(params.toJson()),
    );
    if (res.statusCode != 200) {
      throw ApiException(res.statusCode, res.body);
    }
    return CalibrationParams.fromJson(
      jsonDecode(res.body) as Map<String, dynamic>,
    );
  }

  /// G2: Корректировка вершины страницы (drag-and-drop).
  ///
  /// [uuid] — UUID книги, [index] — индекс вершины (0–3),
  /// [x]/[y] — новые координаты, [page] — 'left' или 'right'.
  Future<AdjustVertexResponse> adjustVertex({
    required String uuid,
    required int index,
    required int x,
    required int y,
    required String page,
  }) async {
    final uri = Uri.parse('$_baseUrl/api/v1/scan/$uuid/adjust-vertex').replace(
      queryParameters: {
        'index': index.toString(),
        'x': x.toString(),
        'y': y.toString(),
        'page': page,
      },
    );

    final res = await _client.patch(uri);
    if (res.statusCode != 200) {
      throw ApiException(res.statusCode, res.body);
    }
    return AdjustVertexResponse.fromJson(
      jsonDecode(res.body) as Map<String, dynamic>,
    );
  }

  /// G5: Экспорт финального PDF из всех страниц книги.
  ///
  /// [uuid] — UUID книги. Сервер собирает PDF из сохранённых страниц
  /// и возвращает путь, размер и количество страниц.
  Future<ExportPdfResponse> exportPdf({required String uuid}) async {
    final res = await _client.post(
      Uri.parse('$_baseUrl/api/v1/export-pdf'),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode({'uuid': uuid}),
    );
    if (res.statusCode != 200) {
      throw ApiException(res.statusCode, res.body);
    }
    return ExportPdfResponse.fromJson(
      jsonDecode(res.body) as Map<String, dynamic>,
    );
  }

  /// G4: Разборка стороннего PDF — растеризация страниц в PNG.
  ///
  /// [inputPdf] — путь к входному PDF на сервере, [dpi] — разрешение (по умолчанию 300).
  Future<ImportPdfResponse> importPdf({
    required String inputPdf,
    int dpi = 300,
    String? outputDir,
  }) async {
    final res = await _client.post(
      Uri.parse('$_baseUrl/api/v1/import-pdf'),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode({
        'input_pdf': inputPdf,
        'dpi': dpi,
        'output_dir': ?outputDir,
      }),
    );
    if (res.statusCode != 200) {
      throw ApiException(res.statusCode, res.body);
    }
    return ImportPdfResponse.fromJson(
      jsonDecode(res.body) as Map<String, dynamic>,
    );
  }

  /// G4: Замена дефектной страницы в PDF.
  ///
  /// [pageIndex] — индекс страницы (0-based), [replacementImage] — путь к PNG-замене.
  Future<PdfOperationResponse> replacePdfPage({
    required String inputPdf,
    required int pageIndex,
    required String replacementImage,
    String? outputPdf,
  }) async {
    final res = await _client.post(
      Uri.parse('$_baseUrl/api/v1/replace-pdf-page'),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode({
        'input_pdf': inputPdf,
        'page_index': pageIndex,
        'replacement_image': replacementImage,
        'output_pdf': ?outputPdf,
      }),
    );
    if (res.statusCode != 200) {
      throw ApiException(res.statusCode, res.body);
    }
    return PdfOperationResponse.fromJson(
      jsonDecode(res.body) as Map<String, dynamic>,
    );
  }

  /// G4: Вставка страницы в PDF.
  ///
  /// [afterIndex] — индекс после которого вставить (-1 = в начало).
  Future<PdfOperationResponse> insertPdfPage({
    required String inputPdf,
    required int afterIndex,
    required String imagePath,
    String? outputPdf,
  }) async {
    final res = await _client.post(
      Uri.parse('$_baseUrl/api/v1/insert-pdf-page'),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode({
        'input_pdf': inputPdf,
        'after_index': afterIndex,
        'image_path': imagePath,
        'output_pdf': ?outputPdf,
      }),
    );
    if (res.statusCode != 200) {
      throw ApiException(res.statusCode, res.body);
    }
    return PdfOperationResponse.fromJson(
      jsonDecode(res.body) as Map<String, dynamic>,
    );
  }

  /// G4: Очистка страницы от шума (профилирование).
  ///
  /// [profile] — text_bw_1bit | illustration_grayscale_8bit | color_rgb_24bit.
  Future<PdfOperationResponse> cleanPdfPage({
    required String imagePath,
    required String profile,
    double kFactor = 0.2,
    int windowSize = 15,
  }) async {
    final res = await _client.post(
      Uri.parse('$_baseUrl/api/v1/clean-pdf-page'),
      headers: {'Content-Type': 'application/json'},
      body: jsonEncode({
        'image_path': imagePath,
        'profile': profile,
        'k_factor': kFactor,
        'window_size': windowSize,
      }),
    );
    if (res.statusCode != 200) {
      throw ApiException(res.statusCode, res.body);
    }
    return PdfOperationResponse.fromJson(
      jsonDecode(res.body) as Map<String, dynamic>,
    );
  }

  void dispose() => _client.close();
}

/// G4: Ответ импорта PDF (растеризованные страницы).
class ImportPdfResponse {
  /// Пути к экспортированным PNG-страницам.
  final List<String> pages;

  /// Количество страниц.
  final int pageCount;

  ImportPdfResponse({required this.pages, required this.pageCount});

  factory ImportPdfResponse.fromJson(Map<String, dynamic> json) =>
      ImportPdfResponse(
        pages: (json['pages'] as List).cast<String>(),
        pageCount: json['page_count'] as int,
      );
}

/// G4: Ответ операции с PDF (замена/вставка/очистка).
class PdfOperationResponse {
  /// Путь к результату (PDF или очищенный PNG).
  final String path;

  /// Размер в байтах.
  final int sizeBytes;

  PdfOperationResponse({required this.path, required this.sizeBytes});

  factory PdfOperationResponse.fromJson(Map<String, dynamic> json) =>
      PdfOperationResponse(
        path: json['path'] as String,
        sizeBytes: json['size_bytes'] as int,
      );
}

/// G5: Ответ экспорта PDF.
class ExportPdfResponse {
  /// Путь к созданному PDF-файлу.
  final String path;

  /// Размер PDF в байтах.
  final int sizeBytes;

  /// Количество страниц.
  final int pageCount;

  ExportPdfResponse({
    required this.path,
    required this.sizeBytes,
    required this.pageCount,
  });

  factory ExportPdfResponse.fromJson(Map<String, dynamic> json) =>
      ExportPdfResponse(
        path: json['path'] as String,
        sizeBytes: json['size_bytes'] as int,
        pageCount: json['page_count'] as int,
      );
}

/// Параметры калибровки бинаризации Сауволы (G1).
class CalibrationParams {
  /// Коэффициент Сауволы (0.1–0.5).
  final double kFactor;

  /// Размер окна Сауволы (нечётное, 11–51).
  final int windowSize;

  /// Профиль обработки: text_bw_1bit | illustration_grayscale_8bit | color_rgb_24bit.
  final String profile;

  CalibrationParams({
    this.kFactor = 0.2,
    this.windowSize = 15,
    this.profile = 'text_bw_1bit',
  });

  factory CalibrationParams.fromJson(Map<String, dynamic> json) =>
      CalibrationParams(
        kFactor: (json['k_factor'] as num).toDouble(),
        windowSize: json['window_size'] as int,
        profile: json['profile'] as String,
      );

  Map<String, dynamic> toJson() => {
    'k_factor': kFactor,
    'window_size': windowSize,
    'profile': profile,
  };
}

/// G2: Ответ корректировки вершины.
class AdjustVertexResponse {
  final Map<String, dynamic> vertices;
  final int index;
  final String page;

  AdjustVertexResponse({
    required this.vertices,
    required this.index,
    required this.page,
  });

  factory AdjustVertexResponse.fromJson(Map<String, dynamic> json) =>
      AdjustVertexResponse(
        vertices: json['vertices'] as Map<String, dynamic>,
        index: json['index'] as int,
        page: json['page'] as String,
      );
}

/// Ошибка HTTP-запроса к серверу.
class ApiException implements Exception {
  final int statusCode;
  final String message;

  ApiException(this.statusCode, this.message);

  @override
  String toString() => 'ApiException($statusCode): $message';
}
