import base64
import io
import threading
from unittest.mock import patch

from zkite import _remote_images as remote
from zkite import errors


def test_release_errors_hide_details_in_both_renderers():
    with patch.object(errors, 'DEBUG', False):
        html = errors.format_error_overlay('SECRET traceback')
        tree = errors.format_error_tree('SECRET traceback')
        assert 'SECRET' not in html
        assert 'SECRET' not in str(tree)
        assert 'Please restart' in html
    with patch.object(errors, 'DEBUG', True):
        assert 'SECRET' in errors.format_error_overlay('SECRET')
        assert 'SECRET' in str(errors.format_error_tree('SECRET'))


def test_https_download_runs_off_render_thread_and_reuses_cache():
    url = 'https://images.example/test.png'
    data = b'\x89PNG\r\n\x1a\nfixture'
    complete = threading.Event()
    unblock = threading.Event()
    calls = []
    class Opener:
        def open(self, request, timeout):
            calls.append(threading.current_thread().name)
            assert unblock.wait(5)
            return io.BytesIO(data)
    with patch.object(remote.urllib.request, 'build_opener', return_value=Opener()), patch.object(remote._store, '_RENDER_CALLBACK', complete.set):
        assert remote.source(url) == ''
        assert remote.source(url) == ''
        unblock.set()
        assert complete.wait(5)
        assert remote.source(url) == 'data:image/png;base64,' + base64.b64encode(data).decode()
        assert len(calls) == 1
        assert calls[0].startswith('zkite-image')


def test_invalid_images_are_not_cached_as_pixels():
    url = 'https://images.example/not-an-image'
    class Opener:
        def open(self, *args, **kwargs):
            return io.BytesIO(b'<html>not an image</html>')
    with patch.object(remote.urllib.request, 'build_opener', return_value=Opener()), patch.object(remote._store, '_RENDER_CALLBACK', None):
        remote._download(url)
        assert remote.source(url) == ''
    assert remote.source('assets/logo.png') == 'assets/logo.png'
    assert remote.source('http://images.example/test.png') == ''


def test_https_redirect_cannot_downgrade_to_cleartext():
    import pytest
    with pytest.raises(ValueError, match='HTTPS'):
        remote._HTTPSRedirect().redirect_request(None, None, 302, '', {}, 'http://example.com/a.png')
