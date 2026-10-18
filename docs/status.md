# Release status

Version **1.1.2** is prepared after published 1.1.1. Upload remains a manual step.

The release set contains Linux x86_64 (glibc 2.28+), Windows x86_64, and shared
framework/payload wheels. Android runtimes cover arm64-v8a and x86_64, API 26+,
targeting API 36. The native backend is supported; WebView remains experimental.
Compatible pure-Python dependencies are supported. No macOS wheel is included.

## Validation

This release passed 227 framework tests, 103 CLI tests, 75 renderer tests,
50 bridge tests, 18 frame-timing tests, 41 browser checks and 12 release-tool tests.
A bridge regression checks that a tab's nested scroll view retains its viewport
and full row heights.

Moto G54 / Android 15 checks covered GPU-rendered color emoji (including flags),
wrapping and ellipsis, header taps and horizontal tab swipes, vertical scrolling,
retained scroll position after tab changes and background/resume, the new starter,
and a delayed HTTPS request completing with HTTP 200 while a separate button
remained responsive. The GPU emoji atlas and resize invalidation fixes were
found and verified on this device. Starlette 1.8.0 and Uvicorn 0.54.0 were imported
from packages resolved from pyproject.toml.

All four wheels passed strict Twine metadata and RECORD/hash checks. Auditwheel
confirmed the Linux glibc 2.28 baseline. Fresh Linux and Windows/Wine installations
imported the framework and built standalone APKs outside the checkout.

Release packaging evidence is stored locally in `release/validation-1.1.2/`.
Native Windows and other Android versions/devices are not newly qualified by this
release; Windows packaging can be smoke-tested under Wine. PyPI upload does not
establish Play Store acceptance or support for arbitrary native-extension packages.
