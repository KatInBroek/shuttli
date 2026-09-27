#!/usr/bin/env python3
"""Two real Linux desktops over SSH; use dedicated accounts and synthetic data.

Config is local-only JSON: {"nodes": [{"label": "mint", "ssh": ["ssh",
"-F", "/path/to/config", "test-node"], "home": "/home/example-user", "uid": 1000,
"binary": "/home/example-user/bin/shuttli", "png": "/tmp/fixture.png"}, ...]}.
Each daemon must already be running, with mutual discovery working. No addresses,
clipboard bodies, or credentials are included in successful reports.
"""

import argparse
import base64
import copy
import hashlib
import json
import select
import shlex
import subprocess
import time
from pathlib import Path


class Node:
    def __init__(self, spec):
        self.spec = spec
        self.label = spec['label']
        self.home = spec['home']
        self.data = spec.get('data', self.home + '/.local/share/shuttli')
        self.binary = spec['binary']
        self.fixture_binary = str(Path(self.binary).parent / 'examples/clipboard_fixture')
        self.children = []
        self.env = ['env', 'DISPLAY=' + spec.get('display', ':0'),
                    'XAUTHORITY=' + spec.get('xauthority', self.home + '/.Xauthority'),
                    'XDG_RUNTIME_DIR=/run/user/' + str(spec['uid']),
                    'DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/' + str(spec['uid']) + '/bus',
                    'SHUTTLI_DATA_DIR=' + self.data]

    def command(self, *args):
        command = [*self.env, *args]
        return [*self.spec['ssh'], shlex.join(command)] if self.spec['ssh'] else command

    def run(self, *args):
        result = subprocess.run(self.command(*args), capture_output=True, timeout=45)
        if result.returncode:
            raise RuntimeError(f'{self.label}: command failed: {result.stderr.decode()[:300]}')
        return result.stdout

    def api(self, *args, error=False):
        result = subprocess.run(self.command(self.binary, *args, '--json'),
                                capture_output=True, timeout=45)
        answer = json.loads(result.stdout)
        if answer['type'] == 'error' and not error:
            raise RuntimeError(self.label + ': ' + answer['message'])
        return answer

    def history(self):
        return self.api('history')['entries']

    def configure(self, settings):
        answer = self.api('request', json.dumps({'version': 2, 'action': {
            'command': 'configure', 'expected': self.api('settings')['settings'], 'settings': settings}}))
        assert answer['settings'] == settings, self.label + ': settings were not applied'

    def latest(self):
        return max((row['id'] for row in self.history()), default=0)

    def inspect(self):
        return self.run(self.fixture_binary, 'inspect').decode().strip().split('digest=')[1]

    def fixture(self, kind='text'):
        value = self.spec['png'] if kind == 'png' else f'Shuttli Linux fixture {time.monotonic_ns()}\n\u4e2d\u6587 🌍'
        child = subprocess.Popen(self.command(self.fixture_binary, kind, value),
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.children.append(child)
        if not select.select([child.stdout], [], [], 20)[0]:
            raise TimeoutError(self.label + ': clipboard fixture startup')
        line = child.stdout.readline().decode()
        assert 'fixture set;' in line, self.label + ': fixture did not acquire clipboard'
        return line.strip().split('digest=')[1]

    def settled(self, previous, state='applied'):
        until = time.monotonic() + 40
        while time.monotonic() < until:
            rows = [r for r in self.history() if r['id'] > previous and r['direction'] == 'send']
            if rows and all(r['state'] not in ('sending', 'receiving') for r in rows):
                assert all(r['state'] == state for r in rows), [(r['state'], r['detail']) for r in rows]
                return rows
            time.sleep(.3)
        raise TimeoutError(self.label + ': no terminal delivery state')

    def cache(self):
        code = ("import os,json;from pathlib import Path;"
                "p=Path(os.environ['SHUTTLI_DATA_DIR'])/'history-images';"
                "f=list(p.iterdir());print(json.dumps({'files':len(f),"
                "'plaintext':any(x.read_bytes().startswith(bytes.fromhex('89504e470d0a1a0a')) for x in f)}))")
        return json.loads(self.run('python3', '-c', code))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True)
    parser.add_argument('--report', required=True)
    parser.add_argument('--allow-clipboard-overwrite', action='store_true', required=True)
    args = parser.parse_args()
    nodes = [Node(n) for n in json.loads(Path(args.config).read_text())['nodes']]
    if len(nodes) != 2:
        parser.error('this suite requires exactly two dedicated desktops')
    a, b = nodes
    results = []
    original = {n.label: n.api('settings')['settings'] for n in nodes}
    report = Path(args.report)
    report.parent.mkdir(parents=True, exist_ok=True)

    def record(name, fn):
        start = time.monotonic()
        try:
            detail = fn()
            results.append(dict(test=name, status='PASS', seconds=round(time.monotonic()-start, 3), detail=detail))
            print('PASS', name, flush=True)
        except Exception as exc:
            results.append(dict(test=name, status='FAIL', detail=str(exc)))
            print('FAIL', name, str(exc), flush=True)
            raise
        finally:
            report.write_text(json.dumps(results, indent=2) + '\n')

    def transfer(source, target, kind='text'):
        previous = source.latest()
        digest = source.fixture(kind)
        source.api('send')
        rows = source.settled(previous)
        assert len(rows) == 1, 'unexpected additional permitted recipient'
        assert target.inspect() == digest, 'independent OS readback differs'
        return dict(format=kind, bytes=rows[0]['bytes'], os_readback='matching canonical digest')

    def automatic():
        for source, target in ((a, b), (b, a)):
            for n in nodes:
                n.api('set', 'automatic', 'on')
            before = {n.label: n.latest() for n in nodes}
            digest = source.fixture()
            source.settled(before[source.label])
            assert target.inspect() == digest
            time.sleep(3)
            sent = [r for r in source.history() if r['id'] > before[source.label]]
            received = [r for r in target.history() if r['id'] > before[target.label]]
            assert len(sent) == 2
            assert sorted(r['direction'] for r in sent) == ['local', 'send']
            assert sent[0]['event'] == sent[1]['event']
            assert len(received) == 1 and received[0]['direction'] == 'receive'
            for n in nodes:
                n.api('set', 'automatic', 'off')
        return 'both source directions: one local event with one send, one receive, zero echo'

    def denied(setting, peer=False, kind='text'):
        command = ['peer', a.identity, setting] if peer else ['set', setting]
        disabled, enabled = 'off', 'on'
        b.api(*command, disabled)
        try:
            baseline = b.inspect()
            previous = a.latest()
            a.fixture(kind)
            a.api('send')
            a.settled(previous, 'failed')
            assert b.inspect() == baseline
        finally:
            b.api(*command, enabled)
        return 'explicit rejection; destination OS clipboard unchanged'

    def send_off(peer=False):
        command = ['peer', b.identity, 'send'] if peer else ['set', 'send']
        a.api(*command, 'off')
        try:
            previous = a.latest()
            baseline = b.inspect()
            response = a.api('send', error=True)
            if peer:
                assert response == {'type': 'done', 'message': 'Queued for 0 allowed device(s)'}
            else:
                assert response['type'] == 'error'
            assert a.latest() == previous and b.inspect() == baseline
        finally:
            a.api(*command, 'on')
        return 'no permitted delivery, no history append, destination clipboard unchanged'

    def historical(n, kind):
        row = next(r for r in n.history() if r['available'] and r['format'] == kind)
        preview = n.api('history', 'preview', str(row['id']))
        data = base64.b64decode(preview['base64'], validate=True)
        before = n.latest()
        n.api('set', 'automatic', 'on')
        try:
            n.api('history', 'copy', str(row['id']), '--local-only')
            time.sleep(2)
            assert n.latest() == before, 'local-only copy published under automatic mode'
        finally:
            n.api('set', 'automatic', 'off')
        n.api('history', 'resend', str(row['id']))
        resent = n.settled(before)[0]
        assert resent['event'] != row['event']
        if kind == 'png':
            assert data.startswith(bytes.fromhex('89504e470d0a1a0a'))
            assert n.cache()['files'] > 0 and not n.cache()['plaintext']
            n.api('set', 'history-limit', '1')
            assert len(n.history()) == 1 and n.cache()['files'] == 1
            n.api('history', 'clear')
            assert not n.history() and n.cache()['files'] == 0
            assert n.api('history', 'preview', str(resent['id']), error=True)['type'] == 'error'
            n.api('set', 'history-limit', '20')
        return dict(preview_bytes=len(data), preview_sha256=hashlib.sha256(data).hexdigest(), new_event=True)

    try:
        for n in nodes:
            status = n.api('status')['status']
            assert status['clipboard_available'], n.label + ': clipboard unavailable'
            n.identity = status['device']
            settings = copy.deepcopy(original[n.label])
            settings.update(automatic=False, notifications=False, send=True, receive=True,
                            text=True, png=True, history='content', history_limit=20)
            # Only test endpoints may receive synthetic fixtures. Restore every
            # original destination and preference, including automatic, below.
            settings['peers'] = {}
            n.configure(settings)
            n.api('refresh')
        until = time.monotonic() + 45
        for source, target in ((a, b), (b, a)):
            while not any(p['id'] == target.identity and p['online'] for p in source.api('devices')['devices']):
                if time.monotonic() > until:
                    raise TimeoutError('mutual discovery did not complete')
                time.sleep(1)
            source.api('peer', target.identity, 'send', 'on')
        record('manual text A to B', lambda: transfer(a, b))
        record('manual text B to A', lambda: transfer(b, a))
        record('automatic bidirectional no echo', automatic)
        record('global receive disabled', lambda: denied('receive'))
        record('global send disabled', send_off)
        record('peer receive disabled', lambda: denied('receive', peer=True))
        record('peer send disabled', lambda: send_off(peer=True))
        record('text reception disabled', lambda: denied('text'))
        for n in nodes:
            record(n.label + ' text history copy/resend', lambda n=n: historical(n, 'text'))
        record('manual PNG A to B', lambda: transfer(a, b, 'png'))
        record('manual PNG B to A', lambda: transfer(b, a, 'png'))
        record('PNG reception disabled', lambda: denied('png', kind='png'))
        for n in nodes:
            record(n.label + ' encrypted PNG history/cleanup', lambda n=n: historical(n, 'png'))
    finally:
        for n in nodes:
            try:
                n.configure(original[n.label])
            except Exception as exc:
                print('cleanup failed:', n.label, str(exc), flush=True)
            for child in n.children:
                if child.poll() is None:
                    child.terminate()
                try:
                    child.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        for n in nodes:
            assert n.api('settings')['settings'] == original[n.label], n.label + ': settings not restored'


if __name__ == '__main__':
    main()
