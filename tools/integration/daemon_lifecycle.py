#!/usr/bin/env python3
"""Synthetic daemon/CLI lifecycle checks. Run ONLY under isolated Xvfb/D-Bus.

Uses a temporary profile and disabled discovery; never reads a user's clipboard
or changes their startup registration. Execution evidence stays outside the repo.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import tempfile
import struct
import zlib
import time
from isolated_session import require_isolated_session


def main():
    require_isolated_session()
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    args = parser.parse_args()
    binary = str(args.binary.resolve())
    with tempfile.TemporaryDirectory(prefix='shuttli-lifecycle-') as directory:
        root = Path(directory)
        env = dict(os.environ, SHUTTLI_DATA_DIR=str(root / 'profile'),
                   XDG_CONFIG_HOME=str(root / 'config'), SHUTTLI_TAILSCALE=shutil.which('false'))
        daemon = None
        groups = []
        def cli(*command):
            result = subprocess.run([binary, *command, '--json'], env=env,
                                    capture_output=True, text=True, timeout=15)
            return result.returncode, json.loads(result.stdout)
        def wait(check):
            deadline = time.monotonic() + 8
            while time.monotonic() < deadline:
                result = check()
                if result:
                    return result
                time.sleep(.05)
            raise AssertionError('lifecycle condition timed out')
        def start(log):
            process = subprocess.Popen([binary, 'daemon'], env=env, stdout=log,
                                       stderr=log, start_new_session=True)
            groups.append(process.pid)
            wait(lambda: cli('status')[1].get('type') == 'status')
            return process
        try:
            with (root / 'daemon.log').open('w') as log:
                daemon = start(log)
                original = cli('settings')[1]['settings']
                assert cli('set', 'notifications', 'off')[0] == 0
                subprocess.run(['xclip', '-selection', 'clipboard'], input=b'Synthetic lifecycle copy', env=env, check=True)
                wait(lambda: cli('history')[1].get('entries'))
                def chunk(kind, data):
                    return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
                image = (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', 1, 1, 8, 6, 0, 0, 0)) +
                         chunk(b'IDAT', zlib.compress(b'\x00\xff\x00\x00\xff')) + chunk(b'IEND', b''))
                subprocess.run(['xclip', '-selection', 'clipboard', '-target', 'image/png'], input=image, env=env, check=True)
                wait(lambda: list((root / 'profile/history-images').glob('*.enc')))
                assert cli('set', 'send', 'off')[0] == 0
                edited = dict(original, history_limit=7)
                code, response = cli('request', json.dumps({'version': 2, 'action': {
                    'command': 'configure', 'expected': original, 'settings': edited}}))
                assert code != 0 and response['type'] == 'error'
                saved = cli('settings')[1]['settings']
                assert saved['send'] is False
                assert cli('quit')[0] == 0
                assert daemon.wait(timeout=12) == 0
                assert not (root / 'profile/control.sock').exists()
                assert cli('status')[1]['type'] == 'stopped'
                assert not list((root / 'profile/history-images').glob('*.enc'))
                daemon = start(log)
                assert cli('settings')[1]['settings'] == saved
                assert cli('history')[1]['entries'] == []
                assert cli('quit')[0] == 0
                assert daemon.wait(timeout=12) == 0
                print('PASS real daemon/CLI: local history, stale-write rejection, quit, socket cleanup and restart preferences')
        except Exception:
            print((root / 'daemon.log').read_text()[-8000:])
            raise
        finally:
            for group in groups:
                try:
                    os.killpg(group, signal.SIGTERM)
                except ProcessLookupError:
                    pass
            if daemon is not None and daemon.poll() is None:
                daemon.wait(timeout=5)


if __name__ == '__main__':
    main()
