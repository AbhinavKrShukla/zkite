"""Bounded, asynchronous HTTPS image loading for the native renderer.

WebView delegates URL loading to its browser. Native uses the same PNG/JPEG
 decoder as local images, via a data URI, and the existing background render path.
"""
from __future__ import annotations

import base64
from collections import OrderedDict
from concurrent.futures import ThreadPoolExecutor
import sys
import threading
import time
import urllib.request

from . import _store

_MAX_BYTES = 2 * 1024 * 1024
_MAX_ENTRIES = 16
_cache: OrderedDict[str, tuple[str, float]] = OrderedDict()
_pending: set[str] = set()
_lock = threading.Lock()
_pool = ThreadPoolExecutor(max_workers=4, thread_name_prefix="zkite-image")


class _HTTPSRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if not newurl.lower().startswith("https://"):
            raise ValueError("image redirects must use HTTPS")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def source(url: str) -> str:
    """Return cached pixels or an empty placeholder; never block rendering."""
    if not url.lower().startswith(("https://", "http://")):
        return url
    if not url.lower().startswith("https://"):
        # Match Android's default cleartext policy. Use HTTPS on both backends.
        return ""
    with _lock:
        if url in _cache:
            data, retry_at = _cache[url]
            if data or time.monotonic() < retry_at:
                _cache.move_to_end(url)
                return data
            del _cache[url]
        if url not in _pending and len(_pending) < _MAX_ENTRIES:
            _pending.add(url)
            _pool.submit(_download, url)
    return ""


def _download(url: str) -> None:
    data_uri = ""
    try:
        opener = urllib.request.build_opener(_HTTPSRedirect())
        request = urllib.request.Request(url, headers={"User-Agent": "ZKite/1"})
        with opener.open(request, timeout=15) as response:
            data = response.read(_MAX_BYTES + 1)
        if len(data) > _MAX_BYTES:
            raise ValueError("image exceeds 2 MiB")
        if data.startswith(b"\x89PNG\r\n\x1a\n"):
            mime = "image/png"
        elif data.startswith(b"\xff\xd8\xff"):
            mime = "image/jpeg"
        else:
            raise ValueError("native URL images must be PNG or JPEG")
        data_uri = f"data:{mime};base64," + base64.b64encode(data).decode("ascii")
    except Exception as exc:
        print(f"ZKite image download failed: {type(exc).__name__}: {exc}", file=sys.stderr)
    with _lock:
        _pending.discard(url)
        _cache[url] = (data_uri, time.monotonic() + 30)
        while len(_cache) > _MAX_ENTRIES:
            _cache.popitem(last=False)
    # Do not hold the cache lock while rendering: Image asks for cached data again.
    if _store._RENDER_CALLBACK is not None:
        _store._RENDER_CALLBACK()
