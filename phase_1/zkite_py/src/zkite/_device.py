"""Versioned, bounded request broker for the native Android service host."""
import asyncio
import itertools
import json
import threading
from . import errors

_ids = itertools.count(1)
_pending = {}
_subscriptions = {}
_pump = None
_backend = None
_guard = threading.RLock()
_generation = 0


def exchange(message):
    backend = _backend
    if backend is None:
        import __main__
        backend = getattr(__main__, "_zkite_device_exchange", None)
    if backend is None:
        raise errors.DeviceUnavailable("Device services require the ZKite 1.2 Android native runtime")
    value = backend(json.dumps(message, allow_nan=False))
    response = json.loads(value)
    if isinstance(response, dict) and "error" in response:
        raise error(response)
    return response


def error(message):
    cls = getattr(errors, message.get("error", "DeviceError"), errors.DeviceError)
    if not isinstance(cls, type) or not issubclass(cls, errors.DeviceError):
        cls = errors.DeviceError
    return cls(message.get("message", "Device operation failed"))


async def request(operation, *, timeout=120, **args):
    global _pump
    from . import _async
    if not _async.current():
        raise asyncio.CancelledError()
    running = asyncio.get_running_loop()
    with _guard:
        if len(_pending) >= 32:
            raise errors.ResourceBusy("Too many pending device requests")
        ident = next(_ids)
        future = running.create_future()
        _pending[ident] = (running, future)
        try:
            exchange({"version": 1, "id": ident, "op": operation, "args": args})
        except BaseException:
            _pending.pop(ident, None)
            raise
        if _pump is None or _pump.done():
            _pump = asyncio.run_coroutine_threadsafe(_poll(), _async.loop())
    try:
        return await asyncio.wait_for(future, timeout)
    finally:
        with _guard:
            _pending.pop(ident, None)
        if future.cancelled():
            try:
                exchange({"op": "cancel", "id": ident})
            except errors.DeviceError:
                pass


def _complete(future, event):
    if not future.done():
        if "error" in event:
            future.set_exception(error(event))
        else:
            future.set_result(event.get("value"))


async def _poll():
    while True:
        with _guard:
            idle = not _pending and not _subscriptions
        if idle:
            await asyncio.sleep(0.05)
            continue
        try:
            events = exchange({"op": "poll"})
            for event in events:
                with _guard:
                    target = _pending.get(event.get("id"))
                    sub = _subscriptions.get(event.get("subscription"))
                if target:
                    target[0].call_soon_threadsafe(_complete, target[1], event)
                elif sub:
                    sub._deliver(event)
        except Exception as exc:
            with _guard:
                targets = list(_pending.values())
            for owner, future in targets:
                owner.call_soon_threadsafe(_complete, future, {
                    "error": "DeviceUnavailable", "message": str(exc)})
        await asyncio.sleep(0.025)


def reset():
    global _generation
    _generation += 1
    with _guard:
        subscriptions = list(_subscriptions.values())
        targets = list(_pending.values())
        _pending.clear()
    for sub in subscriptions:
        sub.close()
    for owner, future in targets:
        if not owner.is_closed():
            owner.call_soon_threadsafe(future.cancel)
    try:
        exchange({"op": "reset"})
    except errors.DeviceError:
        pass


def install_backend(backend):
    """Inject a protocol backend for host tests; None restores native discovery."""
    global _backend
    reset()
    _backend = backend
