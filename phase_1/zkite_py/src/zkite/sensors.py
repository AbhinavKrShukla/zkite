"""Bounded foreground sensor subscriptions. Close subscriptions when leaving a screen."""
from dataclasses import dataclass
import math
from . import _device, _async


@dataclass(frozen=True)
class Sample:
    values: tuple
    timestamp_ns: int
    accuracy: int
    dropped: int = 0

    @property
    def x(self): return self.values[0]
    @property
    def y(self): return self.values[1]
    @property
    def z(self): return self.values[2]


class Subscription:
    def __init__(self, ident, callback):
        self.id, self.callback = ident, callback
        self.closed = False
        self._busy = None
        self.error = None
        self.dropped = 0
        with _device._guard:
            _device._subscriptions[ident] = self

    def _deliver(self, event):
        if self.closed:
            return
        if self._busy and not self._busy.done():
            self.dropped += 1
            return
        try:
            value = event["value"]
            sample = Sample(tuple(value["values"]), value["timestamp_ns"], value["accuracy"], value.get("dropped", 0) + self.dropped)
            async def deliver():
                import asyncio, inspect
                try:
                    value = await asyncio.to_thread(self.callback, sample)
                    if inspect.isawaitable(value):
                        await value
                except Exception as exc:
                    self.error = exc
                    self.close()
            self._busy = _async.submit(deliver())
        except Exception as exc:
            self.error = exc
            self.close()

    def close(self):
        if not self.closed:
            self.closed = True
            with _device._guard:
                _device._subscriptions.pop(self.id, None)
            if self._busy:
                self._busy.cancel()
            try:
                _device.exchange({"op": "unsubscribe", "id": self.id})
            except _device.errors.DeviceError:
                pass

    def __enter__(self): return self
    def __exit__(self, *exc): self.close()


class Sensor:
    def __init__(self, name): self.name = name

    async def watch(self, callback, *, rate_hz=30):
        if not callable(callback) or not math.isfinite(rate_hz) or not 1 <= rate_hz <= 100:
            raise ValueError("callback must be callable; rate_hz must be 1–100")
        ident = await _device.request("sensors.watch", sensor=self.name, rate_hz=rate_hz)
        sub = Subscription(ident, callback)
        # A response may have been the last pending request: ensure polling survives.
        with _device._guard:
            if _device._pump is None or _device._pump.done():
                import asyncio
                _device._pump = asyncio.run_coroutine_threadsafe(_device._poll(), _async.loop())
        return sub


accelerometer = Sensor("accelerometer")
linear_acceleration = Sensor("linear_acceleration")
gyroscope = Sensor("gyroscope")
rotation = Sensor("rotation")
light = Sensor("light")
proximity = Sensor("proximity")
magnetometer = Sensor("magnetometer")
shake = Sensor("shake")


async def available():
    return await _device.request("sensors.list")
