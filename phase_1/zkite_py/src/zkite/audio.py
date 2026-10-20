"""Foreground recording, playback and text-to-speech."""
from pathlib import Path
import math
from urllib.parse import urlsplit
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
        state = await _device.request("audio.state", handle=self.id)
        if state.get("state") == "closed":
            self.closed = True
        return state


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
        _number(seconds, 0, 2147483, "seconds")
        await _device.request("audio.seek", handle=self.id, seconds=seconds)

    def watch(self, callback, *, interval=0.25):
        """Poll state into a sync/async callback; cancel the returned future to stop."""
        from . import _async
        import asyncio
        import inspect
        _number(interval, .05, 60, "interval")
        if not callable(callback):
            raise TypeError("callback must be callable")
        self._check()
        async def run():
            while not self.closed and self._generation == _device._generation:
                state = await self.state()
                result = callback(state)
                if inspect.isawaitable(result): await result
                if state["state"] in ("completed", "closed", "error"): break
                await asyncio.sleep(interval)
        return _async.submit(run())

    async def configure(self, *, volume=None, loop=None, speed=None):
        """Change volume (0–1), looping, or speed (.25–4) without replacing playback."""
        self._check()
        args = {}
        if volume is not None:
            _number(volume, 0, 1, "volume")
            args["volume"] = volume
        if speed is not None:
            _number(speed, .25, 4, "speed")
            args["speed"] = speed
        if loop is not None:
            args["loop"] = bool(loop)
        await _device.request("audio.configure", handle=self.id, **args)

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
    _number(volume, 0, 1, "volume")
    uri = _source_uri(source)
    return Playback(await _device.request("audio.play", uri=uri, volume=volume, loop=bool(loop)))


async def speak(text, *, language=None, rate=1.0):
    if not isinstance(text, str) or not text or len(text) > 4000 or not 0.1 <= rate <= 3:
        raise ValueError("Provide 1–4000 characters and a rate between 0.1 and 3")
    await _device.request("audio.speak", text=text, language=language, rate=rate, timeout=600)


def _number(value, low, high, name):
    if not isinstance(value, (int, float)) or not math.isfinite(value) or not low <= value <= high:
        raise ValueError(f"{name} must be between {low} and {high}")


def _source_uri(source):
    if isinstance(source, FileRef):
        return source.uri
    value = str(source)
    if urlsplit(value).scheme in ("https", "http", "content", "file"):
        return value
    return Path(source).resolve().as_uri()
