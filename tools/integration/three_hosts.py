#!/usr/bin/env python3
"""Three real Linux clipboard endpoints: broadcast, no relay, explicit resend.

Uses linux_hosts.py endpoint configuration. Restores all settings, including peer
permissions. Synthetic clipboard contents/history intentionally remain visible.
"""

import argparse
import copy
import json
import time
from pathlib import Path

from linux_hosts import Node


def configure(node, settings):
    request = json.dumps({'version': 2, 'action': {'command': 'configure', 'expected': node.api('settings')['settings'], 'settings': settings}})
    code = """import json,os,socket,sys
s=socket.socket(socket.AF_UNIX);s.settimeout(15)
s.connect(os.environ['SHUTTLI_DATA_DIR']+'/control.sock')
s.sendall(sys.argv[1].encode());s.shutdown(socket.SHUT_WR)
with s.makefile('rb') as f: data=f.read(65536)
print(data.decode())
"""
    answer = json.loads(node.run('python3', '-c', code, request))
    assert answer['type'] == 'settings' and answer['settings'] == settings, answer


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True)
    parser.add_argument('--report', required=True)
    parser.add_argument('--allow-clipboard-overwrite', action='store_true', required=True)
    args = parser.parse_args()
    nodes = [Node(n) for n in json.loads(Path(args.config).read_text())['nodes']]
    if len(nodes) != 3:
        parser.error('exactly three endpoints required')
    a, b, c = nodes
    original = {}
    results = []
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
            raise
        finally:
            report.write_text(json.dumps(results, indent=2) + '\n')

    def rows_since(before):
        return {n.label: [r for r in n.history() if r['id'] > before[n.label]] for n in nodes}

    def broadcast():
        before = {n.label: n.latest() for n in nodes}
        digest = a.fixture()
        sends = a.settled(before[a.label])
        assert len(sends) == 2 and sends[0]['event'] == sends[1]['event']
        assert b.inspect() == c.inspect() == digest
        time.sleep(4)
        rows = rows_since(before)
        assert len(rows[a.label]) == 3
        assert sorted(r['direction'] for r in rows[a.label]) == ['local', 'send', 'send']
        assert all(r['event'] == sends[0]['event'] for r in rows[a.label])
        for n in (b, c):
            assert len(rows[n.label]) == 1 and rows[n.label][0]['direction'] == 'receive'
        return 'A sent one event to B/C; both OS readbacks match; B/C published nothing despite mutual send permissions'

    def no_relay():
        a.api('peer', c.identity, 'send', 'off')
        before = {n.label: n.latest() for n in nodes}
        baseline = c.inspect()
        digest = a.fixture()
        assert len(a.settled(before[a.label])) == 1
        assert b.inspect() == digest
        time.sleep(4)
        assert c.inspect() == baseline
        rows = rows_since(before)
        assert len(rows[b.label]) == 1 and rows[b.label][0]['direction'] == 'receive'
        assert not rows[c.label]
        return 'A allowed B only; B did not relay to C even though B-to-C sending was enabled'

    def explicit_resend():
        row = next(r for r in b.history() if r['direction'] == 'receive' and r['available'])
        digest = b.inspect()
        before = {n.label: n.latest() for n in nodes}
        b.api('history', 'resend', str(row['id']))
        sends = b.settled(before[b.label])
        assert len(sends) == 2 and all(r['event'] != row['event'] for r in sends)
        assert a.inspect() == c.inspect() == digest
        time.sleep(4)
        rows = rows_since(before)
        for n in (a, c):
            assert len(rows[n.label]) == 1 and rows[n.label][0]['direction'] == 'receive'
        assert len(rows[b.label]) == 2
        return 'B explicit history resend created a new event, reached A/C, and caused no further automatic publication'

    try:
        for n in nodes:
            status = n.api('status')['status']
            assert status['clipboard_available']
            n.identity = status['device']
            original[n.label] = status['settings']
            settings = copy.deepcopy(status['settings'])
            settings.update(automatic=False, send=True, receive=True, notifications=False,
                            history='content', history_limit=100, text=True, png=True)
            for policy in settings['peers'].values():
                policy['send'] = False
            configure(n, settings)
            n.api('refresh')
        until = time.monotonic() + 45
        for n in nodes:
            for target in nodes:
                if n is target:
                    continue
                while not any(p['id'] == target.identity and p['online'] for p in n.api('devices')['devices']):
                    if time.monotonic() > until:
                        raise TimeoutError('three-node mutual discovery incomplete')
                    time.sleep(.5)
                n.api('peer', target.identity, 'send', 'on')
                n.api('peer', target.identity, 'receive', 'on')
            n.api('set', 'automatic', 'on')
        record('three-node broadcast without echo', broadcast)
        record('unauthorized recipient is not reached through an allowed peer', no_relay)
        record('explicit history resend to two peers is a fresh event', explicit_resend)
    finally:
        failures = []
        for n in nodes:
            if n.label in original:
                try:
                    configure(n, original[n.label])
                except Exception as exc:
                    failures.append(n.label + ': ' + str(exc))
            for child in n.children:
                if child.poll() is None:
                    child.terminate()
                try:
                    child.wait(timeout=3)
                except Exception:
                    child.kill()
                    child.wait()
        if failures:
            raise RuntimeError('settings restoration failed: ' + '; '.join(failures))


if __name__ == '__main__':
    main()
