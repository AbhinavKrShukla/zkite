# Publishing ZKite to PyPI

The current prepared release is **1.1.2**, following the published **1.1.1**.
Nothing in this preparation uploads to an index. Android builds require CPython 3.14.

## Prepared upload folder: `publish-dist/1.1.2/`

This folder contains four wheels: the Linux x86_64 (manylinux 2.28) CLI, Windows
x86_64 CLI, shared `zkite-framework`, and shared `zkite-payload`. All use 1.1.2.
Upload all four wheels together. Older files directly under `publish-dist/` belong
to 1.0.0 and must not be included in this upload.

From the repository root:

```bash
uvx twine check --strict publish-dist/1.1.2/*.whl
# Optional TestPyPI rehearsal, using its separate account/token:
uvx twine upload --repository testpypi publish-dist/1.1.2/*.whl
# Real PyPI, when ready; enter your API token at Twine's prompt:
uvx twine upload --username __token__ publish-dist/1.1.2/*.whl
```

Do not upload README.md, MANIFEST.json, SHA256SUMS, or application APKs/AABs.
The manifest and checksums record the prepared wheel set; validation logs are in
`release/validation-1.1.2/`. See [release notes](CHANGELOG.md).

Both platform wheels were installed into fresh environments and used to build
an APK outside the checkout. Windows was tested under Wine; native Windows
qualification remains pending. No macOS wheel is included in this local set.

For local installation before upload:

```bash
python3.14 -m venv /tmp/zkite-install-check
/tmp/zkite-install-check/bin/python -m pip install --no-index --find-links publish-dist/1.1.2 zkite==1.1.2
```

## 1. Settle names, ownership and license

The release consists of three distributions at the **same version**:

| Distribution | Contents |
|---|---|
| `zkite` | Host-specific CLI executable; installs the other two distributions. |
| `zkite-framework` | Python API, imported as `zkite`. |
| `zkite-payload` | Host-independent prebuilt Android runtimes, Python and Dev Shells. |

The command and Python import are both `zkite`. The previous published release
is 1.1.1. Keep the CLI, framework and payload distribution versions identical:
the CLI requires exact matching framework and payload versions.
The old `kite` and `kitecli` distribution names belong to unrelated projects.

The owner selected **MIT** for ZKite's original code. `LICENSE` and
`THIRD_PARTY_NOTICES.txt` are included in all wheel metadata layouts and in the
Android payload. Generated apps and project previews also embed the notices as
assets. Dependencies retain their own licenses; the MIT declaration does not
relicense their code. Refresh notices when changing bundled components.

`scripts/collect_release_licenses.py` collects exact Cargo package notices from
`cargo metadata --locked --format-version 1` JSON files and accepts `--supplement`
files for CPython, native libraries, fonts, pip and compiler runtime notices. The
current snapshot includes these supplements; release inputs and collection logs
are retained in `release/license-inputs/` and `release/icon-validation/`.

## 2. Verify the source and device behavior

From the repository root, with CPython 3.14, Rust, JDK 17 and protoc installed:

```bash
python scripts/check_host.py --native
git diff --check
```

The provided `Host regression gates` workflow runs the host tests on Windows,
Linux and macOS. All intended host jobs must pass before advertising support.

Use a dedicated sample app on an authorized Android device:

```bash
zkite new release_check
cd release_check
zkite run --serial YOUR_DEVICE_SERIAL
zkite run --serial YOUR_DEVICE_SERIAL
zkite dev --serial YOUR_DEVICE_SERIAL --port 9000
```

Check a fresh standalone launch without a dev server, repeated updates with the
same key, retained user data, tapping controls, text input, Back, background/resume,
rotation, and `SystemBars` colors/icon contrast. For dev, check helper-module edits,
file deletion and reconnect. Also test Android 16/target SDK 36 behavior and an
actual 16 KiB page-size device or emulator before claiming that device coverage.

Keep the project's `.zkite/debug.pem` for repeat development installs. Final app
releases use their own permanent signing key. See Documentation.md for the
application-key workflow and supported dependency restrictions.

## 3. Give distributed Dev Shells a stable signing identity

The generic Dev Shell must keep the same signing key across framework releases
so an existing developer installation can be updated without an uninstall.
Generate its key once using the built CLI:

```bash
phase_1/cli/target/release/zkite keygen --out /private/path/zkite-devshell
```

Store the combined PEM as the GitHub Actions secret
`ZKITE_DEVSHELL_SIGNING_PEM`. The candidate workflow uses this secret when present.
Without it, builds are disposable test candidates signed with temporary keys.
Do not commit the private key or include it in wheels. Keep a backed-up copy.

For local runtime builds, set `ZKITE_TEST_PEM` to that same PEM file. This key is
for the generic development shell; developers choose their own app signing keys.

## 4. Build all release wheels from one commit

Push the reviewed source to your repository, then manually run the
**Build installable native candidates** workflow in
`.github/workflows/release-candidate.yml`. Enter a new version such as
`1.1.2`; use the identical version for every artifact in the release.

