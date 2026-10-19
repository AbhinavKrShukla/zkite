"""Owned event coroutines. The loop never holds the render lock across an await."""
import asyncio
import contextvars
import inspect
import threading
import weakref

_lock = threading.RLock()
_loop = None
_app = None
_generation = 0
_jobs = set()
_owner = contextvars.ContextVar("zkite_async_owner", default=None)


def bind_app(app):
    global _app
    previous = _app() if _app else None
    if previous is not None and previous is not app:
        cancel_all()
        from . import _device
        _device.reset()
    _app = weakref.ref(app)


def current():
    owner = _owner.get()
    return owner is None or owner == _generation


def loop():
    global _loop
    with _lock:
        if _loop is None:
            ready = threading.Event()
            def run():
                global _loop
                _loop = asyncio.new_event_loop()
                asyncio.set_event_loop(_loop)
                ready.set()
                _loop.run_forever()
            threading.Thread(target=run, name="zkite-async", daemon=True).start()
            ready.wait()
        return _loop


def submit(value):
    if not inspect.isawaitable(value):
        return value
    with _lock:
        if len(_jobs) >= 64:
            if inspect.iscoroutine(value):
                value.close()
            raise RuntimeError("Too many pending async handlers")
        generation = _generation
        state = {"started": False, "cancelled": False}
        async def owned():
            with _lock:
                if state["cancelled"]:
                    return
                state["started"] = True
            token = _owner.set(generation)
            try:
                if generation != _generation:
                    if inspect.iscoroutine(value):
                        value.close()
                    return
                return await value
            except asyncio.CancelledError:
                raise
            except Exception as exc:
                app = _app() if _app else None
                if app is not None and generation == _generation:
                    from .errors import log_error
                    import traceback
                    app.report_async_error("".join(traceback.format_exception(exc)))
                else:
                    import traceback
                    traceback.print_exc()
            finally:
                _owner.reset(token)
        future = asyncio.run_coroutine_threadsafe(owned(), loop())
        _jobs.add(future)
        def done(f):
            with _lock:
                _jobs.discard(f)
                if not state["started"] and inspect.iscoroutine(value):
                    state["cancelled"] = True
                    value.close()
        future.add_done_callback(done)
        return future


def cancel_all():
    global _generation
    with _lock:
        _generation += 1
        for future in list(_jobs):
            future.cancel()
