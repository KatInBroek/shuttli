#!/usr/bin/env python3
"""Reboot dedicated Linux VMs and verify actual graphical-login autostart.

Node config additionally needs reboot_command: an argv array, e.g. virsh reboot.
Leaves agents running manually and login autostart disabled after the checks.
"""
import argparse
import json
import shlex
import subprocess
import time
from pathlib import Path

from linux_hosts import Node


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--config', required=True)
    p.add_argument('--report', required=True)
    p.add_argument('--allow-reboot', action='store_true', required=True)
    args = p.parse_args()
    nodes = [Node(n) for n in json.loads(Path(args.config).read_text())['nodes']]
    if not all(n.spec.get('reboot_command') and n.spec['ssh'] for n in nodes):
        p.error('only explicitly configured remote test VMs can be rebooted')
    evidence = []
    report = Path(args.report)
    report.parent.mkdir(parents=True, exist_ok=True)
    original = {n.label: n.api('settings')['settings'] for n in nodes}

    def start(n):
        command = 'nohup ' + shlex.join([*n.env, n.binary, 'daemon'])
        command += ' </dev/null >' + shlex.quote(n.home + '/.local/share/shuttli-test/agent.log') + ' 2>&1 &'
        subprocess.run([*n.spec['ssh'], command], check=True, timeout=15)

    try:
        for enabled in (True, False):
            baseline = {}
            for n in nodes:
                n.api('set', 'automatic', 'off')
                n.api('autostart', 'on' if enabled else 'off')
                baseline[n.label] = (n.run('cat', '/proc/sys/kernel/random/boot_id'),
                                     n.api('status')['status']['device'], n.api('settings')['settings'])
                subprocess.run(n.spec['reboot_command'], check=True, timeout=15)
            for n in nodes:
                deadline = time.monotonic() + 120
                while True:
                    try:
                        old_boot, identity, settings = baseline[n.label]
                        assert n.run('cat', '/proc/sys/kernel/random/boot_id') != old_boot
                        n.run('pgrep', '-x', 'xfce4-session')
                        status = n.api('status', error=True)
                        if enabled:
                            assert status['type'] == 'status'
                            assert status['status']['device'] == identity
                            assert status['status']['clipboard_available']
                            assert status['status']['settings'] == settings
                            assert not n.history() and n.cache()['files'] == 0
                            assert len(n.run('pgrep', '-x', 'shuttli').splitlines()) == 1
                        else:
                            assert status['type'] == 'error'
                            # A graphical session existing briefly is insufficient;
                            # allow its complete autostart phase to finish.
                            uptime = float(n.run('cat', '/proc/uptime').split()[0])
                            assert uptime > 25
                            processes = n.run('sh', '-c', 'pgrep -x shuttli || true')
                            assert not processes.strip()
                        break
                    except (AssertionError, RuntimeError, subprocess.SubprocessError, ValueError):
                        if time.monotonic() > deadline:
                            raise TimeoutError(n.label + ': graphical-login autostart check failed')
                        time.sleep(2)
                evidence.append({'test': n.label + (' enabled login autostart' if enabled else ' disabled login autostart'),
                                 'status': 'PASS', 'detail': 'real guest reboot and Xfce login; one agent, identity/settings preserved, history/cache empty' if enabled else 'real guest reboot and Xfce login; no Shuttli process after startup'})
                print('PASS', evidence[-1]['test'], flush=True)
                report.write_text(json.dumps(evidence, indent=2) + '\n')
                if not enabled:
                    start(n)
    except Exception as exc:
        evidence.append({'test': 'lifecycle', 'status': 'FAIL', 'detail': str(exc)})
        report.write_text(json.dumps(evidence, indent=2) + '\n')
        raise
    finally:
        for n in nodes:
            try:
                n.configure(original[n.label])
            except Exception as exc:
                print('settings restoration failed:', n.label, str(exc), flush=True)
        for n in nodes:
            assert n.api('settings')['settings'] == original[n.label], n.label + ': settings not restored'


if __name__ == '__main__':
    main()
