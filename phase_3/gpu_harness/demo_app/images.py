"""`Image` on a device — phase_3/PLAN.md §35.

Every row is readable off a screenshot with arithmetic, in the style §34's
`network.py` established: fixed-size boxes at known offsets, filled with colours that
cannot be confused for each other or for the ground.

The bitmap is `quad40x20.png` — 40x20, four distinct opaque quadrants (red TL, green
TR, blue BL, yellow BR). Two properties are doing work:

- **Non-square (2:1)**, so `contain`/`cover`/`fill` are three visibly different
  answers inside one square box. A square source would make `contain` and `cover`
  agree and hide a swapped `min`/`max`.
- **Four different corners**, so a flip or a transpose moves the picture. A flat
  colour has the same bounding box upside down, which is the failure a paint
  transform gets wrong first — the y axis points the other way in half the coordinate
  systems between a PNG scanline and a vello scene.

Run it on the harness with:

    ZKITE_DEVSHELL= ABI=arm64-v8a \\
      ZKITE_APP_MAIN=phase_3/gpu_harness/demo_app/images.py \\
      phase_3/gpu_harness/scripts/build_apk.sh

and on the WebView backend by copying it to a `zkite new` project (§33.6's method) —
`build_apk.sh` and `zkite build apk` stage `*.png` next to the app with the same
allowlist, so the same file reaches both.
"""

from zkite import Column, Image, Row, Text

BITMAP = "quad40x20.png"

# One 56dp box per `object-fit` mode, on white so a letterboxed `contain` is visible
# against something. `Image.implied_classes` is `object-cover`, and an authored
# `object-*` replaces it rather than fighting it in the cascade (§35.4) — so each of
# these five really is the mode it names, on both backends.
FITS = ["object-fill", "object-contain", "object-cover", "object-none", "object-scale-down"]


def view():
    return Column(
        "flex-1 bg-slate-950 p-4 gap-3",
        Text("text-white text-lg font-bold", "images"),

        # Row 1 — the five modes. Expected geometry inside a 56x56 box, from a 40x20
        # source: fill 56x56 stretched; contain 56x28 centred; cover 112x56 cropped
        # to the box; none 40x20 centred; scale-down 40x20 centred (the source is
        # smaller than the box, so it must NOT be enlarged — this is the only box that
        # distinguishes scale-down from contain).
        Text("text-slate-400 text-xs", "fill / contain / cover / none / scale-down"),
        Row(
            "gap-2",
            *[
                Image(f"w-[56px] h-[56px] bg-white {fit}", src=BITMAP, alt=fit)
                for fit in FITS
            ],
        ),

        # Row 2 — a rounded avatar, the shape that exercises the clip path, next to a
        # second bitmap so two different images are resident at once (the atlas holds
        # both, and one is not the other).
        Text("text-slate-400 text-xs", "rounded + a second bitmap"),
        Row(
            "gap-3 items-center",
            Image("w-[56px] h-[56px] rounded-full bg-white", src=BITMAP, alt="avatar"),
            Image("w-[40px] h-[40px] rounded-lg", src="cyan16.png", alt="cyan"),
            Text("text-white text-sm", "two images"),
        ),

        # Row 3 — the failure modes, each of which must paint *nothing* and leave the
        # app running. A red ground behind them makes "painted nothing" and "painted
        # something wrong" different pixels rather than both being dark.
        Text("text-slate-400 text-xs", "missing / escaping / empty — all must stay red"),
        Row(
            "gap-2",
            Image("w-[40px] h-[40px] bg-red-600", src="nope.png", alt="missing"),
            Image("w-[40px] h-[40px] bg-red-600", src="../../../etc/hosts", alt="escape"),
            Image("w-[40px] h-[40px] bg-red-600", src="", alt="empty"),
        ),

        # Row 4 — the same `src` sixteen times. One decode and one atlas slot serve
        # all of them (`ImageStore` is keyed by `src`), so this row is the visible
        # half of the cache claim; the `images:` log line carries the numeric half.
        Text("text-slate-400 text-xs", "16 nodes, one decode"),
        Row(
            "gap-1 flex-wrap",
            *[Image("w-[16px] h-[16px]", src=BITMAP, alt="") for _ in range(16)],
        ),
    )
