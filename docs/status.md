# Release status

Version **1.1.1** is prepared after published 1.1.0. Upload remains a manual step.

The release set contains Linux x86_64 (glibc 2.28+), Windows x86_64, and shared
framework/payload wheels. Android runtimes cover arm64-v8a and x86_64, API 26+,
targeting API 36. The native backend is supported; WebView remains experimental.
Compatible pure-Python dependencies are supported. No macOS wheel is included.

## Validation

The UI audit passed 219 framework tests, 74 renderer tests and 47 bridge tests.
The dependency changes passed 101 CLI tests and an additional real Requests
resolution/import test. The actual dev CLI was tested with a simulated reload
socket for dependency updates/removal and one-file source reloads.

A minimal final Moto G54 / Android 15 test verified Requests and transitive imports,
package metadata, certificate resources and hot reload through `zkite dev`.
Earlier device checks covered input selection, Unicode editing, list interaction,
navigation, splash, and repeated background/resume. See [audit](ui-audit.md).

All four wheels passed strict Twine metadata and RECORD/hash checks. Auditwheel
confirmed the Linux glibc 2.28 baseline. Fresh Linux and Windows/Wine installations
imported the framework and built standalone APKs outside the checkout.

Release packaging evidence is stored locally in `release/validation-1.1.1/`.
Native Windows and other Android versions/devices are not newly qualified by this
release; Windows packaging can be smoke-tested under Wine. PyPI upload does not
establish Play Store acceptance or support for arbitrary native-extension packages.
