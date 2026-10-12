"""`@store`: reactive state that survives hot reload.

See ../PLAN.md §4. A store class becomes a dataclass; exactly one instance lives per
qualified name (module + class name) in a process-wide registry, created lazily. On
reload, `reload.py` decides whether to migrate that instance onto the new class
(`__class__` reassignment — identity preserved) or reset it (incompatible shape).
"""

from __future__ import annotations

import dataclasses
import inspect
from typing import Callable, TypeVar

T = TypeVar("T")

_INSTANCES: dict[str, object] = {}
_RENDER_CALLBACK: Callable[[], None] | None = None


def set_render_callback(cb: Callable[[], None] | None) -> None:
    global _RENDER_CALLBACK
    _RENDER_CALLBACK = cb


def store(cls: type[T]) -> type[T]:
    cls = dataclasses.dataclass(cls)
    qualname = f"{cls.__module__}.{cls.__qualname__}"
    cls.__zkite_store_qualname__ = qualname  # type: ignore[attr-defined]

    def update(self, **kwargs: object) -> None:
        for k, v in kwargs.items():
            setattr(self, k, v)
        if _RENDER_CALLBACK is not None:
            _RENDER_CALLBACK()

    cls.update = update  # type: ignore[attr-defined]
    return cls


def is_store(cls: object) -> bool:
    return hasattr(cls, "__zkite_store_qualname__")


def get_instance(cls: type[T]) -> T:
    qualname = getattr(cls, "__zkite_store_qualname__", None)
    if qualname is None:
        raise TypeError(f"{cls!r} is not a @store class")
    inst = _INSTANCES.get(qualname)
    if inst is None:
        inst = cls()
        _INSTANCES[qualname] = inst
    return inst  # type: ignore[return-value]


def reset_registry() -> None:
    """Test-only: drop all live store instances so tests don't leak state."""
    _INSTANCES.clear()


def signature_with_resolved_annotations(fn: Callable[..., object]):
    """`inspect.signature(fn)` with string annotations turned back into classes.

    Stores are injected by *annotation* (`def view(c: Counter)`), which means the
    annotation has to be the class and not the string `"Counter"`. It is a string
    whenever the defining module does `from __future__ import annotations` — an
    entirely ordinary thing to write, and one that would otherwise turn store
    injection off silently, surfacing as "view() parameter 'c' is not a @store
    class" about a parameter that visibly is one.

    `eval_str=True` resolves them against the function's own globals. A genuinely
    unresolvable annotation (a real forward reference to something not yet defined)
    raises, and the fallback returns the unevaluated signature rather than failing
    the render — the caller then reports the ordinary "not a @store class" error,
    which is the accurate thing to say about a name it cannot resolve.

    Shared by `app.py` (for `view()`) and `_nav.py` (for a screen) specifically so
    the claim that a screen *is* a view function stays true at this level of detail
    rather than nearly true.
    """
    try:
        return inspect.signature(fn, eval_str=True)
    except Exception:  # noqa: BLE001 - an unresolvable annotation must not kill a render
        return inspect.signature(fn)


__all__ = [
    "store",
    "is_store",
    "get_instance",
    "set_render_callback",
    "reset_registry",
    "signature_with_resolved_annotations",
]
