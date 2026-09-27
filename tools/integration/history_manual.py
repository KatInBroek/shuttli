#!/usr/bin/env python3
"""Local-copy retention, privacy controls and manual delivery on dedicated Linux VMs."""
import argparse
import base64
import copy
import hashlib
import json
from pathlib import Path
import time
from linux_hosts import Node


def wait(fn):
    until = time.monotonic() + 10
    while time.monotonic() < until:
        result = fn()
        if result:
            return result
        time.sleep(.1)
    raise TimeoutError('local history did not converge')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True)
    parser.add_argument('--report', required=True)
    parser.add_argument('--allow-clipboard-overwrite', action='store_true', required=True)
    args = parser.parse_args()
    nodes = [Node(n) for n in json.loads(Path(args.config).read_text())['nodes']]
    a, b = nodes
    original = {n.label: n.api('settings')['settings'] for n in nodes}
    results = []
    def configure(n, settings):
        n.api('request', json.dumps({'version': 1, 'action': {'command': 'configure', 'settings': settings}}))
    try:
        for n in nodes:
            settings = copy.deepcopy(original[n.label])
            settings.update(automatic=False, notifications=False, history='content', history_limit=20)
            configure(n, settings)
        for n in nodes:
            for mode in ('manual', 'send_disabled', 'no_targets'):
                settings = copy.deepcopy(original[n.label])
                settings.update(automatic=mode != 'manual', send=mode != 'send_disabled', history='content', history_limit=20, notifications=False)
                if mode == 'no_targets':
                    settings['peers'] = {}
                configure(n, settings)
                before = n.latest()
                digest = n.fixture()
                rows = wait(lambda: [r for r in n.history() if r['id'] > before])
                assert len(rows) == 1 and rows[0]['direction'] == 'local'
                body = n.api('history', 'preview', str(rows[0]['id']))
                data = base64.b64decode(body['base64'], validate=True)
                canonical = data.decode('utf-8').replace('\r\n', '\n').encode('utf-8')
                assert hashlib.sha256(b'text.v1\0' + canonical).hexdigest() == digest
                time.sleep(.4)
                assert len([r for r in n.history() if r['id'] > before]) == 1
                results.append(n.label + ': local text preview in ' + mode + '; zero outgoing records')
            settings.update(send=False, automatic=False, history='content')
            configure(n, settings)
            before = n.latest()
            n.fixture('png')
            row = wait(lambda: next((r for r in n.history() if r['id'] > before and r['direction'] == 'local'), None))
            data = base64.b64decode(n.api('history', 'preview', str(row['id']))['base64'])
            assert data.startswith(bytes.fromhex('89504e470d0a1a0a'))
            assert n.cache()['files'] == 1 and not n.cache()['plaintext']
            settings['history'] = 'status'
            configure(n, settings)
            assert n.cache()['files'] == 0
            before = n.latest()
            n.fixture()
            row = wait(lambda: next((r for r in n.history() if r['id'] > before), None))
            assert row['direction'] == 'local' and not row['available']
            for mode, limit in [('off', 20), ('content', 0)]:
                settings.update(history=mode, history_limit=limit)
                configure(n, settings)
                n.fixture()
                time.sleep(.5)
                assert n.history() == [] and n.cache()['files'] == 0
            results.append(n.label + ': encrypted local PNG; status/off/zero-limit cleanup respected')
        for n in nodes:
            settings = copy.deepcopy(original[n.label])
            settings.update(automatic=False, send=True, receive=True, history='content', history_limit=20)
            configure(n, settings)
        before = a.latest()
        digest = a.fixture()
        local = wait(lambda: next((r for r in a.history() if r['id'] > before and r['direction'] == 'local'), None))
        previous = a.latest()
        a.api('send')
        rows = a.settled(previous)
        assert rows and all(r['available'] for r in rows)
        assert b.inspect() == digest
        assert any(r['id'] == local['id'] for r in a.history())
        results.append('manual send succeeds while preserving original local copy history')
    finally:
        for n in nodes:
            configure(n, original[n.label])
            for child in n.children:
                child.terminate()
                child.wait(timeout=5)
    Path(args.report).write_text(json.dumps(results, indent=2)+'\n')
    print('\n'.join('PASS '+r for r in results))


if __name__ == '__main__':
    main()
