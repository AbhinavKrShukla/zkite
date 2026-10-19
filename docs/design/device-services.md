# Device services for ZKite — design proposal

Historical design proposal. The implemented 1.2.0 API and automatic permission
behavior are documented in [Device services](../device-services.md). That guide
supersedes illustrative contracts and recommendations below.

## Recommendation

Add a Python device-services layer, backed by a shared Android service bridge.
Keep widget rendering and device operations separate. Developers continue to use
Python and app.json; framework maintainers ship precompiled Android integration.
The existing framework already contains Java and Rust internally. Preserving the
no-Java/no-Gradle developer experience does not require removing these internals.

Start with permissions, lifecycle, async scheduling, files, sharing, and haptics.
Then add system-camera capture, microphone recording, playback, and sensors.
Embedded camera preview, background recording, and Bluetooth deserve later phases.

## Findings in this checkout

| Existing component | Implication |
|---|---|
| Python widgets, stores, native Rust renderer | Services need not become new rendering primitives. |
| `_tasks.py`: four workers, bounded admission, generation checks | Reuse the stale-result principle, but do not occupy workers waiting for permissions or picker results. |
| `widgets.py`: dispatch ignores callback return values | **Async handlers are documented but not scheduled in this checkout.** A host probe produced `coroutine was never awaited`; the async body did not execute. Implement and test scheduling first. |
| `_store.py`: assignments occur before the render callback acquires the app lock | Async service results need atomic state updates; rendering locks alone do not protect a multi-field update. |
| `app.py`: reload invalidates Task results | Extend this to coroutines, subscriptions, pending platform requests, and resource handles. |
| GPU shell JNI and system-bars integration | Existing route to Android, but one reusable operation/result bridge is preferable to per-feature ad hoc paths. |
| Activity has resume/destroy handling; no general device-service lifecycle | Add pause/stop/recreation ownership before acquiring camera or microphone resources. |
| Bundler generates permissions and launcher Activity | Providers, optional hardware features, and later services require manifest generation changes. |
| Custom packer has documented unsupported XML attributes | Verify actual APK/AAB packing of providers, resources and new attributes early. XML snapshots alone are insufficient. |
| Compile platform 34, target 36 | Guard runtime APIs; compile-SDK upgrades belong to the framework build, never the app author's installation. |

Documentation.md also contradicts itself about network images: section 6 says they
are unsupported, while section 10 describes the implemented HTTPS loader. Reconcile
these claims during implementation; keep this proposal separate from implemented API docs.

## Developer interface

Prefer explicit modules with autocomplete: `zkite.files`, `zkite.camera`,
`zkite.audio`, `zkite.haptics`, `zkite.sensors`, `zkite.permissions`.
Use Python dataclasses, pathlib for actual local files, standard exceptions where
appropriate, and async functions for operations that may wait on Android or I/O.
Do not expose JNI references, request codes, intents, or Android URI parsing.

Example after the async foundation exists:

```python
from zkite import Button, Column, Text, store
from zkite import files
from zkite.errors import DeviceError

@store
class Form:
    attachment: object = None
    busy: bool = False
    message: str = ""

def view(s: Form):
    async def choose():
        s.update(busy=True, message="")
        try:
            selected = await files.pick(types=["image/*", "application/pdf"])
            if selected is not None:
                s.update(attachment=selected, message=selected.name)
        except DeviceError as exc:
            s.update(message=str(exc))
        finally:
            s.update(busy=False)

    return Column("flex-1 p-6 gap-4",
        Button("p-4", "Attach file", on_tap=choose, disabled=s.busy),
        Text("", s.message))
```

Small API surface:

| Need | Proposed API | Result |
|---|---|---|
| Select a document | `await files.pick(types=["application/pdf"])` | `FileRef` or `None` on user cancel |
| Multiple documents | `await files.pick_many(types=["image/*"], limit=10)` | list, empty on user cancel |
| Gallery | `await files.pick_photo()` | `FileRef` or `None` |
| Export | `await files.save(source, suggested_name="report.pdf")` | destination `FileRef` or `None` |
| Stable app storage | `files.data_dir()`, `files.cache_dir()` | `pathlib.Path`; never derived from main.py layout |
| Take a photo | `await camera.capture()` | `FileRef` or `None`; system camera UI initially |
| Record audio | `recording = await audio.record()` | `Recording` handle; `await recording.stop()` returns `FileRef` |
| Play audio | `player = await audio.play(file)` | `Playback` handle; playback starts before return |
| Speak text | `await audio.speak("Saved")` | completes when utterance finishes, not when queued |
| UI feedback | `haptics.perform("confirm")` | immediately queues semantic feedback |
| Explicit vibration | `await haptics.vibrate(duration_ms=100)` | completes when request accepted, not physical confirmation |
| Motion | `sensors.accelerometer.watch(callback, rate_hz=30)` | closeable `Subscription` |
| Shake | `sensors.shake.watch(callback)` | closeable derived-event subscription |
| Share | `await share.send(files=[file], text="Receipt")` | chooser dispatched; no claim that recipient sent anything |

