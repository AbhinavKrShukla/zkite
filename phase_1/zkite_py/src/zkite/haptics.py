"""Feedback honors Android's settings and hardware capabilities."""
from ._device import request
from ._async import submit


def perform(effect="confirm"):
    if effect not in ("confirm", "reject", "click", "long_press", "keyboard"):
        raise ValueError("Unknown haptic effect")
    return submit(request("haptics.perform", effect=effect))


async def vibrate(*, duration_ms=100, pattern=None):
    pattern = [0, duration_ms] if pattern is None else list(pattern)
    if not 1 <= len(pattern) <= 64 or any(type(v) is not int or v < 0 for v in pattern) or not 0 < sum(pattern) <= 10000:
        raise ValueError("Vibration pattern must total 1–10000 ms with at most 64 entries")
    await request("haptics.vibrate", pattern=pattern)


async def stop():
    await request("haptics.stop")
