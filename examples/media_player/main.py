"""Pick local media; style native video overlays and ordinary Python controls."""
from zkite import Button, Column, ScrollView, Text, store
from zkite import audio, video, media, files, links

@store
class Screen:
    player: object = None
    snapshot: object = None
    watcher: object = None
    message: str = "Pick audio or video"

async def close(s):
    if s.watcher is not None:
        s.watcher.cancel()
    if s.player is not None:
        await s.player.stop()
    s.update(player=None, snapshot=None, watcher=None)

async def pick(s, movie=False):
    # Remove the native surface before opening Android's picker.
    await close(s)
    source = await files.pick(types=["video/*" if movie else "audio/*"])
    if source is None:
        return
    if movie:
        player = await video.play(source, bounds=(0, 64, 320, 180),
            control_style={"accent": "#34D399", "auto_hide_ms": 3000})
    else:
        player = await audio.play(source)
    s.update(player=player, snapshot=await player.state(), message=source.name)
    watcher = player.watch(lambda snapshot: s.update(snapshot=snapshot))
    s.update(watcher=watcher)

async def opened(link, s: Screen):
    if link.scheme == "zkitemedia" and link.host == "open":
        s.update(message="Opened route: " + link.path)

listener = links.listen(opened)

def view(s: Screen):
    return Column("flex-1 bg-slate-950",
        Text("p-4 text-white", s.message),
        Column("h-[220px]"),  # Reserve space for Android's video surface.
        ScrollView("flex-1", Column("p-4 gap-3",
            Button("bg-indigo-600 text-white p-3 rounded-xl", "Pick audio", on_tap=lambda: pick(s)),
            Button("bg-indigo-600 text-white p-3 rounded-xl", "Pick video", on_tap=lambda: pick(s, True)),
            media.Controls("gap-3", player=s.player, state=s.snapshot,
                parts=("time", "seek", "buttons", "volume", "speed", "loop", "status"),
                button_class="bg-emerald-700 text-white rounded-xl px-3 py-2",
                on_close=lambda: close(s)) if s.player and s.snapshot else None)))