Use `pick` and `pick_many` separately so the return type stays predictable.
Resource creation, requests, and prompts are forbidden inside view construction.
Subscriptions can be created from events and closed explicitly. Before exposing
subscriptions widely, add a documented screen scope that closes resources on nav
pop. Hidden mounted Tabs are not equivalent to visible screens: visibility and
ownership must be represented separately. Default subscriptions pause offscreen.

`Playback` offers async pause/resume/stop/seek plus observable state, position and
error; `Recording` offers stop/cancel and observable elapsed time/state. Closing is
idempotent. No resource starts recording merely because a module is imported.

## Files and uploads

A document selected by Android is commonly a `content://` URI, possibly backed by
a cloud provider. It is not necessarily a real path, seekable stream, or known-size
file. Use `FileRef(name, mime_type, size: int | None)` with opaque internal identity.
Implement async `copy_to(path)`, `local_copy()`, and chunked reading. Provide a
worker-only blocking binary `open()` for ordinary synchronous HTTP clients;
opening and reading may block, and seek support is advertised rather than assumed.
A `FileRef` never pretends to implement pathlib for a content URI. [1]

Ownership contracts:

- `local_copy()` returns an explicit closeable temporary-file object exposing `.path`.
  Cleanup occurs on close; a cache sweeper reclaims abandoned files after crashes.
- `copy_to(files.data_dir() / "receipt.pdf")` creates durable app-owned data.
- `persist()` requests a persistable URI grant when supported; `release()` drops it.
  Persisting access does not prevent the provider from deleting/moving the document.
- Validate returned metadata, use generated cache filenames, enforce byte limits
  while streaming, and never join an untrusted display name directly to a directory.
- Large reads run off the UI thread. Unknown sizes and nonseekable streams work.
- Initially use CATEGORY_OPENABLE and clearly reject unsupported virtual documents.

Use Storage Access Framework for documents; use system Photo Picker where available
and fall back to document selection on older devices. Availability detection matters
because picker implementations may be backported. Neither path needs broad access
to all storage. Avoid READ_MEDIA_IMAGES and MANAGE_EXTERNAL_STORAGE for this use. [1][2]

File selection and upload are separate operations. Start by documenting FileRef
integration with existing Python HTTP clients in Task; avoid forcing a networking
library into every app. A later `Transfer` helper can add reactive progress,
cancellation and streaming multipart uploads. Specify connect/read deadlines,
non-2xx handling, maximum response size, and redirect behavior. Strip credentials
on cross-origin redirects; prohibit HTTPS downgrades by default. Do not automatically
retry non-idempotent uploads. Cancellation may stop local transfer without undoing
bytes or effects already accepted by the server. Progress must not render per chunk.

## Camera

First release: system-camera photo and video capture. Use an app-private output
file and narrow FileProvider URI grant for full-resolution photos; do not return
only the thumbnail from intent extras. Handle cancel, missing camera app, empty
output, process recreation and cleanup of abandoned output. Provider authority must
use the actual installed package, including the separate preview identity. [3][4]

Delegated camera capture can avoid declaring CAMERA; direct camera access requires
it. If an app also declares CAMERA for embedded capture, ensure it is granted before
invoking camera actions that require it for declaring apps. [5]

Later: Camera controller plus CameraPreview widget, backed by CameraX after a
packaging spike establishes its complete dependency/resource requirements. CameraX
is recommended for complex camera use and offers lifecycle integration, but adding
its classes alone is not enough. Camera2 is an alternative if the existing build
pipeline cannot support the dependency graph; it increases maintenance work. [3][6]

