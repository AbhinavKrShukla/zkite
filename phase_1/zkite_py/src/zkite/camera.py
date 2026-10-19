"""Capture with the system camera; media stays private until explicitly shared."""
from ._device import request
from .files import _ref


async def capture():
    return _ref(await request("camera.capture", timeout=600))


async def record_video(*, duration=60):
    if not 1 <= duration <= 600:
        raise ValueError("duration must be between 1 and 600 seconds")
    return _ref(await request("camera.video", timeout=duration + 120, duration=duration))


async def torch(enabled=True):
    await request("camera.torch", enabled=bool(enabled))
