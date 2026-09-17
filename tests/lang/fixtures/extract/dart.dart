// Fixture docs mention doc_only_dart and should not become code.
import 'dart:async';

abstract class GoldenRenderable {
  String render(String name);
}

class GoldenWidget implements GoldenRenderable {
  // Method docs mention doc_only_dart.
  GoldenWidget();

  @override
  String render(String name) => formatWidget(name);

  String get label => 'widget';
}

enum GoldenState { ready, spent }

GoldenWidget makeWidget(String name) {
  return GoldenWidget();
}

String formatWidget(String name) => name.trim();
