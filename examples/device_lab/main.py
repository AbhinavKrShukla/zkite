"""ZKite 1.2 Device Lab. Actions are explicit; results stay in app-private storage."""
import asyncio
import json
import math
import struct
import wave
from zkite import Button, Column, ScrollView, Text, store
from zkite import audio, camera, clipboard, device, files, haptics, permissions, sensors, share
from zkite import biometrics, location, network, notifications, secrets

@store
class Lab:
    status: str = "Ready — choose a test"
    busy: bool = False
    selected: object = None
    recording: object = None

async def action(s, name, fn):
    s.update(busy=True, status=name + "…")
    try:
        result = await fn()
        s.update(status=name + ": " + str(result))
    except Exception as exc:
        s.update(status=name + ": " + type(exc).__name__ + ": " + str(exc))
    finally:
        s.update(busy=False)

async def core(s):
    results = {"info": device.info(), "battery": await device.battery(), "network": await device.network()}
    sample = asyncio.Event()
    loop = asyncio.get_running_loop()
    def received(value):
        results["sensor_sample"] = list(value.values)
        loop.call_soon_threadsafe(sample.set)
    sub = await sensors.accelerometer.watch(received)
    try:
        await asyncio.wait_for(sample.wait(), 5)
    finally:
        sub.close()
    results["sensors"] = await sensors.available()
    await clipboard.write("ZKite 1.2 device test")
    results["clipboard"] = await clipboard.read()
    source = files.data_dir() / "fixture.txt"
    source.write_text("ZKite 1.2 streamed file test\n")
    ref = files.FileRef(source.as_uri(), source.name, "text/plain", source.stat().st_size, str(source))
    destination = files.data_dir() / "fixture-copy.txt"
    await ref.copy_to(destination)
    results["copy"] = destination.read_bytes() == source.read_bytes()
    haptics.perform("confirm")
    await haptics.vibrate(duration_ms=80)
    tone = files.data_dir() / "tone.wav"
    with wave.open(str(tone), "wb") as out:
        out.setparams((1, 2, 22050, 0, "NONE", "not compressed"))
        out.writeframes(b"".join(struct.pack("<h", int(500 * math.sin(i * math.tau * 440 / 22050))) for i in range(4410)))
    player = await audio.play(tone, volume=.1)
    await asyncio.sleep(.35)
    results["playback"] = await player.state()
    await player.stop()
    await secrets.set("device-lab-test", "encrypted fixture")
    results["secrets"] = await secrets.get("device-lab-test") == "encrypted fixture"
    await secrets.delete("device-lab-test")
    report = files.data_dir() / "report.json"
    report.write_text(json.dumps(results, indent=2))
    print("DEVICE_LAB_REPORT " + json.dumps(results), flush=True)
    return "Core checks passed; report.json saved"

async def pick(s):
    selected = await files.pick()
    s.update(selected=selected)
    return selected.name if selected else "cancelled"

async def photo(s):
    selected = await camera.capture()
    s.update(selected=selected)
    return selected.name if selected else "cancelled"

async def record(s):
    if s.recording is None:
        s.update(recording=await audio.record(max_seconds=10))
        return "Recording; tap again to stop (10 second limit)"
    selected = await s.recording.stop()
    s.update(recording=None, selected=selected)
    return selected.name

async def export(s):
    if s.selected is None:
        return "Choose or capture a file first"
    return await files.save(s.selected, suggested_name=s.selected.name, mime_type=s.selected.mime_type)

async def sharing(s):
    return await share.send(files=[s.selected] if s.selected else [], text="ZKite 1.2", mime_type=s.selected.mime_type if s.selected else "text/plain")

async def upload_fixture():
    path = files.data_dir() / "upload.txt"
    path.write_bytes(b"ZKite 1.2 upload fixture\n" * 10000)
    response = await network.upload("http://127.0.0.1:8769/upload", path, allow_http=True)
    return response.json()

async def play_selected(s):
    if s.selected is None:
        return "Record or select audio first"
    player = await audio.play(s.selected)
    try:
        await asyncio.sleep(2)
        return await player.state()
    finally:
        await player.stop()

def view(s: Lab):
    def button(label, fn):
        return Button("rounded-xl bg-indigo-600 p-4 text-white", label,
                      on_tap=lambda: action(s, label, fn), disabled=s.busy)
    return ScrollView("flex-1 bg-slate-950", Column("gap-3 p-5",
        Text("text-2xl text-white font-bold", "ZKite 1.2 Device Lab"),
        Text("text-white", s.status),
        button("Run core checks", lambda: core(s)),
        button("Pick document", lambda: pick(s)),
        button("Capture photo", lambda: photo(s)),
        button("Record / stop microphone", lambda: record(s)),
        button("Export selected file", lambda: export(s)),
        button("Share", lambda: sharing(s)),
        button("Speak", lambda: audio.speak("ZKite device services are ready")),
        button("Camera permission", lambda: permissions.ask("CAMERA")),
        button("Play selected audio", lambda: play_selected(s)),
        button("Test upload (local)", upload_fixture),
        button("Location", lambda: location.current(timeout=15)),
        button("Notification", lambda: notifications.show("ZKite 1.2", "Device Lab notification")),
        button("Biometric prompt", lambda: biometrics.authenticate()),
        Text("text-slate-400", "Camera and picker cancellation is normal. Microphone stops when backgrounded. Touch feedback respects phone settings.")))
