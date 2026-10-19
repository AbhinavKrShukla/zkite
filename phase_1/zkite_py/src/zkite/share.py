from pathlib import Path
from ._device import request
from .files import FileRef


async def send(*, files=(), text="", mime_type="*/*"):
    uris = [f.uri if isinstance(f, FileRef) else Path(f).resolve().as_uri() for f in files]
    await request("share.send", uris=uris, text=text, mime=mime_type)