Embedded preview needs explicit surface composition, clipping, z-order, rotation,
mirroring, and tab/navigation lifecycle tests against ZKite's GPU renderer. Keep
video frames native; do not serialize them through JSON or Python per-frame events.
For image analysis expose bounded frame access later, with explicit release and a
latest-frame policy. Generate bounded thumbnails compatible with the existing
16,777,216-pixel image limit and handle EXIF orientation before displaying captures.

## Audio and haptics

Record foreground voice notes through MediaRecorder, initially AAC in an M4A
container, with bounded duration/size and configurable supported settings. Request
RECORD_AUDIO at the user's action. Stop/release on leaving the foreground, expose an
interruption reason, and retain only successfully finalized output. Handle stop
before enough data exists, privacy toggles, permission revocation, resource contention
and storage exhaustion. Raw PCM via AudioRecord is a later opt-in facility. [7]

Start with platform playback for local files and short sounds. Handle audio focus,
headset unplugging, completion, and interruption. "Speaker" normally means playback
through Android's selected media route; do not force the loudspeaker over headphones
or Bluetooth. Communication routing is a separate future feature. Apps targeting
API 35+ can request audio focus only while topmost or with an appropriate foreground
service. Initial playback is foreground-only; background media needs its own
service/session design. Text-to-speech must report missing engine/language data. [8]

Semantic haptics should use View.performHapticFeedback and respect user settings.
Explicit durations/patterns use Vibrator/VibratorManager with VIBRATE, which is a
manifest permission without a runtime prompt. Check device/effect support and use
supported fallbacks. Lack of haptic hardware is a benign no-op for UI feedback;
strict explicit vibration may return UnsupportedFeature. Do not promise physical
actuation merely because Android accepted the request. [9]

## Sensors

There is no standard Android sensor named "vibration sensor". Expose accelerometer,
linear acceleration, gyroscope, rotation vector, light, proximity and magnetometer;
provide shake detection as a derived convenience. Sustained vibration analysis is
a separate signal-processing use case, limited by actual hardware sample rates. [10]

Return typed samples with monotonic timestamps, accuracy, documented units and
coordinate system (device coordinates by default, explicit screen transform).
Example acceleration fields are x/y/z in m/s²; gyroscope values are rad/s. Requested
rate is a hint, not a guaranteed delivery rate. Use a bounded latest-sample buffer,
report dropped samples, and coalesce UI notifications independently of sampling.
Do not render the whole widget tree for every sensor sample. Pause on background,
unregister on close/reload, and do not silently restart a closed subscription.
Shake detection needs gravity filtering, hysteresis and cooldown; it is a heuristic.
Ordinary motion use should stay below restricted high-rate modes. Step sensors have
separate ACTIVITY_RECOGNITION requirements on newer Android versions. [10]

## One bridge and one lifecycle contract

```text
Python services + typed objects
           |
request broker: futures, deadlines, generation, resource ownership
           |
Rust / CPython bridge (small validated messages; native handles for bulk data)
           |
shared Android services (main-thread dispatch + bounded I/O executor)
           |
Android permissions / pickers / camera / audio / sensors
```

Suggested code locations:

- Python: new `_async.py`, `_device.py`, and public service modules beside `_tasks.py`.
- Native glue: `phase_3/pybridge` and GPU shell JNI; a separate device event channel.
- Android: `phase_1/android_shared/java/dev/zkite/runtime/`, reusable by shells.
- Manifest/config: `phase_2/bundler/src/manifest_gen.rs`, CLI manifest/config and
  preview-generation code, plus payload staging/provenance inputs.

Each request carries protocol version, runtime generation, unique ID, operation,
validated arguments, and optional deadline. Results carry ID, generation and either
a typed value descriptor or a stable error code. Use handles or duplicated file
descriptors for bulk bytes; never base64 whole recordings across the control bridge.
Negotiate supported operations against the installed runtime; an old shell returns
a clear runtime-upgrade error rather than hanging.

Android callbacks enqueue results and return immediately. Complete Python futures
on their owning asyncio loop with thread-safe scheduling. Never wait for Python
while holding the Android main thread, or hold the app/render lock across await.
Make store.update atomic under the app lock and audit other shared state touched
by async handlers. Track every scheduled event coroutine, report exceptions through
the existing error policy, and cancel it on reload/owner shutdown.

Serialize interactive platform requests. Concurrent requests for the same permission
share a result. Bound admissions; do not queue an unlimited series of dialogs.
Use separate limits for short control events and streaming data. Device calls do
not occupy the four Task threads while waiting for user interaction.

