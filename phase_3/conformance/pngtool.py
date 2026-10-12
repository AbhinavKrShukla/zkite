"""A dependency-free PNG writer, for generating the test bitmaps `Image` needs.

There is no PIL on this box and adding one would put a native build step in front of
a test suite that currently needs only `python3.14` (phase_3/PLAN.md §35). A PNG of a
handful of flat rectangles is ~40 lines of `zlib` and `struct`, so this writes them
directly rather than pulling in an imaging library to draw four squares.

**Only what the fixtures need**: 8-bit RGBA, colour type 6, no interlacing, one
`IDAT`. That is a real PNG by the spec and every decoder reads it, but this is not a
general encoder and deliberately does not try to be.

Used by `fixtures.py` (the conformance bitmaps, read by both real Chromium and
`vello_cpu`), by `probe_object_fit.mjs`'s setup, and by the device fixture in
`../gpu_harness/demo_app/images.py`.
"""

from __future__ import annotations

import struct
import zlib

RGBA = tuple[int, int, int, int]


def _chunk(tag: bytes, data: bytes) -> bytes:
    return (
        struct.pack(">I", len(data))
        + tag
        + data
        + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
    )


def write_png(path: str, width: int, height: int, pixels: list[list[RGBA]]) -> None:
    """Writes `pixels` (row-major, `height` rows of `width` RGBA tuples) to `path`."""
    if len(pixels) != height or any(len(row) != width for row in pixels):
        raise ValueError(f"pixel grid is not {width}x{height}")
    raw = bytearray()
    for row in pixels:
        # Filter type 0 (None) per scanline. Every other filter would compress
        # better and none of these images is big enough for that to matter.
        raw.append(0)
        for r, g, b, a in row:
            raw += bytes((r, g, b, a))
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(_chunk(b"IHDR", ihdr))
        f.write(_chunk(b"IDAT", zlib.compress(bytes(raw), 9)))
        f.write(_chunk(b"IEND", b""))


def solid(path: str, width: int, height: int, colour: RGBA) -> None:
    """One flat colour — the shape that makes a placement bounding box readable."""
    write_png(path, width, height, [[colour] * width for _ in range(height)])


def quadrants(path: str, width: int, height: int) -> None:
    """Four distinct opaque quadrants: red TL, green TR, blue BL, yellow BR.

    **This is the orientation control.** A solid bitmap makes a placement error
    visible and a *flip* invisible — an image drawn upside down has the identical
    bounding box. Four different corners cannot be transposed, mirrored or rotated
    without the diff moving, which is the failure mode a paint transform gets wrong
    first (the y axis points the other way in half the coordinate systems involved).
    """
    tl, tr = (220, 38, 38, 255), (22, 163, 74, 255)
    bl, br = (37, 99, 235, 255), (234, 179, 8, 255)
    half_w, half_h = width // 2, height // 2
    rows = []
    for y in range(height):
        left, right = (tl, tr) if y < half_h else (bl, br)
        rows.append([left] * half_w + [right] * (width - half_w))
    write_png(path, width, height, rows)


def read_png_rgba(path: str) -> tuple[int, int, list[list[RGBA]]]:
    """Decodes the subset this module writes, plus whatever else happens to be 8-bit
    RGBA non-interlaced — enough to measure a screenshot without PIL.

    Supports all five PNG filter types, because a *screenshot* is not written by this
    module and Chromium/`screencap` both use them. §34 wrote a reader like this and
    threw it away; this one is kept, because every `Image` measurement in §35 is
    arithmetic out of a PNG and rewriting it per session is how a measurement bug
    survives.
    """
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(f"{path} is not a PNG")
    pos, idat, w, h = 8, bytearray(), 0, 0
    depth = colour_type = interlace = 0
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        tag = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if tag == b"IHDR":
            w, h, depth, colour_type, _comp, _filt, interlace = struct.unpack(">IIBBBBB", body)
        elif tag == b"IDAT":
            idat += body
        elif tag == b"IEND":
            break
        pos += 12 + length
    if depth != 8 or colour_type not in (2, 6) or interlace != 0:
        raise ValueError(
            f"{path}: only 8-bit RGB/RGBA, non-interlaced is supported "
            f"(got depth={depth} colour_type={colour_type} interlace={interlace})"
        )
    channels = 4 if colour_type == 6 else 3
    raw = zlib.decompress(bytes(idat))
    stride = w * channels
    out: list[list[RGBA]] = []
    prev = bytearray(stride)
    p = 0
    for _ in range(h):
        ftype = raw[p]
        p += 1
        line = bytearray(raw[p : p + stride])
        p += stride
        for i in range(stride):
            a = line[i - channels] if i >= channels else 0
            b = prev[i]
            c = prev[i - channels] if i >= channels else 0
            if ftype == 1:
                line[i] = (line[i] + a) & 0xFF
            elif ftype == 2:
                line[i] = (line[i] + b) & 0xFF
            elif ftype == 3:
                line[i] = (line[i] + (a + b) // 2) & 0xFF
            elif ftype == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 0xFF
            elif ftype != 0:
                raise ValueError(f"{path}: unknown filter type {ftype}")
        row: list[RGBA] = []
        for x in range(w):
            px = line[x * channels : x * channels + channels]
            row.append((px[0], px[1], px[2], px[3] if channels == 4 else 255))
        out.append(row)
        prev = line
    return w, h, out
