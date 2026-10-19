import asyncio
import http.server
from pathlib import Path
import tempfile
import threading
import unittest
from zkite.network import upload, HTTPError


class UploadTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.received = None
        self.status = 201
        case = self
        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self):
                case.received = self.rfile.read(int(self.headers['Content-Length']))
                self.send_response(case.status)
                self.end_headers()
                self.wfile.write(b'{"saved":true}')
            def log_message(self, *args): pass
        self.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.directory = tempfile.TemporaryDirectory()
        self.file = Path(self.directory.name) / 'fixture.bin'
        self.file.write_bytes(bytes(range(256)) * 1024)
        self.url = 'http://127.0.0.1:%d/upload' % self.server.server_port

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()
        self.directory.cleanup()

    async def test_streams_exact_bytes_and_progress(self):
        progress = []
        response = await upload(self.url, self.file, allow_http=True,
                                on_progress=lambda sent, total: progress.append((sent,total)))
        await asyncio.sleep(.01)
        self.assertEqual(response.status, 201)
        self.assertEqual(response.json(), {'saved': True})
        self.assertIn(self.file.read_bytes(), self.received)
        self.assertIn(b'name="file"; filename="fixture.bin"', self.received)
        self.assertEqual(progress[-1][0], progress[-1][1])

    async def test_http_requires_opt_in(self):
        with self.assertRaises(ValueError):
            await upload(self.url, self.file)
        self.assertIsNone(self.received)

    async def test_non_success_is_not_silently_accepted(self):
        self.status = 403
        with self.assertRaises(HTTPError) as caught:
            await upload(self.url, self.file, allow_http=True)
        self.assertEqual(caught.exception.response.status, 403)

    async def test_response_size_and_input_limits(self):
        with self.assertRaises(ValueError):
            await upload(self.url, self.file, allow_http=True, max_bytes=1)
        with self.assertRaises(OSError):
            await upload(self.url, self.file, allow_http=True, max_response_bytes=1)
