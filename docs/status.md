# Release status

The current source targets **1.1.0**, following 1.0.0. The prepared wheels have
not been uploaded by the release-preparation tooling.

## Supported release set

- Linux x86_64, glibc 2.28 or newer.
- Windows x86_64; tested under Wine, native Windows qualification pending.
- Android API 26+, targeting API 36, arm64-v8a and x86_64.
- CPython 3.14 and compatible pure-Python dependencies.

The local 1.1.0 set does not include macOS wheels. Native-extension packages,
iOS support and advanced platform integrations remain outside the released scope.
The WebView backend is experimental; native is the supported default.

## Validation for 1.1.0

Host regression gates passed, including renderer and Python bridge tests. The
framework pytest suite passed 218 tests and 66 subtests. All four wheels passed
strict Twine checks and wheel integrity checks. Auditwheel confirmed Linux
manylinux_2_28_x86_64 compatibility. Fresh Linux and Windows/Wine installations
built APKs using the packaged runtime outside the checkout.

A Moto G54 running Android 15 displayed an HTTPS image and completed a requests
HTTPS call from an app built with installed 1.1.0 wheels. APK v2/v3 signatures
passed apksigner. A two-ABI AAB passed Google bundletool 1.18.3 validation.
Earlier device checks covered startup, resume, retained app data and text selection.
These checks do not establish Play acceptance or coverage of every device.

Release logs and screenshots are local build outputs under
`release/validation-1.1.0/`; they are not required to clone or build the framework.
See [publishing](../PUBLISHING.md) and [release notes](../CHANGELOG.md).
