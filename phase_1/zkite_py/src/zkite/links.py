"""Incoming Android links. Register schemes/hosts in app.json before building."""
import asyncio
import inspect
from dataclasses import dataclass
from urllib.parse import urlsplit, parse_qs, urlencode, quote
from . import _device, _async


@dataclass(frozen=True)
class Link:
    url: str

    @property
    def scheme(self): return urlsplit(self.url).scheme
    @property
    def host(self): return urlsplit(self.url).hostname
    @property
    def path(self): return urlsplit(self.url).path
    @property
    def query(self): return parse_qs(urlsplit(self.url).query, keep_blank_values=True)


def create(scheme, host, path="/", **query):
    """Build a shareable URL; query values are encoded, sequences repeat keys."""
    import re
    if not re.fullmatch(r"[a-z][a-z0-9+.-]*", scheme) or not re.fullmatch(r"[a-z0-9.-]+", host):
        raise ValueError("Use a lowercase scheme and hostname")
    if not path.startswith("/"):
        raise ValueError("path must start with /")
    return f"{scheme}://{host}{quote(path, safe='/')}" + ("?" + urlencode(query, doseq=True) if query else "")


def next_link():
    """Consume the oldest cold/warm launch link, or None (queue limit: 32)."""
    url = _device.exchange({"op": "links.next"}).get("url")
    return Link(url) if url else None


def listen(callback, *, interval=0.1):
    """Deliver launch and subsequent links in order; returns a cancellable future.

    Start once outside view(), after defining stores. Sync/async callbacks work.
    Reload cancels this listener. Only one consumer should read the link queue.
    """
    if not callable(callback) or not 0.025 <= interval <= 10:
        raise ValueError("Provide a callback and interval between .025 and 10")
    from . import _store
    parameters = list(_store.signature_with_resolved_annotations(callback).parameters.values())
    stores = {p.name: p.annotation for p in parameters[1:] if _store.is_store(p.annotation)}
    async def run():
        while True:
            link = next_link()
            if link is not None:
                result = callback(link, **{name: _store.get_instance(cls) for name, cls in stores.items()})
                if inspect.isawaitable(result):
                    await result
            await asyncio.sleep(interval)
    return _async.submit(run())
