"""System file pickers and URI-safe file references (Android native runtime)."""
from dataclasses import dataclass
from pathlib import Path
import asyncio
import shutil
import tempfile
from ._device import request, exchange


@dataclass(frozen=True)
class FileRef:
    uri: str
    name: str
    mime_type: str = "application/octet-stream"
    size: int | None = None
    path: str | None = None

    async def copy_to(self, destination, *, max_bytes=256 * 1024 * 1024):
        """Stream into destination atomically. Existing file is replaced on success."""
        destination = str(Path(destination).resolve())
        await request("files.copy", uri=self.uri, destination=destination, max_bytes=max_bytes)
        return Path(destination)

    async def local_copy(self, *, max_bytes=256 * 1024 * 1024):
        directory = tempfile.mkdtemp(prefix="zkite-", dir=cache_dir())
        target = Path(directory) / "content"
        try:
            await self.copy_to(target, max_bytes=max_bytes)
        except BaseException:
            shutil.rmtree(directory, ignore_errors=True)
            raise
        return LocalCopy(target, directory)

    async def persist(self):
        await request("files.persist", uri=self.uri)
        return self

    async def release(self):
        await request("files.release", uri=self.uri)

    def open(self, mode="rb"):
        """Open a local capture. For provider documents use await local_copy()."""
        if mode != "rb":
            raise ValueError("FileRef.open supports rb; use files.save for exporting")
        if self.path is None:
            raise ValueError("Provider file has no path; use await file.local_copy()")
        return open(self.path, "rb")


class LocalCopy:
    def __init__(self, path, directory):
        self.path, self._directory = Path(path), directory

    def close(self):
        shutil.rmtree(self._directory, ignore_errors=True)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def _ref(value):
    return FileRef(**value) if value else None


def _types(types):
    values = list(types or ["*/*"])
    if not values or any(not isinstance(t, str) or "/" not in t for t in values):
        raise ValueError("types must contain MIME types, such as image/*")
    return values


async def pick(*, types=None):
    return _ref(await request("files.pick", types=_types(types), multiple=False))


async def pick_many(*, types=None, limit=10):
    if not isinstance(limit, int) or not 1 <= limit <= 100:
        raise ValueError("limit must be between 1 and 100")
    return [_ref(v) for v in await request("files.pick", types=_types(types), multiple=True, limit=limit)]


async def pick_photo():
    return _ref(await request("files.photo"))


async def save(source, *, suggested_name="file", mime_type="application/octet-stream"):
    if not suggested_name or Path(suggested_name).name != suggested_name:
        raise ValueError("suggested_name must be a filename")
    uri = source.uri if isinstance(source, FileRef) else Path(source).resolve().as_uri()
    return _ref(await request("files.save", source=uri, name=suggested_name, mime=mime_type))


def data_dir():
    return Path(exchange({"op": "info"})["data_dir"])


def cache_dir():
    return Path(exchange({"op": "info"})["cache_dir"])
