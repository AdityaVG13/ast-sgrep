"""Directory-plugin entry point. Registration does no indexing, no network,
no filesystem writes."""


def register(ctx):
    # Hermes loads directory plugins under a unique package name; defer the
    # import so plain inspection tools can load this file without a package
    # context or external dependencies.
    if __package__:
        from .plugin import register as implementation
    else:
        import sys
        from pathlib import Path

        here = str(Path(__file__).resolve().parent)
        if here not in sys.path:
            sys.path.insert(0, here)
        from plugin import register as implementation
    return implementation(ctx)
