import 'dart:convert';

import 'package:http/http.dart' as http;

import 'models.dart';

/// HTTP-клиент для Flat Scanner Server (Axum).
///
/// Адрес сервера настраивается через [host] и [port]
/// (по умолчанию `127.0.0.1:54321`).
class ApiService {
  final String host;
  final int port;
  final http.Client _client;

  ApiService({
    this.host = '127.0.0.1',
    this.port = 54321,
    http.Client? client,
  }) : _client = client ?? http.Client();

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

  void dispose() => _client.close();
}

/// Ошибка HTTP-запроса к серверу.
class ApiException implements Exception {
  final int statusCode;
  final String message;

  ApiException(this.statusCode, this.message);

  @override
  String toString() => 'ApiException($statusCode): $message';
}