Distinguish lifecycle cases:

- Opening the system picker pauses the app: preserve that pending request.
- Backgrounding pauses sensors and stops foreground recording; do not abandon
  picker results merely because the initiating activity lost focus.
- Activity recreation reattaches Android ownership and restores routing metadata;
  keep the Python runtime/broker stable if feasible. If not, terminate outstanding
  requests explicitly as interrupted; never leave a future permanently pending.
- Process death cannot preserve Python coroutine stacks. On fresh startup clean up
  abandoned captures and expose recoverable durable outputs separately.
- Hot reload cancels old Python operations, closes old subscriptions, and rejects
  late results using generation tokens. An external camera/picker may remain open;
  its eventual result must still be consumed and cleaned up safely.

Prefer AndroidX Activity Result APIs if compatible with the build system. Otherwise
encapsulate platform callbacks with saved routing state and tests. Android recommends
Activity Result APIs and highlights recreation while another activity is running. [11]

Error contract: DeviceError base, PermissionDenied, PermissionNotDeclared,
UnsupportedFeature, ResourceBusy, DeviceUnavailable, OperationInterrupted,
DeviceIOError and typed runtime-mismatch error. Use standard TimeoutError for
deadlines and asyncio.CancelledError for programmatic cancellation. User dismissal
of a picker returns None/empty list. Do not collapse all errors into None.

Host imports must be harmless. Calling real device APIs without an Android backend
raises DeviceUnavailable; provide an injectable fake backend for tests. Native comes
first; WebView either uses the same native service broker or explicitly reports
unsupported operations. Do not imply browser parity without testing.

## Permissions and build configuration

Keep current `android.permissions` compatible. Add semantic capabilities to app.json
so maintainers map features to manifest declarations; never infer the entire manifest
from Python imports, since imports may be dynamic or unused.

Proposed configuration (new keys, not accepted today):

```json
{
  "android": {
    "permissions": ["INTERNET"],
    "capabilities": ["files", "camera.capture", "audio.record", "audio.play", "haptics"]
  }
}
```

Capability planning supplies providers, optional uses-feature entries and necessary
permissions. `camera.capture` means external capture; `camera.preview` adds CAMERA.
Mark optional camera/microphone hardware `required=false` so permission-derived
filtering does not accidentally exclude devices. Explicit required hardware remains
a developer choice. Provide build output showing why each permission/component was
added. App developers never edit AndroidManifest.xml.

At runtime check permission at each protected operation, request just in time, and
expose granted/denied plus rationale/settings guidance. Do not treat a false
shouldShowRequestPermissionRationale result alone as proof of permanent denial.
Never repeatedly prompt after denial or automatically navigate to settings. [12]

Preview and release must share capability-derived configuration; changing capabilities
requires rebuilding/reinstalling the preview. Include capability schema, provider
resources, shared Android sources, and bridge protocol in cache/provenance inputs.
Never give the preview every dangerous permission as a shortcut.

## Additional features worth including

| Priority | Feature | Why / scope |
|---|---|---|
| First foundation | App-private storage paths, lifecycle, permission/capability introspection | Every real device feature depends on these. |
| Early | Share sheet, clipboard, open URL/email/dialer | Useful small integrations; use system UI and avoid unnecessary access. |
| Early | File export, photo picking, network state, battery/charging state | Complete ordinary offline/online app workflows; connectivity is not proof of internet reachability. |
| Next | Foreground location, local notifications | Broad utility; require their own permission and lifecycle research before implementation. |
| Next | Torch, QR/barcode scanner, text-to-speech | Build on camera/audio foundations; keep dependencies optional where practical. |
| Next | KeyStore-backed secrets and biometric unlock | Useful security primitives; expose precise guarantees, not a generic "secure storage" promise. |
| Later | Bluetooth LE, NFC, USB serial | Valuable but larger protocol, permission and hardware compatibility surfaces. |
| Later | Background transfers, scheduled jobs, background media | Need explicit Android service/job ownership; Python threads do not imply background reliability. |
| Later | Contacts/calendar selection, deep links, receiving shared files | Prefer selected-item flows and well-defined external entry points. |

## Delivery order and acceptance criteria

1. **Foundation:** async event scheduler, atomic state updates, request broker,
   error types, lifecycle and permission host, version handshake, injectable backend.
   Test coroutine execution/errors, concurrent completions, timeout/cancel/result
   races, reload and runtime mismatch before exposing service imports.
