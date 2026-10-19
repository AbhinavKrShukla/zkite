"""Static ZKite permission analysis. Never imports or executes application code.

stdin: {"files": {"relative.py": "source"}, "capabilities": [...]}
stdout: {"permissions": [...], "reasons": {...}, "warnings": [...]}
"""
import ast
import json
import sys

CAPABILITIES = {
    "files": [], "camera.capture": [], "camera.torch": ["CAMERA"],
    "audio.record": ["RECORD_AUDIO"], "audio.play": [], "haptics": ["VIBRATE"],
    "sensors": [], "share": [], "clipboard": [], "device": [], "secrets": [],
    "biometrics": ["USE_BIOMETRIC"], "network": ["INTERNET", "ACCESS_NETWORK_STATE"],
    "location": ["ACCESS_COARSE_LOCATION", "ACCESS_FINE_LOCATION"],
    "notifications": ["POST_NOTIFICATIONS"],
}
OPERATIONS = {
    "zkite.camera.torch": ["CAMERA"],
    "zkite.audio.record": ["RECORD_AUDIO"],
    "zkite.haptics.vibrate": ["VIBRATE"], "zkite.haptics.stop": ["VIBRATE"],
    "zkite.network.upload": ["INTERNET"], "zkite.device.network": ["ACCESS_NETWORK_STATE"],
    "zkite.location.current": ["ACCESS_COARSE_LOCATION"],
    "zkite.notifications.show": ["POST_NOTIFICATIONS"],
    "zkite.biometrics.authenticate": ["USE_BIOMETRIC"],
    "urllib.request.urlopen": ["INTERNET"], "urllib.request.urlretrieve": ["INTERNET"],
    "requests.get": ["INTERNET"], "requests.post": ["INTERNET"], "requests.request": ["INTERNET"],
    "httpx.get": ["INTERNET"], "httpx.post": ["INTERNET"], "httpx.AsyncClient": ["INTERNET"],
    "httpx.Client": ["INTERNET"], "requests.Session": ["INTERNET"],
    "aiohttp.ClientSession": ["INTERNET"], "socket.socket": ["INTERNET"],
    "socket.create_connection": ["INTERNET"], "http.client.HTTPConnection": ["INTERNET"],
    "http.client.HTTPSConnection": ["INTERNET"],
}
MODULE_EXPORTS = {
    "zkite": ["audio", "camera", "files", "sensors", "haptics", "network", "device", "location", "notifications", "biometrics", "permissions", "Image"],
    "zkite.audio": ["record", "play", "speak"], "zkite.camera": ["capture", "record_video", "torch"],
    "zkite.haptics": ["perform", "vibrate", "stop"], "zkite.location": ["current"],
    "zkite.network": ["upload"], "zkite.notifications": ["show", "cancel"],
    "zkite.biometrics": ["authenticate"], "zkite.device": ["network", "info", "battery"],
    "zkite.permissions": ["ask", "status", "open_settings"],
}


