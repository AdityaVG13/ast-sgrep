# Fixture docs mention doc_only_ruby and should not become code.
require "json"

class GoldenWidget
  # Method docs mention doc_only_ruby.
  def render(name)
    format_widget(make_widget(name))
  end
end

# Function docs mention doc_only_ruby.
def make_widget(name)
  name.to_s
end

def format_widget(name)
  name.strip
end
