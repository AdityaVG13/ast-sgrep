// Fixture docs mention doc_only_kotlin and should not become code.
import kotlin.text.trim

interface GoldenRenderable {
  fun render(name: String): String
}

class GoldenWidget {
  // Method docs mention doc_only_kotlin.
  fun render(name: String): String {
    return formatWidget(name)
  }
}

enum class GoldenState {
  READY,
  SPENT
}

fun makeWidget(name: String): GoldenWidget {
  return GoldenWidget()
}

fun formatWidget(name: String): String {
  return name.trim()
}