2. **First useful slice:** app paths, files/photo picker, save/share, haptics,
   manifest provider and optional hardware support. Build a Device Lab Python app.
   Gate on real packaged APK plus preview; validate AAB generation too.
3. **Capture and audio:** external camera, record/play/TTS. Gate on denial,
   interruption, output validation, lifecycle cleanup and device audio-focus behavior.
4. **Sensors:** typed samples, bounded subscriptions, shake, navigation visibility
   integration. Gate on missing sensors, rate pressure and zero retained listeners.
5. **Advanced features:** embedded CameraPreview and scanner, then separately scoped
   location/notifications/background capabilities. Do packaging/composition spikes
   before promising their API or release date.

Do not choose dates before the broker/recreation and provider-packaging spikes.
No need to replace the renderer or introduce a Java build tool for app developers.

## Device inspection and validation plan

Connected Moto g54 5G, Android API 35 (Android 15), authorized via adb. Read-only
`pm list features` reports rear/front cameras, flash, microphone, audio output,
accelerometer, gyroscope, compass, light, proximity, step counter/detector, GPS and
Bluetooth LE. `dumpsys sensorservice` lists linear acceleration and rotation vector
as well. Vibrator services exist. These are capability observations, not functional
validation of proposed Python APIs.

The vibrator service currently reports TOUCH intensity OFF. An accepted haptic
request may therefore produce no sensation; tests must respect settings rather
than overriding them. Do not change the phone's settings to make a test pass.

Completed host probe: registering async def with RenderContext then dispatching
returned True but left the body unexecuted and emitted an unawaited-coroutine warning.
No camera capture, microphone recording, upload, APK installation, or new feature
implementation was performed for this research/design task.

For each implementation milestone, use a dedicated Device Lab package and disposable
fixtures. Test both standalone and preview:

- Picker success, Back cancellation, repeated taps, multi-select limits, provider
  unavailability, large/unknown-size/cloud file, revoked grant and export failure.
- First permission grant, deny, repeat deny, settings revocation, one-time grant,
  sensor privacy switches, and missing manifest entry.
- Rotation/recreation, Home/return, lock/unlock, navigation pop, tab hiding, process
  death and hot reload during pending operations. Check stale results never mutate
  the new screen and resources do not leak.
- Full-resolution camera output, EXIF orientation, cancelled/empty captures;
  short recording, pause interruption, focus loss, headphones and Bluetooth routes.
- Sensor timing and bounded buffering under slow Python callbacks; no event or
  recording continues after its owner is closed.
- Upload to a local controlled endpoint using disposable data: checksum, multipart
  metadata, progress bounds, interruption, timeout, unknown-size input and redirects.
- Capture app-scoped logs, operation outcomes, resource counts and APK identity.
  Shell-level vibrator/camera commands alone do not verify the Python/JNI path.

One API-35 phone cannot establish API-26-to-36 compatibility. Add API 26 and 36
emulator coverage for fallbacks/target behavior, plus later real-device OEM coverage.
Physical camera/audio/haptics quality needs observation beyond successful callbacks.

## Research references

These are primary Android sources; design choices above are recommendations for
ZKite, not claims that Android prescribes this exact Python interface.

1. [Storage Access Framework and documents](https://developer.android.com/training/data-storage/shared/documents-files)
2. [Photo Picker and fallback support](https://developer.android.com/training/data-storage/shared/photo-picker)
3. [Camera intents](https://developer.android.com/media/camera/camera-intents)
4. [Secure file sharing and FileProvider](https://developer.android.com/training/secure-file-sharing/setup-sharing)
5. [MediaStore ACTION_IMAGE_CAPTURE permission contract](https://developer.android.com/reference/android/provider/MediaStore#ACTION_IMAGE_CAPTURE)
6. [CameraX architecture](https://developer.android.com/media/camera/camerax/architecture)
7. [MediaRecorder overview](https://developer.android.com/media/platform/mediarecorder)
8. [Audio focus](https://developer.android.com/media/optimize/audio-focus)
9. [Haptic feedback](https://developer.android.com/develop/ui/views/haptics/haptic-feedback)
10. [Sensors overview](https://developer.android.com/develop/sensors-and-location/sensors/sensors_overview)
11. [Activity results and recreation](https://developer.android.com/training/basics/intents/result)
12. [Runtime permission workflow](https://developer.android.com/training/permissions/requesting)
