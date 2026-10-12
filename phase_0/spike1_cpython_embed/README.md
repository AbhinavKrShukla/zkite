# Spike 1 — Embed official CPython 3.14 for Android in a Rust `cdylib`

**Criterion (plan1.md §10):** *Embed official CPython 3.14 Android in a Rust `cdylib`; run
`print("hi")` from `android_main`; confirm it works on arm64 + x86_64 emulator.*

**Verdict: PASS on x86_64.** arm64 cross-compiles, links and passes every static check but
**could not be executed on this host** — see [arm64](#arm64) below, which is a finding
about the tooling rather than about ZKite.

```
ZKITE_SPIKE1_RESULT ok=1 platform=android stdout="hi\n" version=3.14.6 (tags/v3.14.6-dirty:c63aec6, ...)
```

Confirmed twice, in deliberately different environments:

| Where | How | Why it matters |
|---|---|---|
| `adb shell`, `/data/local/tmp` | `scripts/run_on_device.sh` | Isolates the embedding from packaging. A failure here is unambiguously about CPython. |
| Inside a real app process | `../spike2_pack_apk/scripts/run_on_device.sh` | Strictly harder: SELinux app domain, W^X, `argv[0]` is the zygote. **This is the one that found the bug.** |

## Build

```bash
../scripts/fetch_python.sh          # official python.org Android prefixes
scripts/build.sh                    # both ABIs
scripts/build.sh x86_64             # just one
scripts/run_on_device.sh            # the pass/fail check
```

Outputs land in `../artifacts/<abi>/`: `libzkitespike1.so`, `zkite-py-probe`, and the
`libpython3.14.so` they need beside them.

## Why `pyo3-ffi` and not `pyo3`

plan1.md §9 lists this as a **High** risk and prescribes the fallback:

> Spike this in **week 1** — if it resists, drop PyO3 and use raw `pyo3-ffi`/`bindgen`
> against the C API, which is more code but has no build-script magic to fight.

The build-script magic is the whole problem. `pyo3`'s build script wants to *run* a Python
interpreter to introspect the target's configuration — impossible when the target
interpreter is an Android `.so` that cannot execute on the build host. The documented
escape is `PYO3_CONFIG_FILE`, and `scripts/build.sh` writes one per ABI:

```
implementation=CPython
version=3.14
shared=true
abi3=false
lib_name=python3.14
lib_dir=<prefix>/lib
executable=
pointer_width=64
build_flags=
suppress_build_script_link_lines=false
```

With that in place there is nothing left to fight, so the cost of the fallback is nine
lines rather than the "more code" §9 budgets for. `pyo3-ffi` also happens to be the right
dependency for a framework that owns its own object model — ZKite is not binding Rust types
into Python, it is driving an interpreter.

**Recommendation for phase 1:** stay on `pyo3-ffi`. Revisit only if the widget layer wants
`#[pyclass]`, and even then only for the parts that need it.

## The finding: `PYTHONHOME` is not enough, and standalone binaries hide it

This is the substantive result of the spike, and it is worth reading before writing any
phase-1 code.

An isolated `PyConfig` — the correct choice for an interpreter embedded in someone else's
process — forces `use_environment = 0` inside `PyConfig_Read`, **regardless of what you
assigned to `config.use_environment` beforehand**. So `PYTHONHOME` is discarded.

The first version of this spike did exactly that and the standalone probe passed anyway.
CPython falls back to searching upward from `argv[0]` for a `lib/python3.14/os.py`
landmark, and `/data/local/tmp/zkite-phase0/zkite-py-probe` sits right next to
`/data/local/tmp/zkite-phase0/lib/python3.14/os.py`. Accidental success.

In the app process there is no such luck — `argv[0]` is the zygote — and startup died with:

```
Py_InitializeFromConfig failed: Failed to import encodings module
```

The fix is to set `config.home` explicitly via `PyConfig_SetBytesString`, which is the
supported replacement for the `Py_SetPythonHome` that CPython 3.13 removed. See
`set_python_home` and `boot_inner` in `src/lib.rs`.

Two lessons carry into phase 1:

1. **Set `config.home`. Never rely on the environment.** The Dev Shell will be laying the
   prefix down at a path only known at runtime, so this is not optional.
2. **A host-side or `adb shell` test is not a substitute for running in an app process.**
   The two environments differ in ways that silently mask failures.

## `print()` has to be observed, not assumed

`Py_Initialize` returning 0 proves nothing about whether `print("hi")` did anything: in an
Android app process, `write(1, ...)` goes nowhere. That is why CPython ships
`_android_support.py` at all.

So the spike installs a `sys.stdout` / `sys.stderr` tee before running the criterion, and
asserts on the captured text — `stdout == "hi\n"` — rather than on a return code. The
report also carries `sys.platform`, asserted to be `android`, which catches the
embarrassing failure mode of accidentally testing the host interpreter.

## Entry points

`src/lib.rs` exports three, for three different consumers:

| Symbol | Shape | Used by |
|---|---|---|
| `android_main` | `NativeActivity` | The literal wording of the §10 criterion. |
| `Java_dev_zkite_phase0_Native_boot` | JNI | Spike 2's dex shim — the shape the framework will actually ship. |
| `zkite_spike1_boot` | plain C | `zkite-py-probe`, the standalone prober. |

`android_main` is exported for completeness, but plan1.md §3 already concluded
`NativeActivity` is not viable for a real app (no IME support), which is why the dex shim
exists. **The JNI entry is the one that reflects the architecture.**

## arm64

Everything short of execution is verified for `aarch64-linux-android`:

```
$ readelf -d ../artifacts/arm64-v8a/libzkitespike1.so | grep NEEDED
  NEEDED  libpython3.14.so
  NEEDED  libdl.so
  NEEDED  liblog.so
  NEEDED  libc.so

$ nm -D --defined-only ../artifacts/arm64-v8a/libzkitespike1.so | grep -E 'android_main|Java_dev_zkite'
  T android_main
  T Java_dev_zkite_phase0_Native_boot
```

…plus the 16 KB page-alignment assertion in `zkite-pack`. The two ABIs are the same source
built with the same configuration against the same upstream release, so the residual risk
is low — but it is not zero and it is not measured.

**It could not be measured here.** The `arm64-v8a-34` system image downloads and unpacks
fine; the emulator refuses to boot it:

```
FATAL | Avd's CPU Architecture 'arm64' is not supported by the QEMU2 emulator on
        x86_64 host. System image must match the host architecture.
        (Android emulator 37.2.2.0)
```

Cross-architecture emulation is gone from current emulator builds, and `qemu-user` is not
a substitute — Android binaries need bionic and its linker, not glibc. **An arm64 AVD
needs an arm64 host.**

`scripts/run_on_device.sh` closes this the moment arm64 hardware is available; it detects
the device ABI itself and takes no arguments. Doing that requires an arm64 CI runner or a
physical phone, which is a procurement item with a lead time — see the phase-0 README's
recommendation.

## Known simplifications

- The stdlib is pushed as loose `.py`; no `.pyc` precompilation (plan1.md §7 step 2).
- `test/`, `idlelib`, `tkinter`, `turtledemo` and `lib2to3` are excluded from the push
  purely for speed.
- The interpreter is initialised once per process and never finalised. Fine for a spike;
  phase 1's hot reload will need a real story about sub-interpreters or re-exec.
