import asyncio
import json
import threading
import time
import types
import unittest
from zkite import App, Button, store
from zkite import _async, _device, files, sensors
from zkite.errors import DeviceUnavailable, PermissionDenied


class Backend:
    def __init__(self):
        self.events = []
        self.requests = []
        self.lock = threading.Lock()

    def __call__(self, raw):
        q = json.loads(raw)
        with self.lock:
            if q['op'] == 'poll':
                events, self.events = self.events, []
                return json.dumps(events)
            self.requests.append(q)
        return 'null'

    def reply(self, **event):
        with self.lock:
            self.events.append(event)


class Services(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.backend = Backend()
        _device.install_backend(self.backend)

    def tearDown(self):
        _device.install_backend(None)
        _async.cancel_all()

    async def wait_request(self, operation):
        for _ in range(200):
            match = [q for q in self.backend.requests if q['op'] == operation]
            if match:
                return match[-1]
            await asyncio.sleep(.005)
        self.fail('request never submitted')

    async def test_picker_cancellation_and_permission_error(self):
        task = asyncio.create_task(files.pick())
        q = await self.wait_request('files.pick')
        self.backend.reply(id=q['id'], value=None)
        self.assertIsNone(await task)
        task = asyncio.create_task(files.pick())
        await asyncio.sleep(.01)
        q = await self.wait_request('files.pick')
        self.backend.reply(id=q['id'], error='PermissionDenied', message='denied')
        with self.assertRaises(PermissionDenied):
            await task

    async def test_cancel_ignores_late_response(self):
        task = asyncio.create_task(files.pick())
        q = await self.wait_request('files.pick')
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.backend.reply(id=q['id'], value={'name': 'stale'})
        await asyncio.sleep(.05)
        self.assertNotIn(q['id'], _device._pending)
        self.assertTrue(any(r['op']=='cancel' and r['id']==q['id'] for r in self.backend.requests))

    async def test_timeout_cleans_native_request(self):
        with self.assertRaises(TimeoutError):
            await _device.request('test.wait', timeout=.02)
        self.assertFalse(_device._pending)
        self.assertTrue(any(r['op']=='cancel' for r in self.backend.requests))

    async def test_repeated_requests_do_not_lose_polling(self):
        for i in range(15):
            task = asyncio.create_task(_device.request('test.echo', timeout=1))
            await asyncio.sleep(.005)
            q = self.backend.requests[-1]
            self.backend.reply(id=q['id'], value=i)
            self.assertEqual(await task, i)

    async def test_no_backend_is_explicit(self):
        _device.install_backend(None)
        with self.assertRaises(DeviceUnavailable):
            await files.pick()

    async def test_sensor_delivers_and_closes(self):
        received = threading.Event()
        task = asyncio.create_task(sensors.accelerometer.watch(lambda sample: received.set()))
        q = await self.wait_request('sensors.watch')
        self.backend.reply(id=q['id'], value=77)
        sub = await task
        self.backend.reply(subscription=77, value={'values':[1,2,3], 'timestamp_ns':123, 'accuracy':3})
        for _ in range(100):
            if received.is_set(): break
            await asyncio.sleep(.01)
        self.assertTrue(received.is_set())
        sub.close()
        self.assertNotIn(77, _device._subscriptions)


class AsyncEvents(unittest.TestCase):
    def tearDown(self):
        _async.cancel_all()

    def test_handler_actually_runs(self):
        ran = threading.Event()
        async def tapped():
            await asyncio.sleep(.01)
            ran.set()
        module = types.ModuleType('async_probe')
        module.view = lambda: Button('', 'go', on_tap=tapped)
        app = App(module)
        app.render()
        self.assertTrue(app.dispatch('r', 'tap'))
        self.assertTrue(ran.wait(2))

    def test_old_handler_cannot_mutate_store_after_reload(self):
        @store
        class State:
            value: int = 0
        state = State()
        started, finished = threading.Event(), threading.Event()
        async def operation():
            started.set()
            try:
                await asyncio.sleep(30)
            finally:
                state.update(value=99)
                finished.set()
        _async.submit(operation())
        self.assertTrue(started.wait(2))
        _async.cancel_all()
        self.assertTrue(finished.wait(2))
        self.assertEqual(state.value, 0)
