"""One foreground location fix; no background tracking or Google Play dependency."""
from dataclasses import dataclass
from ._device import request


@dataclass(frozen=True)
class Position:
    latitude: float
    longitude: float
    accuracy: float
    timestamp_ms: int
    altitude: float | None = None


async def current(*, high_accuracy=False, timeout=30):
    return Position(**await request("location.current", high_accuracy=bool(high_accuracy), timeout=timeout))
