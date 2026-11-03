#!/usr/bin/env python3
"""Explicit user-agent restart fixture. State must live outside the checkout."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--binary', required=True)
p.add_argument('--pid', type=int, required=True)
p.add_argument('--offline-seconds', type=float, default=3)
p.add_argument('--state', type=Path, required=True)
p.add_argument('--allow-restart', action='store_true', required=True)
a = p.parse_args()
proc = Path('/proc') / str(a.pid)
if proc.stat().st_uid != os.getuid():
    raise SystemExit('agent must belong to this user')
if proc.joinpath('exe').resolve() != Path(a.binary).resolve():
    raise SystemExit('agent binary does not match requested binary')
env = dict(os.environ)
allowed = {b'DISPLAY', b'XAUTHORITY', b'XDG_RUNTIME_DIR', b'WAYLAND_DISPLAY',
           b'DBUS_SESSION_BUS_ADDRESS', b'LANG', b'SHUTTLI_DATA_DIR'}
for entry in proc.joinpath('environ').read_bytes().split(b'\0'):
    key, sep, value = entry.partition(b'=')
    if sep and key in allowed:
        env[key.decode()] = value.decode()
a.state.parent.mkdir(parents=True, exist_ok=True)
a.state.write_text(json.dumps({'binary': a.binary, 'environment': {k: v for k, v in env.items() if k.encode() in allowed}}))
subprocess.run([a.binary, 'quit', '--json'], env=env, check=True, stdout=subprocess.DEVNULL)
deadline = time.monotonic() + 8
while proc.exists() and time.monotonic() < deadline:
    time.sleep(.1)
if proc.exists():
    raise SystemExit('agent did not exit; no duplicate started')
print(json.dumps({'stopped': True}), flush=True)
try:
    time.sleep(max(0, min(a.offline_seconds, 15)))
finally:
    with a.state.with_suffix('.log').open('ab') as log:
        child = subprocess.Popen([a.binary, 'daemon'], env=env, stdin=subprocess.DEVNULL,
                                 stdout=log, stderr=log, start_new_session=True)
    print(json.dumps({'pid': child.pid}), flush=True)