def analyze(files, capabilities=()):
    trees = {}
    modules = {}
    symbols = {}
    reasons = {}
    warnings = set()
    def add(permission, reason):
        permission = permission.removeprefix("android.permission.")
        if not permission or any(not (c.isalnum() or c in "._") for c in permission):
            raise ValueError(f"Invalid permission name: {permission!r}")
        reasons.setdefault(permission, set()).add(reason)
    for capability in capabilities:
        if capability not in CAPABILITIES:
            raise ValueError(f"Unknown Android capability: {capability}")
        for permission in CAPABILITIES[capability]:
            add(permission, f"android.capabilities: {capability}")
    for path, source in files.items():
        trees[path] = ast.parse(source, filename=path)
        module = path.removesuffix(".py").replace("/", ".")
        if module.endswith(".__init__"):
            module = module[:-9]
        modules[path] = module
        symbols[module] = {}

    def resolve(node, table):
        if isinstance(node, ast.Name):
            return table.get(node.id, "")
        if isinstance(node, ast.Attribute):
            base = resolve(node.value, table)
            return base + "." + node.attr if base else ""
        if isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and node.func.id == "getattr" and len(node.args) >= 2 and isinstance(node.args[1], ast.Constant) and isinstance(node.args[1].value, str):
            base = resolve(node.args[0], table)
            return base + "." + node.args[1].value if base else ""
        return ""

    def expand(name):
        # Resolve local re-exports, with a finite cycle bound.
        for _ in range(20):
            replacement = name
            for module in sorted(symbols, key=len, reverse=True):
                if name.startswith(module + "."):
                    rest = name[len(module)+1:]
                    first, _, tail = rest.partition(".")
                    if first in symbols[module]:
                        replacement = symbols[module][first] + (("." + tail) if tail else "")
                        break
            if replacement == name:
                return name
            name = replacement
        return name

    def scope_nodes(root):
        for child in ast.iter_child_nodes(root):
            yield child
            if not isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef, ast.Lambda)):
                yield from scope_nodes(child)

    for _ in range(8):
        changed = False
        for path, tree in trees.items():
            module = modules[path]
            table = symbols[module]
            before = dict(table)
            for node in scope_nodes(tree):
                if isinstance(node, ast.Import):
                    for alias in node.names:
                        table[alias.asname or alias.name.split('.')[0]] = alias.name if alias.asname else alias.name.split('.')[0]
                elif isinstance(node, ast.ImportFrom):
                    base = node.module or ""
                    if node.level:
                        parent = module.split('.') if path.endswith('/__init__.py') else module.split('.')[:-1]
                        base = '.'.join(parent[:len(parent)-node.level+1] + ([base] if base else []))
                    for alias in node.names:
                        if alias.name == "*":
                            for name in MODULE_EXPORTS.get(base, symbols.get(base, {})):
                                table[name] = base + "." + name
                        else:
                            table[alias.asname or alias.name] = base + "." + alias.name
                elif isinstance(node, (ast.Assign, ast.AnnAssign)):
                    value = resolve(node.value, table)
                    if not value and isinstance(node.value, ast.Call):
                        target = expand(resolve(node.value.func, table))
                        if target in ("importlib.import_module", "__import__") and node.value.args and isinstance(node.value.args[0], ast.Constant):
                            value = node.value.args[0].value
                    targets = node.targets if isinstance(node, ast.Assign) else [node.target]
                    if isinstance(value, str) and value:
                        for target in targets:
                            if isinstance(target, ast.Name): table[target.id] = expand(value)
            changed |= before != table
        if not changed: break

    def scoped_nodes(root, inherited):
        table = dict(inherited)
        nodes = list(scope_nodes(root))
        for node in nodes:
            if isinstance(node, ast.Import):
                for alias in node.names:
                    table[alias.asname or alias.name.split('.')[0]] = alias.name if alias.asname else alias.name.split('.')[0]
            elif isinstance(node, ast.ImportFrom) and node.level == 0:
                for alias in node.names:
                    if alias.name == "*":
                        for name in MODULE_EXPORTS.get(node.module, ()):
                            table[name] = node.module + "." + name
                    else:
                        table[alias.asname or alias.name] = (node.module or "") + "." + alias.name
            elif isinstance(node, (ast.Assign, ast.AnnAssign)):
                value = expand(resolve(node.value, table))
                targets = node.targets if isinstance(node, ast.Assign) else [node.target]
                if value:
                    for target in targets:
                        if isinstance(target, ast.Name): table[target.id] = value
        for node in nodes:
            yield node, table
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef, ast.Lambda)):
                yield from scoped_nodes(node, table)

    for path, tree in trees.items():
        for node, table in scoped_nodes(tree, symbols[modules[path]]):
            if isinstance(node, (ast.Name, ast.Attribute)):
                target = expand(resolve(node, table))
                for permission in OPERATIONS.get(target, []):
                    add(permission, f"{path}:{node.lineno} ({target})")
            if not isinstance(node, ast.Call): continue
            target = expand(resolve(node.func, table))
            where = f"{path}:{node.lineno} ({target or 'dynamic call'})"
            for permission in OPERATIONS.get(target, []):
                add(permission, where)
            if target == "zkite.location.current":
                precise = next((k.value for k in node.keywords if k.arg == "high_accuracy"), None)
                if precise is not None and not (isinstance(precise, ast.Constant) and precise.value is False):
                    add("ACCESS_FINE_LOCATION", where)
            if target == "zkite.permissions.ask":
                name = node.args[0] if node.args else next((k.value for k in node.keywords if k.arg == "permission"), None)
                if isinstance(name, ast.Constant) and isinstance(name.value, str): add(name.value, where)
                else: warnings.add(f"{where}: dynamic permission name; declare it in android.permissions")
            if target == "zkite.Image":
                src = next((k.value for k in node.keywords if k.arg == "src"), None)
                if isinstance(src, ast.Constant) and isinstance(src.value, str):
                    if src.value.startswith(("https://", "http://")): add("INTERNET", where)
                elif src is not None:
                    # Dynamic image may be a URL: INTERNET is normal, with no prompt.
                    add("INTERNET", where + " dynamic image source")
            if isinstance(node.func, ast.Name) and node.func.id in ("eval", "exec", "__import__"):
                warnings.add(f"{path}:{node.lineno}: dynamic code/import; declare capabilities for device calls it loads")
            if isinstance(node.func, ast.Name) and node.func.id == "getattr" and node.args:
                base = expand(resolve(node.args[0], table))
                if base.startswith("zkite.") and (len(node.args)<2 or not isinstance(node.args[1], ast.Constant)):
                    warnings.add(f"{path}:{node.lineno}: dynamic ZKite lookup; declare android.capabilities")
    return {"permissions": sorted(reasons), "reasons": {p: sorted(r) for p,r in sorted(reasons.items())}, "warnings": sorted(warnings)}


if __name__ == "__main__":
    payload = json.load(sys.stdin)
    try:
        print(json.dumps(analyze(payload["files"], payload.get("capabilities", []))))
    except (SyntaxError, ValueError) as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)
