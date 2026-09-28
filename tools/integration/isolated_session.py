#!/usr/bin/env python3
"""Run a desktop integration command with its own D-Bus and XDG runtime.

Usage: python3 tools/integration/isolated_session.py -- COMMAND [ARG ...]
The login session's runtime directory must never be shared with a private bus:
its document portal FUSE mount can otherwise be displaced by a second portal.
"""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time


def login_document_mount():
    """Return the login mount ID, or None if no document portal is mounted."""
    target = f'/run/user/{os.getuid()}/doc'
    for line in Path('/proc/self/mountinfo').read_text().splitlines():
        parts = line.split(' ', 5)
        if len(parts) > 4 and parts[4] == target:
            return parts[0]
    return None


def private_mounts(runtime):
    prefix = str(runtime) + '/'
    mounts = []
    for line in Path('/proc/self/mountinfo').read_text().splitlines():
        parts = line.split(' ', 5)
        if len(parts) > 4 and parts[4].startswith(prefix):
            mounts.append(Path(parts[4]))
    return sorted(mounts, key=lambda path: len(path.parts), reverse=True)


def clean_private_runtime(runtime):
    # D-Bus activated FUSE helpers can take a moment to release their mount or
    # briefly leave a disconnected endpoint after the private bus shuts down.
    for _ in range(30):
        if not private_mounts(runtime):
            try:
                shutil.rmtree(runtime)
                return True
            except OSError:
                pass
        time.sleep(.1)
    for mount in private_mounts(runtime):
        subprocess.run(['fusermount3', '-uz', str(mount)], check=False, capture_output=True)
    for _ in range(30):
        if not private_mounts(runtime):
            try:
                shutil.rmtree(runtime)
                return True
            except OSError:
                pass
        time.sleep(.1)
    return False


def require_isolated_session():
    """Fail before GTK/GIO starts when a test bypasses the safe launcher."""
    value = os.environ.get('XDG_RUNTIME_DIR')
    marker = os.environ.get('SHUTTLI_TEST_RUNTIME_DIR')
    if not value or marker != value:
        raise RuntimeError('Use tools/integration/isolated_session.py to run desktop integration tests')
    runtime = Path(value).resolve()
    login_runtime = Path(f'/run/user/{os.getuid()}').resolve()
    if runtime == login_runtime or login_runtime in runtime.parents or not (runtime / '.shuttli-test-runtime').is_file():
        raise RuntimeError('Desktop integration test runtime is not isolated from the login session')
    if os.environ.get('DBUS_SESSION_BUS_ADDRESS') == f'unix:path={login_runtime}/bus':
        raise RuntimeError('Desktop integration test is using the login D-Bus session')


def main(argv):
    if len(argv) < 2 or argv[0] != '--':
        raise SystemExit(__doc__)
    before = login_document_mount()
    runtime = Path(tempfile.mkdtemp(prefix='shuttli-desktop-test-'))
    (runtime / '.shuttli-test-runtime').touch()
    env = dict(os.environ)
    env['XDG_RUNTIME_DIR'] = str(runtime)
    env['SHUTTLI_TEST_RUNTIME_DIR'] = str(runtime)
    env['GIO_USE_PORTALS'] = '0'
    env['GTK_USE_PORTAL'] = '0'
    env.pop('DBUS_SESSION_BUS_ADDRESS', None)
    env.pop('WAYLAND_DISPLAY', None)
    code = 1
    try:
        code = subprocess.run(['dbus-run-session', '--', *argv[1:]], env=env, check=False).returncode
    finally:
        if not clean_private_runtime(runtime):
            print(f'Could not clean private test runtime {runtime}; leaving it untouched', file=sys.stderr)
            code = 1
        if login_document_mount() != before:
            print('Login document portal mount changed during the test', file=sys.stderr)
            code = 1
    return code


if __name__ == '__main__':
    raise SystemExit(main(sys.argv[1:]))
