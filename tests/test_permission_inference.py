import importlib.util
from pathlib import Path
import unittest

path = Path(__file__).resolve().parents[1] / 'phase_1/cli/python/infer_permissions.py'
spec = importlib.util.spec_from_file_location('permission_analyzer', path)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class Permissions(unittest.TestCase):
    def infer(self, source, **extra):
        return module.analyze({'main.py': source, **extra})

    def test_aliases_callbacks_and_local_reexports(self):
        result = self.infer('from helpers import rec\nfrom zkite import haptics as h\ncallback = rec\nh.vibrate(duration_ms=100)',
                            **{'helpers.py': 'from zkite.audio import record as rec'})
        self.assertEqual(result['permissions'], ['RECORD_AUDIO', 'VIBRATE'])

    def test_nested_imports_do_not_hide_other_scope(self):
        result = self.infer('def one():\n from zkite import camera\n camera.torch()\ndef two():\n import unrelated as camera\n camera.torch()')
        self.assertEqual(result['permissions'], ['CAMERA'])

    def test_file_pickers_external_camera_and_feedback_need_no_dangerous_permission(self):
        result = self.infer('from zkite import files,camera,haptics\nfiles.pick()\ncamera.capture()\nhaptics.perform("confirm")')
        self.assertEqual(result['permissions'], [])

    def test_imports_comments_and_strings_are_not_usage(self):
        result = self.infer('from zkite import audio\n# audio.record()\ntext="camera.torch()"')
        self.assertEqual(result['permissions'], [])

    def test_location_accuracy_and_explicit_permission(self):
        result = self.infer('from zkite import location,permissions\nlocation.current(high_accuracy=True)\npermissions.ask("RECORD_AUDIO")')
        self.assertEqual(result['permissions'], ['ACCESS_COARSE_LOCATION','ACCESS_FINE_LOCATION','RECORD_AUDIO'])

    def test_star_import_and_literal_getattr(self):
        result = self.infer('from zkite.audio import *\nrecord()\nimport zkite\ngetattr(zkite.camera,"torch")(True)')
        self.assertEqual(result['permissions'], ['CAMERA','RECORD_AUDIO'])

    def test_remote_images_and_network_clients(self):
        result = self.infer('from zkite import Image\nImage("",src="https://example.com/a.jpg")\nimport requests as r\nr.get("https://example.com")')
        self.assertEqual(result['permissions'], ['INTERNET'])

    def test_media_permissions_distinguish_local_remote_and_dynamic(self):
        self.assertEqual(self.infer('from zkite import audio,video\naudio.play("assets/a.mp3")\nvideo.play("content://media/42")')['permissions'], [])
        self.assertEqual(self.infer('from zkite.video import play as v\nv("https://example.com/a.mp4")')['permissions'], ['INTERNET'])
        self.assertEqual(self.infer('from zkite import audio\naudio.play(source=url)')['permissions'], ['INTERNET'])

    def test_dynamic_code_reports_limits(self):
        result = self.infer('from zkite import permissions\npermissions.ask(variable)\nexec(code)')
        self.assertEqual(len(result['warnings']), 2)

    def test_no_execution_and_syntax_failure(self):
        self.infer('raise Exception("must not run")')
        with self.assertRaises(SyntaxError): self.infer('def broken(')

if __name__ == '__main__': unittest.main()
