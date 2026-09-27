#!/usr/bin/env python3
"""Inject sender-side lost durable ACK in an isolated Linux profile.

This is a persisted-state fault injection, not a dropped network packet test.
Stops/restarts the specified Linux test agent, overwrites the Mac clipboard with
synthetic content, then proves STATUS recovery does not reapply the old content.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import signal
import sqlite3
import subprocess
import time

p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--ssh', required=True)
p.add_argument('--local-bin', required=True)
p.add_argument('--remote-bin', required=True)
p.add_argument('--local-data', required=True)
p.add_argument('--remote-data', required=True)
p.add_argument('--local-pid', type=int, required=True)
p.add_argument('--allow-test-profile-mutation', action='store_true', required=True)
p.add_argument('--report', required=True)
a = p.parse_args()
env = dict(os.environ, SHUTTLI_DATA_DIR=a.local_data)

def remote(binary, *args):
    return ['ssh', '-oBatchMode=yes', a.ssh, shlex.join(['env', 'SHUTTLI_DATA_DIR='+a.remote_data, binary, *args])]

def api(binary, *args, mac=False):
    command = remote(binary, *args, '--json') if mac else [binary, *args, '--json']
    r = subprocess.run(command, env=env, capture_output=True, timeout=25, check=True)
    result = json.loads(r.stdout)
    assert result['type'] != 'error', result.get('message')
    return result

# Refuse to signal an unrelated process/profile.
proc = Path('/proc')/str(a.local_pid)
assert Path(os.readlink(proc/'exe')).resolve() == Path(a.local_bin).resolve()
assert b'daemon' in (proc/'cmdline').read_bytes().split(b'\0')
assert ('SHUTTLI_DATA_DIR='+a.local_data).encode() in (proc/'environ').read_bytes().split(b'\0')
for binary, mac in ((a.local_bin, False), (a.remote_bin, True)):
    settings = api(binary, 'settings', mac=mac)['settings']
    assert not settings['automatic'], 'disable automatic sync on test profiles first'
mac_id = api(a.remote_bin, 'status', mac=True)['status']['device']
with sqlite3.connect(Path(a.local_data)/'state.sqlite3') as db:
    selected = db.execute("SELECT event_key,event FROM outbox WHERE peer=? AND state=? ORDER BY rowid DESC LIMIT 1", (mac_id, '"applied"')).fetchone()
assert selected, 'need a completed test delivery first'
event_key, event_json = selected
fixture = str(Path(a.remote_bin).parent/'examples/clipboard_fixture')
child = subprocess.Popen(remote(fixture, 'text', 'Shuttli newer local copy during ACK recovery'), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
line = child.stdout.readline().decode().strip()
assert 'fixture set;' in line, 'failed to write synthetic Mac clipboard'
expected = line.split('digest=')[1]
log = None
stopped = False
restarted = False
try:
    os.kill(a.local_pid, signal.SIGTERM)
    stopped = True
    until = time.monotonic()+10
    while proc.exists() and time.monotonic()<until:
        time.sleep(.1)
    assert not proc.exists(), 'test agent did not stop'
    with sqlite3.connect(Path(a.local_data)/'state.sqlite3') as db:
        changed = db.execute('UPDATE outbox SET state=? WHERE event_key=? AND peer=?',
                             ('"unknown"', event_key, mac_id)).rowcount
        assert changed == 1
    log = open(Path(a.local_data)/'recovery-agent.log', 'ab', buffering=0)
    subprocess.Popen([a.local_bin, 'daemon'], env=env, stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
    restarted = True
    until = time.monotonic()+40
    found = None
    while time.monotonic()<until:
        with sqlite3.connect(Path(a.local_data)/'state.sqlite3') as db:
            found = db.execute('SELECT state FROM outbox WHERE event_key=? AND peer=?', (event_key, mac_id)).fetchone()
        if found and found[0]=='"applied"':
            break
        time.sleep(.5)
    assert found and found[0]=='"applied"', 'STATUS did not recover Applied'
    assert not api(a.local_bin, 'history')['entries'], 'session history must stay empty after restart'
    assert not list((Path(a.local_data)/'history-images').iterdir()), 'restart must remove abandoned image cache'
    inspect = subprocess.run(remote(fixture, 'inspect'), capture_output=True, timeout=25, check=True).stdout.decode()
    assert inspect.strip().split('digest=')[1] == expected, 'STATUS overwrote the newer Mac clipboard'
    report = dict(test='sender restart after injected missing ACK; receiver has newer local clipboard', status='PASS',
                  method='dedicated sender SQLite recovery ledger changed Applied to Unknown while stopped',
                  result='authenticated STATUS restored Applied; independent receiver OS digest stayed on the newer local copy; session history and image cache stayed empty after restart',
                  limitation='persisted-state injection; not actual wire ACK loss or full crash-window matrix')
    Path(a.report).write_text(json.dumps(report, indent=2)+'\n')
    print('PASS receipt recovery without clipboard reapplication')
finally:
    if stopped and not restarted:
        subprocess.Popen([a.local_bin, 'daemon'], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    if log:
        log.close()
    child.terminate()
    try:
        child.wait(timeout=3)
    except subprocess.TimeoutExpired:
        child.kill()
