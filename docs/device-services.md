# Device services in ZKite 1.2

The native Android backend exposes device operations as ordinary Python modules.
App authors need no Java, Gradle, Android SDK, or JNI code. Importing the modules on
a desktop is safe; calling an Android-only operation raises `DeviceUnavailable`.
The experimental WebView backend does not implement the device bridge.

## Automatic permissions

`zkite build apk`, `zkite build aab`, and `zkite run` analyze your project's Python
source **without executing it**, then update `app.json` before packaging. Preview
preparation also uses this scan. For example:

```python
from zkite import audio, haptics

async def start_recording():
    recording = await audio.record()
    await haptics.vibrate(duration_ms=80)
    return recording
```

The build adds:

```json
{
  "android": {
    "permissions": ["RECORD_AUDIO", "VIBRATE"],
    "inferred_permissions": ["RECORD_AUDIO", "VIBRATE"]
  }
}
```

These keys are merged into your existing configuration; identity, icon, version,
and manual permissions are preserved. `inferred_permissions` records ownership:
when the last detected use disappears, a later build removes only the permission
that ZKite added. To retain one manually, keep it in `permissions` and remove it
from `inferred_permissions`. Commit app.json so permission changes are reviewable.
A failed parse leaves app.json unchanged. Concurrent edits cause an explicit error
rather than overwriting the edit. Repeating a scan with no changes does not rewrite
app.json. Existing legacy pyproject identity is preserved when app.json is created.

The build prints reasons and writes `.zkite/permissions.json`. It detects ordinary
imports, aliases, local re-exports, function references, literal `getattr`, nested
imports, remote Image sources, and common Python HTTP clients. It scans project
Python files, excluding build output, environments, hidden directories and symlinks.
It is conservative about code in branches and functions: it does not prove runtime
reachability. It cannot prove the behavior of arbitrary dynamically generated code,
third-party packages, monkey-patching, or values passed through unknown objects.
Declare permissions or capabilities explicitly for those cases. Dynamic lookups
and dynamic permission names generate diagnostics where recognizable.

```json
{
  "android": {
    "capabilities": ["audio.record", "location", "notifications"],
    "permissions": ["INTERNET"]
  }
}
```

Capabilities are optional. Supported names: `files`, `camera.capture`,
`camera.torch`, `audio.record`, `audio.play`, `haptics`, `sensors`, `share`,
`clipboard`, `device`, `network`, `location`, `notifications`, `secrets`,
`biometrics`. Explicit capabilities also participate in permission inference.

File/photo selection, external camera capture, and semantic UI haptics do not
inherently need broad storage/CAMERA/VIBRATE permissions respectively. The scanner
only requests their more privileged variants when used. Location defaults to coarse
permission; `high_accuracy=True` or a dynamic accuracy argument adds fine permission.
Android may still grant only approximate location. INTERNET and other normal
permissions have no runtime prompt; microphone/camera/location/notifications ask
in context. Denial is an ordinary Python exception, not an app crash.

Every generated APK/AAB is checked after signing: ZKite reads its compiled manifest,
checks that `uses-permission` entries are direct children of `manifest`, verifies
`android:name` namespace/resource encoding, rejects duplicates, and compares the
exact permission set with the resolved configuration. AAB checks inspect
`base/manifest/AndroidManifest.xml`; APK checks inspect `AndroidManifest.xml`.
This uses built-in Rust parsing, so app developers need no Android tooling.
Camera, autofocus, microphone and location hardware are marked optional to avoid
unintended permission-derived device filtering.

