"""Desktop test launcher must not inherit the login portal runtime or D-Bus."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/integration'))
from isolated_session import login_document_mount, require_isolated_session


@unittest.skipUnless(sys.platform == 'linux' and shutil.which('dbus-run-session'), 'Linux D-Bus required')
class IsolatedSessionTests(unittest.TestCase):
    def test_direct_desktop_test_is_rejected_before_gio_import(self):
        with self.assertRaisesRegex(RuntimeError, 'isolated_session.py'):
            require_isolated_session()
        result = subprocess.run([sys.executable, str(ROOT / 'tools/integration/linux_ui.py')],
                                capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('isolated_session.py', result.stderr)

    def test_launcher_creates_private_runtime_and_bus_then_cleans_up(self):
        before = login_document_mount()
        code = '''import json, os, pathlib
from isolated_session import require_isolated_session
require_isolated_session()
runtime=pathlib.Path(os.environ['XDG_RUNTIME_DIR'])
print(json.dumps({'runtime':str(runtime), 'mode':runtime.stat().st_mode & 0o777,
                  'bus':os.environ['DBUS_SESSION_BUS_ADDRESS']}))'''
        result = subprocess.run([sys.executable, str(ROOT / 'tools/integration/isolated_session.py'),
                                 '--', sys.executable, '-c', code],
                                cwd=ROOT / 'tools/integration', capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        data = json.loads(result.stdout)
        self.assertEqual(data['mode'], 0o700)
        self.assertNotEqual(data['runtime'], f'/run/user/{os.getuid()}')
        self.assertNotIn(f'/run/user/{os.getuid()}/bus', data['bus'])
        self.assertFalse(Path(data['runtime']).exists())
        self.assertEqual(login_document_mount(), before)


if __name__ == '__main__':
    unittest.main()
