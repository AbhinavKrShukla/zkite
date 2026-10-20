import asyncio
import unittest
from unittest.mock import AsyncMock, patch
from zkite import audio, video, links, media, _device
from zkite.errors import OperationInterrupted
from zkite.widgets import RenderContext


class MediaLinks(unittest.IsolatedAsyncioTestCase):
    async def test_network_sources_stay_urls_and_controls_share_handle(self):
        with patch.object(_device, "request", new_callable=AsyncMock, return_value=7) as request:
            player = await audio.play("https://example.org/song.mp3?x=1")
            self.assertEqual(request.call_args.kwargs["uri"], "https://example.org/song.mp3?x=1")
            await player.configure(volume=.2, speed=1.5, loop=True)
            request.assert_awaited_with("audio.configure", handle=7, volume=.2, speed=1.5, loop=True)
            await player.stop()
            await player.stop()
            with self.assertRaises(OperationInterrupted): await player.resume()

    async def test_video_and_invalid_inputs(self):
        with patch.object(_device, "request", new_callable=AsyncMock, return_value=8) as request:
            p = await video.play("content://media/42", bounds=(1,2,320,180), controls=False)
            self.assertEqual(request.call_args.args, ("video.play",))
            self.assertEqual(request.call_args.kwargs["uri"], "content://media/42")
            await p.fullscreen()
            request.assert_awaited_with("video.fullscreen", handle=8, enabled=True)
            await p.bounds(0,0,640,360)
            request.assert_awaited_with("video.bounds", handle=8, bounds=[0,0,640,360])
            for kwargs in ({"volume":float('nan')}, {"fit":"wrong"}, {"bounds":(0,0,0,1)}, {"control_style":{"accent":"red"}}, {"control_style":{"bogus":True}}):
                with self.assertRaises(ValueError): await video.play("movie.mp4", **kwargs)
            for value in (-1, float('inf'), float('nan')):
                with self.assertRaises(ValueError): await p.seek(value)
            p._generation -= 1
            with self.assertRaises(OperationInterrupted): await p.configure(volume=.2)

    def test_link_encoding_and_queue_consumption(self):
        url=links.create("demo","open","/item/a b", tag=["a&b","c"], empty="")
        link=links.Link(url)
        self.assertEqual(link.path,"/item/a%20b")
        self.assertEqual(link.query,{"tag":["a&b","c"],"empty":[""]})
        with patch.object(_device,"exchange", side_effect=[{"url":url},{"url":None}]):
            self.assertEqual(links.next_link(),link)
            self.assertIsNone(links.next_link())
        with self.assertRaises(ValueError): links.create("bad scheme","open")

    async def test_listener_preserves_order_and_injects_stores(self):
        import threading
        from zkite import store
        received = []
        done = threading.Event()
        @store
        class Route:
            count: int = 0
        async def callback(link, route: Route):
            route.count += 1
            received.append((link.path, route.count))
            if len(received) == 2: done.set()
        queue = [links.Link("demo://open/first"), links.Link("demo://open/second")]
        with patch.object(links, "next_link", side_effect=lambda: queue.pop(0) if queue else None):
            listener = links.listen(callback, interval=.025)
            try:
                self.assertTrue(await asyncio.to_thread(done.wait, 2))
                self.assertEqual(received, [("/first",1), ("/second",2)])
            finally:
                listener.cancel()

    def test_controls_are_composable_and_dispatch_to_playback(self):
        player=audio.Playback(1)
        controls=media.Controls("gap-2",player=player,state={"state":"playing","position":3,"duration":20}, parts=("time","seek","buttons","loop"), labels={"pause":"Hold"})
        ctx=RenderContext()
        html=controls.render(ctx,"r")
        self.assertIn("Hold", html)
        self.assertIn("0:03 / 0:20", html)
        self.assertTrue(ctx.handlers)
        with self.assertRaises(ValueError): media.Controls(player=player,state={},parts=("invalid",))
