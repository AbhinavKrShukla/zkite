# ZKite

Build native Android apps in Python with reactive widgets, Tailwind-style layout,
hot reload, and standalone APK/AAB packaging. ZKite uses a Rust renderer and an
embedded CPython runtime.

This source tree targets **1.1.0**, following 1.0.0. Prepared 1.1.0 wheels await
manual publication; see [release notes](CHANGELOG.md) and [release status](docs/status.md).

With CPython 3.14 and ZKite installed:

```bash
zkite new hello
cd hello
zkite dev       # hot reload on a connected Android device
zkite run       # standalone build, install, launch
```

App settings live in `app.json`; `pyproject.toml` is optional. Use `zkite new hello .`
to scaffold the current folder. Put bundled resources in `assets/` and declare
Python dependencies in `requirements.txt` or your package manager's pyproject.

## Documentation

- [Getting started](Documentation.md): installation, first app, and build commands.
- [Detailed guide](Documentation-detailed.md): concepts, configuration, API, and troubleshooting.
- [Widget reference](docs/widgets.md) and [CLI reference](docs/cli.md).
- [Contributing](CONTRIBUTING.md): source map, setup, testing, and changes.
- [Publishing](PUBLISHING.md): release wheels and PyPI uploads.

## Source map

| Directory | Contents |
|---|---|
| `phase_0/` | Foundational style/parser crates, packaging tools, and dependency fetch scripts. |
| `phase_1/` | Rust CLI, Python framework, project templates, and development/WebView support. |
| `phase_2/` | APK/AAB bundler, bytecode/stdlib tools, and standalone shell support. |
| `phase_3/` | Native renderer, Python bridge, Android activity, images, and animation. |
| `scripts/`, `tests/`, `.github/` | Build tooling, regression checks, and CI. |

The phase names reflect development history. Earlier directories still contain
live dependencies; they are not disposable build output. Historical plans are
engineering records, not promises of supported features.

## Scope and license

The current local release set supports Linux x86_64 and Windows x86_64 hosts,
Android arm64-v8a/x86_64, and compatible pure-Python dependencies. Windows testing
used Wine; native Windows qualification is pending. Native-extension packages,
iOS, and advanced platform integrations are outside the released scope.

ZKite's original code is [MIT licensed](LICENSE). Bundled dependencies retain
their own licenses; see [third-party notices](THIRD_PARTY_NOTICES.txt).
