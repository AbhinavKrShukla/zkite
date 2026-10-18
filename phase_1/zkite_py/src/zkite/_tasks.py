"""Bounded background I/O with reactive state and stale-result suppression."""
from concurrent.futures import ThreadPoolExecutor
import threading
import weakref

_pool = ThreadPoolExecutor(max_workers=4, thread_name_prefix="zkite-task")
_slots = threading.BoundedSemaphore(32)
_app = None
_tasks = weakref.WeakSet()


def bind_app(app):
    global _app
    _app = weakref.ref(app)


def cancel_tasks():
    for task in list(_tasks):
        task.cancel(render=False)


class Task:
    """Run blocking I/O outside event dispatch. Create once at module scope.

    Read ``loading``, ``data`` and ``error`` in view(). ``start`` returns immediately;
    the latest start wins. ``cancel`` discards results, but cannot interrupt running
    Python code: always give network operations a timeout. Workers must return data,
    rather than mutate stores. Up to four jobs run and 32 are admitted at once.
    """
    def __init__(self):
        self.loading = False
        self.data = None
        self.error = None
        self._generation = 0
        self._future = None
        _tasks.add(self)

    def start(self, fn, *args, **kwargs):
        app = _app() if _app else None
        if app is None:
            raise RuntimeError("Task.start() requires a running ZKite app")
        with app._lock:
            if not _slots.acquire(blocking=False):
                raise RuntimeError("Background task queue is full; retry after a task finishes")
            self._generation += 1
            generation = self._generation
            if self._future:
                self._future.cancel()
            self.loading, self.error = True, None
            try:
                future = _pool.submit(fn, *args, **kwargs)
            except BaseException:
                _slots.release()
                self.loading = False
                raise
            self._future = future
            # Release capacity before acquiring the app lock, even for cancelled jobs.
            future.add_done_callback(lambda _: _slots.release())
            app._render_all()

            def complete(done):
                with app._lock:
                    if generation != self._generation or done.cancelled():
                        return
                    self.loading = False
                    try:
                        self.data = done.result()
                    except Exception as exc:
                        self.error = exc
                    app._render_all()
            future.add_done_callback(complete)
        return self

    def cancel(self, *, render=True):
        app = _app() if _app else None
        lock = app._lock if app else threading.RLock()
        with lock:
            self._generation += 1
            self.loading = False
            if self._future:
                self._future.cancel()
            if render and app:
                app._render_all()
