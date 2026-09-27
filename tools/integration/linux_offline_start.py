#!/usr/bin/env python3
"""Real offline startup/recovery on a dedicated VM; SSH must use its LAN IP.

The second node is restarted while its Tailscale service is stopped. It needs
tailscale_stop_command and tailscale_start_command argv arrays in local config.
"""
import argparse
import json
import shlex
import subprocess
import time
from pathlib import Path

from linux_hosts import Node


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', required=True)
    parser.add_argument('--report', required=True)
    parser.add_argument('--allow-network-interruption', action='store_true', required=True)
    parser.add_argument('--allow-clipboard-overwrite', action='store_true', required=True)
    args = parser.parse_args()
    nodes = [Node(s) for s in json.loads(Path(args.config).read_text())['nodes']]
    if len(nodes) != 2:
        parser.error('exactly two dedicated Linux test nodes required')
    target, source = nodes
    for key in ('tailscale_stop_command', 'tailscale_start_command'):
        if not source.spec.get(key):
            parser.error('missing explicit network control command: ' + key)
    results = []
    report = Path(args.report)
    report.parent.mkdir(parents=True, exist_ok=True)
    original = source.api('status')['status']
    target_original = target.api('settings')['settings']
    source.api('set', 'automatic', 'off')
    target.api('set', 'automatic', 'off')
    before_target = target.inspect()
    baseline = target.latest()
    try:
        source.run('pkill', '-x', 'shuttli')
        subprocess.run(source.spec['tailscale_stop_command'], check=True, timeout=20)
        command = 'nohup ' + shlex.join([*source.env, source.binary, 'daemon'])
        command += ' </dev/null >' + shlex.quote(source.home + '/.local/share/shuttli-test/agent.log') + ' 2>&1 &'
        subprocess.run([*source.spec['ssh'], command], check=True, timeout=15)
        deadline = time.monotonic() + 15
        while source.api('status', error=True)['type'] != 'status':
            if time.monotonic() > deadline:
                raise TimeoutError('local controls unavailable while Tailscale is stopped')
            time.sleep(.3)
        status = source.api('status')['status']
        assert status['device'] == original['device'] and status['clipboard_available']
        pid = source.run('pgrep', '-x', 'shuttli').strip()
        previous = source.latest()
        source.fixture()
        source.api('send')
        failed = source.settled(previous, 'failed')
        assert all('not ready' in row['detail'] for row in failed)
        assert target.inspect() == before_target and target.latest() == baseline
        results.append({'test': 'offline startup keeps local controls and rejects outbound content', 'status': 'PASS',
                        'detail': 'real tailscaled stop; fresh agent PID; OS clipboard readable; explicit failed send, no destination change'})
        subprocess.run(source.spec['tailscale_start_command'], check=True, timeout=20)
        deadline = time.monotonic() + 60
        target_id = target.api('status')['status']['device']
        while not any(p['id'] == target_id and p['online'] for p in source.api('devices')['devices']):
            if time.monotonic() > deadline:
                raise TimeoutError('discovery did not recover after tailscaled restart')
            source.api('refresh')
            target.api('refresh')
            time.sleep(2)
        time.sleep(3)
        assert source.run('pgrep', '-x', 'shuttli').strip() == pid
        assert target.inspect() == before_target and target.latest() == baseline
        previous = source.latest()
        digest = source.fixture()
        source.api('send')
        source.settled(previous)
        assert target.inspect() == digest
        results.append({'test': 'network recovery reuses agent without replaying failed offline copy', 'status': 'PASS',
                        'detail': 'same agent PID recovered discovery; failed copy never delivered later; fresh explicit send passed independent OS readback'})
    except Exception as exc:
        results.append({'test': 'offline startup/recovery', 'status': 'FAIL', 'detail': str(exc)})
        raise
    finally:
        subprocess.run(source.spec['tailscale_start_command'], check=True, timeout=20)
        for n, settings in ((source, original['settings']), (target, target_original)):
            n.api('set', 'automatic', 'on' if settings['automatic'] else 'off')
            for child in n.children:
                if child.poll() is None:
                    child.terminate()
                try:
                    child.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        report.write_text(json.dumps(results, indent=2) + '\n')
        for row in results:
            print(row['status'], row['test'], flush=True)


if __name__ == '__main__':
    main()
