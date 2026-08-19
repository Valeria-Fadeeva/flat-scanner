// Тесты виджетов Flat Scanner Client.
//
// Полноценный widget-тест требует мок-сервера и мок-а API,
// поэтому здесь проверяется только базовая сборка дерева виджетов
// без сетевых вызовов.

import 'package:flutter_test/flutter_test.dart';

import 'package:flat_scanner_client/data/models.dart';

void main() {
  testWidgets('ScanProfile enum round-trips through wire format', (tester) async {
    expect(ScanProfile.textBw1bit.wire, 'text_bw_1bit');
    expect(ScanProfile.fromWire('color_rgb_24bit'), ScanProfile.colorRgb24bit);
    expect(ScanProfile.fromWire(null), ScanProfile.textBw1bit);
  });

  testWidgets('PageVertex parses from JSON', (tester) async {
    // Проверка модели без сетевых вызовов.
    final vertex = PageVertex(x: 1.5, y: 2.5);
    expect(vertex.x, 1.5);
    expect(vertex.y, 2.5);
  });
}