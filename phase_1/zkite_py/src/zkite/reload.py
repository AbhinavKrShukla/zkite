"""Re-exec a changed module and migrate `@store` instances onto it.

See ../PLAN.md §4 for the design rationale: re-exec into a *fresh* module namespace
(not `importlib.reload`, which mutates the live module in place and leaves no old
class object to diff against), then for each `@store` class already live, either
class-swap the existing instance onto the new class (same field shape -> identity and
data both survive) or drop it and let it be recreated fresh (shape changed -> that one
store resets, everything else is untouched).
"""

from __future__ import annotations

import dataclasses
import importlib.util
import sys
import types
from pathlib import Path

from . import _store as _store_mod


@dataclasses.dataclass
class ReloadResult:
    module: types.ModuleType
    migrated_stores: list[str]
    reset_stores: list[str]


def load_module(path: str | Path, module_name: str = "main") -> types.ModuleType:
    path = Path(path)
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise ImportError(f"cannot load module from {path}")
    module = importlib.util.module_from_spec(spec)

    # Register in `sys.modules` **before** executing, which is what
    # `importlib._bootstrap._load` does and what CPython's own "import a source
    # file directly" recipe shows. Skipping it looks harmless — the module object
    # is returned either way — but it means that while the user's `main.py` is
    # running, `sys.modules[__name__]` does not exist, and stdlib code is entitled
    # to assume it does.
    #
    # The bug that forced this: `@store` calls `dataclasses.dataclass(cls)`, and
    # under `from __future__ import annotations` every field annotation is a
    # *string*, so dataclasses must resolve names textually to spot `KW_ONLY`.
    # `dataclasses._is_type` then does, with no guard:
    #
    #     ns = sys.modules.get(cls.__module__).__dict__
    #
    # which is `None.__dict__` -> `AttributeError: 'NoneType' object has no
    # attribute '__dict__'`, thrown from inside CPython with nothing in the
    # traceback pointing at ZKite. Any app whose `main.py` starts with
    # `from __future__ import annotations` and declares a single `@store` could
    # not boot at all. Reported from a real app; reproduced host-side in
    # `tests/test_reload.py`.
    #
    # Restoring the previous entry on failure (rather than popping) keeps a failed
    # hot reload from also unregistering the module that was working a moment ago —
    # the same courtesy `importlib.reload` extends.
    previous = sys.modules.get(module_name)
    sys.modules[module_name] = module
    try:
        spec.loader.exec_module(module)
    except BaseException:
        if previous is not None:
            sys.modules[module_name] = previous
        else:
            sys.modules.pop(module_name, None)
        raise
    return module


def _store_classes(module: types.ModuleType) -> dict[str, type]:
    found = {}
    for obj in vars(module).values():
        if isinstance(obj, type) and _store_mod.is_store(obj):
            found[obj.__zkite_store_qualname__] = obj  # type: ignore[attr-defined]
    return found


def _same_shape(old_cls: type, new_cls: type) -> bool:
    old_fields = {(f.name, f.type) for f in dataclasses.fields(old_cls)}
    new_fields = {(f.name, f.type) for f in dataclasses.fields(new_cls)}
    return old_fields == new_fields


def _app_modules(root: Path) -> dict[str, types.ModuleType]:
    """Only imports owned by this project; never reload the framework or stdlib."""
    modules = {}
    for name, module in tuple(sys.modules.items()):
        if not isinstance(module, types.ModuleType) or name == "zkite" or name.startswith("zkite."):
            continue
        filename = vars(module).get("__file__")
        paths = [filename] if filename else vars(module).get("__path__", ())
        if any(Path(entry).resolve().is_relative_to(root) for entry in paths):
            modules[name] = module
    return modules


def reload_module(path: str | Path, module_name: str = "main") -> ReloadResult:
    root = Path(path).resolve().parent
    previous = _app_modules(root)
    # Import caches can otherwise retain deleted helpers or reuse same-size .pyc
    # edits made within a filesystem timestamp tick. Only app-local cache files
    # are removed; installed dependencies outside this project stay loaded.
    for name, module in previous.items():
        cached = vars(module).get("__cached__")
        if cached and Path(cached).resolve().is_relative_to(root):
            Path(cached).unlink(missing_ok=True)
        sys.modules.pop(name, None)
    importlib.invalidate_caches()
    instances = dict(_store_mod._INSTANCES)
    try:
        new_module = load_module(path, module_name)
    except BaseException:
        for name in _app_modules(root):
            sys.modules.pop(name, None)
        sys.modules.update(previous)
        _store_mod._INSTANCES.clear()
        _store_mod._INSTANCES.update(instances)
        raise
    # Stores may live in helpers imported as `import model`, without their class
    # ever appearing directly in main's globals. Migrate those stores as well.
    new_stores = {}
    for module in _app_modules(root).values():
        new_stores.update(_store_classes(module))

    migrated: list[str] = []
    reset: list[str] = []
    for qualname, new_cls in new_stores.items():
        old_instance = _store_mod._INSTANCES.get(qualname)
        if old_instance is None:
            continue
        old_cls = type(old_instance)
        if _same_shape(old_cls, new_cls):
            old_instance.__class__ = new_cls
            migrated.append(qualname)
        else:
            del _store_mod._INSTANCES[qualname]
            reset.append(qualname)

    # `load_module` already installed it (before executing, which is the point);
    # this line used to be the only registration and ran too late to help anything
    # executing inside the module. Kept as an assertion of intent rather than
    # re-assigning: the module that just ran is the one that stays.
    assert sys.modules.get(module_name) is new_module
    return ReloadResult(module=new_module, migrated_stores=migrated, reset_stores=reset)


__all__ = ["ReloadResult", "load_module", "reload_module"]