These checks verify packaging, not Play approval. Your app's actual use, privacy
policy, Data safety answers, and any applicable Play declarations must match.
See [Android manifest permissions](https://developer.android.com/guide/topics/manifest/uses-permission-element)
and [Google Play permission policy](https://support.google.com/googleplay/android-developer/answer/16558241).

## Async handlers and errors

Use `async def` handlers for device operations. They run on ZKite's background async
loop, so permission dialogs and file pickers do not block rendering. `state.update`
is atomic with respect to rendering. Keep CPU-heavy and blocking Python work out of
async handlers; use Task or `asyncio.to_thread` for that work.

```python
from zkite import Button, Column, Text, store
from zkite import files
from zkite.errors import DeviceError

@store
class Form:
    name: str = "No file selected"
    busy: bool = False

def view(s: Form):
    async def choose():
        s.update(busy=True)
        try:
            file = await files.pick(types=["application/pdf", "image/*"])
            if file is not None:
                s.update(name=file.name)
        except DeviceError as exc:
            s.update(name=str(exc))
        finally:
            s.update(busy=False)
    return Column("flex-1 p-6 gap-4",
        Button("p-4 bg-indigo-600 text-white", "Attach", on_tap=choose, disabled=s.busy),
        Text("", s.name))
```

User picker cancellation returns `None` (`[]` for pick_many). Programmatic
cancellation uses `asyncio.CancelledError`; deadlines use `TimeoutError`.
Device failures derive from `DeviceError`: `PermissionDenied`,
`PermissionNotDeclared`, `UnsupportedFeature`, `ResourceBusy`, `DeviceUnavailable`,
`OperationInterrupted`, `DeviceIOError`, and `RuntimeMismatch`.
Uncaught async errors use the normal development traceback/release generic overlay.
Catch expected permission/provider errors and offer recovery in your own UI.

There are at most 64 event coroutines, 32 pending Android requests, 16 sensor
subscriptions, and 8 audio handles. An already-open system dialog rejects another
interactive request with ResourceBusy. Cancellation discards late results; it cannot
undo an external action that already happened or forcibly dismiss every system UI.
A reload invalidates pending work and closes native resources. Old cancelled handlers
cannot update stores through state.update. Activity replacement cancels the prior
app's async work; process death does not preserve Python coroutine stacks.

## Files, export, and upload

```python
from zkite import files, network

file = await files.pick(types=["application/pdf"])
photos = await files.pick_many(types=["image/*"], limit=10)
photo = await files.pick_photo()

if file:
    await file.copy_to(files.data_dir() / "document.pdf")
    with await file.local_copy() as local:
        # local.path is an actual pathlib.Path; do blocking I/O in a worker.
        print(local.path)
    await files.save(file, suggested_name="export.pdf", mime_type="application/pdf")
    response = await network.upload("https://your-service.example/upload", file)
    print(response.status)
```

`FileRef` has name, mime_type, optional size, uri, and optional path. A provider URI
is not a filesystem path and may refer to cloud content. `copy_to` streams with a
default 256 MiB limit and replaces the destination only after a successful copy.
`local_copy` returns a context manager whose temporary directory is deleted on exit.
`FileRef.open("rb")` only supports local captures; materialize provider files first.
`persist()` requests persistent read access if supported; `release()` gives it up.
Neither prevents the provider from deleting the document. The provider must expose
an openable stream; virtual documents without one are not supported.

`files.data_dir()` is durable app-private storage. `files.cache_dir()` is disposable.
Captures and recordings initially live in cache; copy anything important into
`data_dir`. Android can reclaim cache. Export can leave a partial destination if an
external provider fails after writing starts; app-local copy uses atomic replacement.

Photo Picker is used on API 33+, with the document picker on older Android versions.
No all-files storage permission is added. App-private sharing uses a non-exported
provider with per-URI grants and a narrowly scoped cache directory.

`network.upload` streams multipart POST with a fixed memory buffer, TLS verification,
size limits, timeout, optional fields/headers and throttled `on_progress(sent,total)`.
It supports at most four concurrent uploads. It does not retry or follow redirects;
non-2xx status raises `network.HTTPError` with `.response`. HTTP requires explicit
`allow_http=True` for development. Cancellation is cooperative between chunks;
already transmitted bytes and remote side effects cannot be undone.

## Camera and microphone

```python
from zkite import camera, audio

photo = await camera.capture()                  # external camera UI
video = await camera.record_video(duration=30)  # external camera UI
await camera.torch(True)
await camera.torch(False)

recording = await audio.record(max_seconds=60)  # AAC/M4A
clip = await recording.stop()                   # FileRef
# Or await recording.cancel() to discard it.
```

External capture uses a full-resolution output file, not just a thumbnail. Camera
app behavior and capture quality depend on the device. Recording stops when the
app leaves the foreground; `stop()` retrieves a successfully finalized recording.
Very short/failed recordings raise a DeviceError. Camera and microphone hardware,
privacy switches and resource contention can make an operation unavailable even
after permission was granted. Torch turns off when backgrounded.

Embedded CameraPreview, raw PCM/frame streams, image orientation/thumbnail helpers,
and background recording are not included in 1.2.0.

## Playback, speech, and haptics

```python
from zkite import audio, haptics

player = await audio.play("assets/sound.wav", volume=0.5)
await player.pause()
await player.seek(1.5)
await player.resume()
print(await player.state())
await player.stop()
await audio.speak("Saved", language="en-IN")

haptics.perform("confirm")  # click, reject, long_press, keyboard also supported
await haptics.vibrate(duration_ms=100)
await haptics.vibrate(pattern=[0, 50, 100, 50])
await haptics.stop()
```

Playback follows Android's selected output route, handles audio focus loss and
headset disconnects, and pauses in background. Resume is explicit. Close players
when finished. Local files and FileRefs are supported; network streaming players,
background media sessions, forced communication routing and volume-key interception
are not part of this API. Speech requires a working installed engine/language.
Haptics honor user settings; an accepted request is not proof of physical vibration.
Explicit patterns are bounded to ten seconds and do not repeat indefinitely.

## Sensors

```python
from zkite import sensors

print(await sensors.available())
subscription = await sensors.accelerometer.watch(
    lambda sample: print(sample.x, sample.y, sample.z), rate_hz=30)
# Close when leaving the screen:
subscription.close()
```

Available service names: accelerometer, linear_acceleration, gyroscope, rotation,
magnetometer, light, proximity, shake. `watch` is async and returns a Subscription.
`Sample` contains values, timestamp_ns (monotonic), accuracy and dropped count.
Acceleration uses m/s²; gyroscope uses rad/s; magnetic field uses µT; light uses lux;
proximity uses the device's distance values in cm; rotation exposes Android's vector
components. Coordinates are device-relative. Shake is an accelerometer heuristic
with gravity filtering and cooldown, not a calibrated vibration analyzer.

Requested rates are 1–100 Hz and are hints. Slow consumers drop/coalesce samples
instead of accumulating an unbounded queue. Sensor callbacks run off the Android UI
thread. Subscriptions pause in background and resume with the app, and close on
reload. Close them on screen exit yourself; automatic navigation/tab visibility
ownership is not implemented. Async callbacks are allowed; callback errors are
available in `subscription.error` and stop that subscription.

## Other integrations

```python
from zkite import share, clipboard, device, location, notifications, secrets, biometrics

await share.send(files=[file], text="Receipt", mime_type=file.mime_type)
await clipboard.write("Copied text")
text = await clipboard.read()
await device.open_url("https://example.com")  # also mailto/tel/geo
await device.keep_awake(True)
print(device.info(), await device.battery(), await device.network())
position = await location.current(high_accuracy=False, timeout=30)
await notifications.show("Download finished", "Your report is ready", id=7)
await notifications.cancel(7)
await secrets.set("token", "small secret")
value = await secrets.get("token")
await secrets.delete("token")
verified = await biometrics.authenticate(title="Unlock")
```

Share returns when the chooser launches; it does not report that anything was sent.
Location requests one foreground fix using platform providers, without a Google
Play Services dependency. Missing/disabled providers and unavailable fixes are
reported or time out; this is not background tracking. Notifications are immediate
and use Android channels; scheduling and notification action callbacks are deferred.

Secrets use an app-owned Android Keystore AES-GCM key with authenticated names and
atomic ciphertext files in Android's no-backup directory. Values are limited to
16 KiB. Hardware-backed storage is not guaranteed on every device. Biometric prompt
requires API 28+ and enrollment. It is a separate authentication UI: it does not
cryptographically gate access to secrets or provide an authentication token to a
remote service.

Bluetooth LE, NFC, USB serial, contacts/calendar, deep links, QR scanning and
background jobs remain separate future work. They are not silently emulated.

## Device Lab

`examples/device_lab` exercises the APIs without external dependencies. Run it with
`zkite run` from that directory. Core checks save a report in app-private storage;
recording/camera/permission actions are explicit buttons. The local-upload button
requires a test endpoint on port 8769 and `adb reverse tcp:8769 tcp:8769`.
