"""Foreground recording, playback and text-to-speech."""
from pathlib import Path
from . import _device
from .files import FileRef, _ref
from .errors import OperationInterrupted


class _Handle:
    def __init__(self, ident):
        self.id = ident
        self._generation = _device._generation
        self.closed = False

    def _check(self):
        if self.closed or self._generation != _device._generation:
            raise OperationInterrupted("Audio handle is closed or belongs to a previous runtime generation")

    async def state(self):
        self._check()
        return await _device.request("audio.state", handle=self.id)


class Recording(_Handle):
    async def stop(self):
        self._check()
        try:
            return _ref(await _device.request("audio.record.stop", handle=self.id))
        finally:
            self.closed = True

    async def cancel(self):
        if not self.closed:
            try:
                await _device.request("audio.close", handle=self.id)
            finally:
                self.closed = True


class Playback(_Handle):
    async def pause(self):
        self._check()
        await _device.request("audio.pause", handle=self.id)

    async def resume(self):
        self._check()
        await _device.request("audio.resume", handle=self.id)

    async def seek(self, seconds):
        self._check()
        if seconds < 0:
            raise ValueError("seconds must be nonnegative")
        await _device.request("audio.seek", handle=self.id, seconds=seconds)

    async def stop(self):
        if not self.closed:
            try:
                await _device.request("audio.close", handle=self.id)
            finally:
                self.closed = True


async def record(*, max_seconds=300, max_bytes=32 * 1024 * 1024):
    if not 1 <= max_seconds <= 3600 or not 1024 <= max_bytes <= 256 * 1024 * 1024:
        raise ValueError("Recording limits out of range")
    return Recording(await _device.request("audio.record", max_seconds=max_seconds, max_bytes=max_bytes))


async def play(source, *, volume=1.0, loop=False):
    if not 0 <= volume <= 1:
        raise ValueError("volume must be between 0 and 1")
    uri = source.uri if isinstance(source, FileRef) else Path(source).resolve().as_uri()
    return Playback(await _device.request("audio.play", uri=uri, volume=volume, loop=bool(loop)))


async def speak(text, *, language=None, rate=1.0):
    if not isinstance(text, str) or not text or len(text) > 4000 or not 0.1 <= rate <= 3:
        raise ValueError("Provide 1–4000 characters and a rate between 0.1 and 3")
    await _device.request("audio.speak", text=text, language=language, rate=rate, timeout=600)