The workflow:

1. Fetches the pinned pack dependency and Android CPython binaries.
2. Builds the native runtime and Dev Shells for arm64-v8a and x86_64 using JDK 17.
3. Stages one shared Android payload.
4. Builds real Windows x86_64 and macOS Apple-silicon CLI binaries.
5. Builds Linux x86_64 in the manylinux 2.28 container.
6. Installs each wheel set into a fresh venv and builds an APK outside the checkout.
7. Uploads workflow artifacts. **It does not publish to an index.**

Download the wheel artifacts and collect one copy of each wheel in a clean
directory, for example `publish-dist/1.1.2/`. Take the framework and payload wheels from
the Linux job and one CLI wheel from each host job. Desktop jobs also produce
framework/payload copies for their installation tests; do not upload duplicates.
Expect three platform-specific CLI wheels plus one framework and one payload wheel.

Do not upload the local `linux_x86_64` wheel: PyPI needs the compatible manylinux
artifact from the container build. Changing a filename or passing `--platform`
alone does not make a binary compatible with that platform. Windows ARM and Intel
macOS are outside this release's build matrix.

For a local candidate, follow the contributor commands in Documentation.md. The
wheel builder's defaults already use the three ZKite distribution names.

## 5. Check metadata and Android artifacts

Use a separate publishing environment:

```bash
python3.14 -m venv .publish-venv
. .publish-venv/bin/activate
python -m pip install --upgrade pip twine
python -m twine check --strict publish-dist/1.1.2/*.whl
```

On Windows, create the environment with `py -3.14 -m venv .publish-venv` and use
`.\.publish-venv\Scripts\python.exe` instead of `python`.

Install the collected wheels on clean machines before uploading:

```bash
python -m pip install --no-index --find-links publish-dist/1.1.2 "zkite==1.1.2"
python -c "from zkite import App, SystemBars, Text, store"
zkite doctor
```

An absent phone makes device checks fail; the build itself does not require one.
Build a sample AAB with an explicit test signing key and validate its contents with
Google's [bundletool](https://github.com/google/bundletool/releases):

```bash
zkite build aab --keystore /private/path/sample-release.pem
java -jar bundletool-all.jar validate --bundle=dist/release_check.aab
```

Use `apksigner verify --verbose` on sample APKs. Test a release-signed AAB on the
intended Play testing track separately. A structurally valid AAB is not evidence
of Play acceptance or correct rendering on every device.

## 6. Upload and install from TestPyPI

Create a TestPyPI account and use a TestPyPI API token. Let Twine prompt for it;
do not put tokens into source files or terminal command arguments.

```bash
python -m twine upload --repository testpypi publish-dist/1.1.2/*.whl
```

Test from a fresh CPython 3.14 venv using **only** TestPyPI for this check:

```bash
python -m pip install --index-url https://test.pypi.org/simple/ "zkite==1.1.2"
zkite new test_install
cd test_install
zkite build apk --abi arm64-v8a
```

All three distributions must be uploaded; the CLI depends on the exact matching
framework and payload versions. Repeat installation with the Windows wheel on
Windows. TestPyPI and real PyPI use separate accounts/tokens and project ownership.
See the [official TestPyPI guide](https://packaging.python.org/en/latest/guides/using-testpypi/).

## 7. Publish the reviewed release to PyPI

After the previous checks pass, upload those same reviewed artifacts using your
real PyPI credentials:

```bash
python -m twine upload --username __token__ publish-dist/1.1.2/*.whl
```

Then verify from a clean environment:

```bash
python -m pip install --index-url https://pypi.org/simple/ "zkite==1.1.2"
zkite new hello
cd hello
zkite run
```

Use an explicit version or `--pre` only when deliberately installing a prerelease.
After this stable release is published, the normal command is `python -m pip install zkite`. The official
[packaging tutorial](https://packaging.python.org/en/latest/tutorials/packaging-projects/)
describes the Twine upload flow.

For later automation, configure a [PyPI Trusted Publisher](https://docs.pypi.org/trusted-publishers/)
for **each** distribution and a dedicated publishing workflow. The supplied
candidate workflow deliberately has no upload credentials or publishing step.

## 8. Maintain the release

Record the source commit, wheel hashes, toolchain versions and validation results.
Tag the tested source and attach release notes explaining the supported host and
Android versions. Never replace an already published version's contents: fix the
source, choose a new version, and rebuild the complete matching wheel set.

Keep [release status](docs/status.md) accurate about device and
Windows testing. Do not represent deferred advanced features as implemented.

## Local 1.1.2 signing identity

Both generic native Dev Shell ABIs use the persistent key at
`~/.local/share/zkite-release/native-devshell.pem`. Keep a secure backup; never
include this file in upload artifacts. Earlier generic test shells used disposable
keys, so they are not signature-compatible with this identity. Default project
previews are re-signed with each project's existing debug key and retain update
compatibility.
