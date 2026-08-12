"""Fixture docs mention doc_only_python and should not become code."""
from pathlib import Path

class GoldenWidget:
    """Class docs mention doc_only_python."""

    def render(self, path: Path) -> str:
        """Method docs mention doc_only_python."""
        return format_widget(make_widget(path))

def make_widget(path: Path) -> str:
    """Function docs mention doc_only_python."""
    return path.name

def format_widget(name: str) -> str:
    return name.strip()
