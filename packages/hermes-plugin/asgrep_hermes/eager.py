"""Eager-tools installation (the proven hermes-genome host pattern).

Hermes defers every plugin tool behind the ``tool_search`` / ``tool_describe`` /
``tool_call`` bridge and omits plugin toolsets from coding focus posture and
delegated children. While ``enabled()``, this module keeps the asgrep tools
out of deferral and present in the coding/file toolsets by wrapping three host
seams, identity-marked and idempotently chainable with other plugins doing the
same (each wrapper honors its own marker). Any seam that cannot be wrapped is
skipped silently — the failure mode is benign degradation to the bridged
(slower but functional) surface.
"""

from __future__ import annotations

import contextlib
import importlib
from typing import Callable, Optional

_EAGER_TOOLSETS = frozenset(
    {"file", "coding", "hermes-cli", "hermes-acp", "hermes-api-server"}
)
_MARKER = "_asgrep_eager"


def install_eager_tools(names, enabled: Callable[[], bool]) -> Optional[Callable[[], None]]:
    """Keep ``names`` eager while ``enabled()``. Returns an undo, or None when
    no host seam could be wrapped (degrade to bridged)."""
    names = frozenset(names)
    ordered = tuple(sorted(names))

    module = None
    original = None
    wrapped = None
    with contextlib.suppress(Exception):
        module = importlib.import_module("tools.tool_search")
        original = getattr(module, "is_deferrable_tool_name", None)
        if getattr(original, _MARKER, False):
            return None
        if callable(original):
            def wrapped(name, *args, **kwargs):  # noqa: F811
                if name in names and enabled():
                    return False
                return original(name, *args, **kwargs)
            setattr(wrapped, _MARKER, True)
            module.is_deferrable_tool_name = wrapped

    registry = None
    orig_for_toolset = None
    had_own_for_toolset = False
    for_toolset = None
    with contextlib.suppress(Exception):
        registry = importlib.import_module("tools.registry").registry
        orig_for_toolset = getattr(registry, "get_tool_names_for_toolset", None)
        if getattr(orig_for_toolset, _MARKER, False):
            return None
        if callable(orig_for_toolset):
            had_own_for_toolset = "get_tool_names_for_toolset" in registry.__dict__

            def for_toolset(target: str, *args, **kwargs):
                out = list(orig_for_toolset(target, *args, **kwargs))
                if target in _EAGER_TOOLSETS and enabled():
                    for item in ordered:
                        if item not in out:
                            out.append(item)
                return out
            setattr(for_toolset, _MARKER, True)
            registry.get_tool_names_for_toolset = for_toolset
            if isinstance(getattr(registry, "_generation", None), int):
                registry._generation += 1

    if wrapped is None and for_toolset is None:
        return None

    toolsets_mod = None
    orig_reg_gen = None
    reg_gen = None
    with contextlib.suppress(Exception):
        toolsets_mod = importlib.import_module("toolsets")
        orig_reg_gen = getattr(toolsets_mod, "_registry_generation", None)
        if callable(orig_reg_gen) and not getattr(orig_reg_gen, _MARKER, False):
            def reg_gen():  # noqa: F811
                gid, gnum = orig_reg_gen()
                return (gid, gnum * 2 + (1 if enabled() else 0))
            setattr(reg_gen, _MARKER, True)
            toolsets_mod._registry_generation = reg_gen

    def undo() -> None:
        if module is not None and wrapped is not None \
                and module.__dict__.get("is_deferrable_tool_name") is wrapped:
            module.is_deferrable_tool_name = original
        if registry is not None and for_toolset is not None \
                and registry.__dict__.get("get_tool_names_for_toolset") is for_toolset:
            if had_own_for_toolset:
                registry.get_tool_names_for_toolset = orig_for_toolset
            else:
                del registry.get_tool_names_for_toolset
            if isinstance(getattr(registry, "_generation", None), int):
                registry._generation += 1
        if toolsets_mod is not None and reg_gen is not None \
                and toolsets_mod.__dict__.get("_registry_generation") is reg_gen:
            toolsets_mod._registry_generation = orig_reg_gen

    return undo
