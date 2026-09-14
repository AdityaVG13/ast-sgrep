// Fixture docs mention doc_only_moonbit and should not become code.
import { "moonbitlang/core/string" }

trait GoldenRenderable {
  render(self : Self, name : String) -> String
}

struct GoldenWidget {
  name : String
}

type GoldenAlias = String

enum GoldenState {
  Ready
  Spent(String)
}

fn GoldenWidget::render(self : Self, name : String) -> String {
  format_widget(name)
}

fn make_widget(name : String) -> GoldenWidget {
  let req = @core.HttpRequest(Post, "/users")
  let greeting = "widget"
  format_widget(greeting + name)
}

fn format_widget(name : String) -> String {
  fn decorate(s : String) -> String {
    s
  }
  decorate(name.trim())
}

test "golden" {
  format_widget("golden")
}
