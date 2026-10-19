"""Streaming multipart uploads with deadlines, cancellation, and bounded responses."""
import asyncio
from dataclasses import dataclass
import http.client
import json
from pathlib import Path
import secrets
import threading
import time
from urllib.parse import urlsplit
from .files import FileRef


@dataclass(frozen=True)
class Response:
    status: int
    headers: dict
    body: bytes

    def json(self):
        return json.loads(self.body)


class HTTPError(OSError):
    def __init__(self, response):
        self.response = response
        super().__init__(f"Upload returned HTTP {response.status}")


_slots = threading.BoundedSemaphore(4)


async def upload(url, file, *, field="file", fields=None, headers=None,
                 timeout=30, max_bytes=256 * 1024 * 1024, max_response_bytes=2 * 1024 * 1024,
                 on_progress=None, allow_http=False):
    """Upload one file. Progress(sent, total) runs on the caller's event loop.

    No retries or redirects: callers decide whether replaying an upload is safe.
    Cancellation stops sending at the next chunk; socket I/O is bounded by timeout.
    """
    parsed = urlsplit(url)
    if parsed.scheme not in (("https", "http") if allow_http else ("https",)) or not parsed.hostname:
        raise ValueError("Use HTTPS, or explicitly enable allow_http for development")
    if parsed.username or parsed.password or parsed.fragment:
        raise ValueError("URL must not contain credentials or a fragment")
    if not 0 < timeout <= 600 or max_bytes <= 0 or max_response_bytes <= 0:
        raise ValueError("Upload limits must be positive; timeout must be at most 600s")
    if not _slots.acquire(False):
        raise RuntimeError("At most four concurrent uploads")
    local = None
    cancel = threading.Event()
    owner = asyncio.get_running_loop()
    def quote(value):
        value = str(value)
        if any(c in value for c in '\r\n"\\'):
            raise ValueError("Multipart names cannot contain quotes, backslashes or newlines")
        return value
    try:
        if isinstance(file, FileRef):
            local = await file.local_copy(max_bytes=max_bytes)
            path, filename, mime = local.path, quote(file.name), file.mime_type
        else:
            path = Path(file)
            filename, mime = quote(path.name), "application/octet-stream"
        quote(mime)
        boundary = "zkite-" + secrets.token_hex(16)
        parts = []
        for key, value in (fields or {}).items():
            parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{quote(key)}"\r\n\r\n{value}\r\n'.encode())
        parts.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{quote(field)}"; filename="{filename}"\r\nContent-Type: {mime}\r\n\r\n'.encode())
        prefix = b"".join(parts)
        if len(prefix) > 65536:
            raise ValueError("Multipart metadata exceeds 64 KiB")
        suffix = f"\r\n--{boundary}--\r\n".encode()
        def transfer():
            connection = None
            try:
                with path.open("rb") as stream:
                    import os
                    size = os.fstat(stream.fileno()).st_size
                    if size > max_bytes:
                        raise ValueError("File exceeds max_bytes")
                    total = len(prefix) + size + len(suffix)
                    cls = http.client.HTTPSConnection if parsed.scheme == "https" else http.client.HTTPConnection
                    connection = cls(parsed.hostname, parsed.port, timeout=timeout)
                    connection.putrequest("POST", (parsed.path or "/") + ("?" + parsed.query if parsed.query else ""))
                    for key, value in (headers or {}).items():
                        if key.lower() in ("content-length", "content-type", "transfer-encoding", "host"):
                            raise ValueError(f"Header {key} is managed by upload")
                        connection.putheader(key, value)
                    connection.putheader("Content-Type", f"multipart/form-data; boundary={boundary}")
                    connection.putheader("Content-Length", str(total))
                    connection.endheaders()
                    sent, last = 0, 0.0
                    deadline = time.monotonic() + timeout
                    def send(chunk):
                        nonlocal sent, last
                        if cancel.is_set():
                            raise InterruptedError("Upload cancelled")
                        remaining = deadline - time.monotonic()
                        if remaining <= 0:
                            raise TimeoutError("Upload deadline exceeded")
                        if connection.sock:
                            connection.sock.settimeout(remaining)
                        connection.send(chunk)
                        sent += len(chunk)
                        now = time.monotonic()
                        if on_progress and (now-last >= .1 or sent == total):
                            owner.call_soon_threadsafe(progress, sent, total)
                            last = now
                    send(prefix)
                    remaining = size
                    while remaining:
                        chunk = stream.read(min(65536, remaining))
                        if not chunk:
                            raise OSError("File changed during upload")
                        send(chunk)
                        remaining -= len(chunk)
                    send(suffix)
                    response = connection.getresponse()
                    chunks, count = [], 0
                    while count <= max_response_bytes:
                        if cancel.is_set():
                            raise InterruptedError("Upload cancelled")
                        remaining = deadline - time.monotonic()
                        if remaining <= 0:
                            raise TimeoutError("Upload deadline exceeded")
                        if connection.sock:
                            connection.sock.settimeout(remaining)
                        chunk = response.read1(min(65536, max_response_bytes + 1 - count))
                        if not chunk:
                            break
                        chunks.append(chunk)
                        count += len(chunk)
                    body = b"".join(chunks)
                    if len(body) > max_response_bytes:
                        raise OSError("Upload response exceeds limit")
                    result = Response(response.status, dict(response.getheaders()), body)
                    if not 200 <= result.status < 300:
                        raise HTTPError(result)
                    return result
            finally:
                if connection:
                    connection.close()
                if local:
                    local.close()
                _slots.release()
        def progress(sent, total):
            if not cancel.is_set():
                from ._async import submit
                submit(on_progress(sent, total))
        worker = owner.run_in_executor(None, transfer)
        # Shield prevents cancellation of a queued worker from skipping its cleanup.
        try:
            return await asyncio.shield(worker)
        except asyncio.CancelledError:
            cancel.set()
            def consume(done):
                try: done.exception()
                except BaseException: pass
            worker.add_done_callback(consume)
            raise
    except BaseException:
        if "worker" not in locals():
            if local: local.close()
            _slots.release()
        raise
