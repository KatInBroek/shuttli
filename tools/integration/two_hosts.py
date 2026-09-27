#!/usr/bin/env python3
"""Real clipboard acceptance. Opt-in: temporarily overwrites BOTH clipboards.

Requires running agents, synthetic fixtures, and fingerprint permissions already
set on both endpoints. No private clipboard data or credentials are recorded.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import time

p = argparse.ArgumentParser()
p.add_argument('--ssh', required=True)
p.add_argument('--local-bin', required=True)
p.add_argument('--remote-bin', required=True)
p.add_argument('--local-data', required=True)
p.add_argument('--remote-data', required=True)
p.add_argument('--allow-clipboard-overwrite', action='store_true', required=True)
p.add_argument('--report', required=True)
p.add_argument('--local-png')
p.add_argument('--remote-png')
p.add_argument('--png-only', action='store_true')
p.add_argument('--history-cleanup', action='store_true', help='shrink/clear test history and verify encrypted image cache deletion')
a = p.parse_args()
if bool(a.local_png) != bool(a.remote_png) or (a.png_only and not a.local_png):
    p.error('PNG tests need both --local-png and --remote-png')
results, children = [], []
local_env = dict(os.environ, SHUTTLI_DATA_DIR=a.local_data)


def raw(side, binary, *args, **kwargs):
    if side == 'linux':
        return subprocess.run([binary, *args], env=local_env, capture_output=True, timeout=40, **kwargs)
    command = shlex.join(['env', 'SHUTTLI_DATA_DIR=' + a.remote_data, binary, *args])
    return subprocess.run(['ssh', '-oBatchMode=yes', a.ssh, command], capture_output=True, timeout=40, **kwargs)


def api(side, *args, allow_error=False):
    r = raw(side, a.local_bin if side == 'linux' else a.remote_bin, *args, '--json')
    answer = json.loads(r.stdout)
    if answer['type'] == 'error' and not allow_error:
        raise RuntimeError(answer['message'])
    return answer


def history(side):
    return api(side, 'history')['entries']


def latest(side):
    rows = history(side)
    return max([r['id'] for r in rows], default=0)


def fixture(side, text, kind="text"):
    binary = str(Path(a.local_bin if side == 'linux' else a.remote_bin).parent / 'examples/clipboard_fixture')
    command = [binary, kind, text]
    if side == 'mac':
        command = ['ssh', '-oBatchMode=yes', a.ssh, shlex.join(['env', 'SHUTTLI_DATA_DIR=' + a.remote_data, *command])]
    child = subprocess.Popen(command, env=local_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    children.append(child)
    line = child.stdout.readline().decode()
    if 'fixture set;' not in line:
        raise RuntimeError('fixture could not acquire real clipboard: ' + child.stderr.read().decode())
    return line.strip().split('digest=')[1]


def inspect(side):
    binary = str(Path(a.local_bin if side == 'linux' else a.remote_bin).parent / 'examples/clipboard_fixture')
    r = raw(side, binary, 'inspect')
    if r.returncode:
        raise RuntimeError(r.stderr.decode())
    return r.stdout.decode().strip().split('digest=')[1]


def settle(side, previous, expected='applied'):
    until = time.monotonic() + 35
    while time.monotonic() < until:
        rows = [r for r in history(side) if r['id'] > previous and r['direction'] == 'send']
        if rows and rows[0]['state'] not in ('sending', 'receiving'):
            assert rows[0]['state'] == expected, rows[0]['detail']
            return rows[0]
        time.sleep(.3)
    raise AssertionError('delivery did not reach a terminal state')


def record(name, fn):
    start = time.monotonic()
    try:
        detail = fn()
        results.append(dict(test=name, status='PASS', seconds=round(time.monotonic()-start, 3), detail=detail))
        print('PASS', name, flush=True)
    except Exception as e:
        results.append(dict(test=name, status='FAIL', detail=str(e)))
        print('FAIL', name, str(e), flush=True)
        raise
    finally:
        Path(a.report).write_text(json.dumps(results, indent=2) + '\n')


original = {side: api(side, 'settings')['settings'] for side in ('linux', 'mac')}
try:
    for side in original:
        api(side, 'set', 'automatic', 'off')
        api(side, 'set', 'notifications', 'off')
        api(side, 'refresh')
    time.sleep(2)

    if not a.png_only:
        def transfer(source, target):
            previous = latest(source)
            digest = fixture(source, f'Shuttli fixture {source} → {target} · {time.monotonic_ns()}\nUTF-8 \u4e2d\u6587 🌍')
            api(source, 'send')
            row = settle(source, previous)
            assert inspect(target) == digest, 'independent target OS readback differs'
            return dict(bytes=row['bytes'], os_readback='matching canonical digest')

        record('manual Linux → macOS text', lambda: transfer('linux', 'mac'))
        record('manual macOS → Linux text over reverse connection', lambda: transfer('mac', 'linux'))

        def automatic():
            for side in original:
                api(side, 'set', 'automatic', 'on')
            before = {side: latest(side) for side in original}
            digest = fixture('linux', 'Shuttli auto/no-relay fixture ' + str(time.monotonic_ns()))
            settle('linux', before['linux'])
            assert inspect('mac') == digest
            time.sleep(3)
            rows = {side: [r for r in history(side) if r['id'] > before[side]] for side in original}
            assert len(rows['linux']) == 2, rows
            assert sorted(r['direction'] for r in rows['linux']) == ['local', 'send'], rows
            assert rows['linux'][0]['event'] == rows['linux'][1]['event'], rows
            assert len(rows['mac']) == 1 and rows['mac'][0]['direction'] == 'receive', rows
            for side in original:
                api(side, 'set', 'automatic', 'off')
            return 'one source publication, one remote apply, no return publication'
        record('automatic sync with both directions allowed does not echo', automatic)

        def denied():
            api('mac', 'set', 'receive', 'off')
            before_clip = inspect('mac')
            before = latest('linux')
            fixture('linux', 'Shuttli rejected fixture ' + str(time.monotonic_ns()))
            api('linux', 'send')
            settle('linux', before, 'failed')
            assert inspect('mac') == before_clip
            api('mac', 'set', 'receive', 'on')
            return 'receiver unchanged; sender reports rejection'
        record('global receive off prevents overwrite', denied)

        def send_off():
            api('linux', 'set', 'send', 'off')
            before = latest('linux')
            response = api('linux', 'send', allow_error=True)
            assert response['type'] == 'error'
            assert latest('linux') == before
            api('linux', 'set', 'send', 'on')
            return 'explicit send denied without a new delivery'
        record('global send off applies to manual commands', send_off)

        def historical(side):
            rows = [r for r in history(side) if r['available']]
            assert rows, 'enable recent content history on both test profiles before running'
            row = rows[0]
            preview = api(side, 'history', 'preview', str(row['id']))
            data = base64.b64decode(preview['base64'], validate=True)
            before = latest(side)
            api(side, 'history', 'copy', str(row['id']), '--local-only')
            time.sleep(1)
            assert latest(side) == before
            api(side, 'refresh')
            api(side, 'history', 'resend', str(row['id']))
            resent = settle(side, before)
            assert resent['event'] != row['event']
            return dict(preview_bytes=len(data), preview_sha256=hashlib.sha256(data).hexdigest(), new_event=True)
        for side in original:
            record(f'{side} session history preview, local-only copy and fresh resend', lambda side=side: historical(side))

    if a.local_png:
        def png_transfer(source, target):
            previous = latest(source)
            digest = fixture(source, a.local_png if source == 'linux' else a.remote_png, 'png')
            api(source, 'send')
            row = settle(source, previous)
            assert row['format'] == 'png'
            assert inspect(target) == digest, 'independent target PNG readback differs'
            return dict(bytes=row['bytes'], canonical_digest=digest, os_readback='matching canonical PNG pixels')
        record('manual Linux → macOS PNG', lambda: png_transfer('linux', 'mac'))
        record('manual macOS → Linux PNG', lambda: png_transfer('mac', 'linux'))

    if a.history_cleanup:
        def cache_stats(side):
            source = "import os,json;from pathlib import Path;p=Path(os.environ['SHUTTLI_DATA_DIR'])/'history-images';f=list(p.iterdir());print(json.dumps({'files':len(f),'plaintext_png':any(x.read_bytes().startswith(bytes.fromhex('89504e470d0a1a0a')) for x in f)}))"
            r = raw(side, 'python3', '-c', source)
            r.check_returncode()
            return json.loads(r.stdout)
        def clean_images(side):
            rows = [r for r in history(side) if r['format']=='png' and r['available']]
            assert rows, 'PNG fixtures required before cache cleanup test'
            row = rows[0]
            preview = api(side, 'history', 'preview', str(row['id']))
            assert base64.b64decode(preview['base64'], validate=True).startswith(bytes.fromhex('89504e470d0a1a0a'))
            before = latest(side)
            api(side, 'history', 'copy', str(row['id']), '--local-only')
            assert latest(side)==before
            api(side, 'history', 'resend', str(row['id']))
            newest = settle(side, before)
            stats = cache_stats(side)
            assert stats['files'] > 0 and not stats['plaintext_png']
            api(side, 'set', 'history-limit', '1')
            assert len(history(side))==1 and history(side)[0]['id']==newest['id']
            assert cache_stats(side)['files']==1
            api(side, 'history', 'clear')
            assert not history(side) and cache_stats(side)['files']==0
            assert api(side, 'history', 'preview', str(newest['id']), allow_error=True)['type']=='error'
            api(side, 'set', 'history-limit', str(original[side]['history_limit']))
            return 'PNG preview/copy/resend works; ciphertext cache; shrink to one removes older files; clear removes all files and invalidates previews'
        for side in original:
            record(f'{side} encrypted PNG history and physical cache cleanup', lambda side=side: clean_images(side))

finally:
    # Restore the user's mode and permissions, not a hardcoded manual mode.
    for side in original:
        try:
            api(side, 'request', json.dumps({'version': 2, 'action': {
                'command': 'configure', 'expected': api(side, 'settings')['settings'], 'settings': original[side]}}))
        except Exception as exc:
            print('cleanup failed:', side, str(exc), flush=True)
    for child in children:
        if child.poll() is None:
            child.terminate()
        try:
            child.wait(timeout=3)
        except subprocess.TimeoutExpired:
            child.kill()
    Path(a.report).write_text(json.dumps(results, indent=2) + '\n')
    for side in original:
        assert api(side, 'settings')['settings'] == original[side], side + ': settings not restored'
