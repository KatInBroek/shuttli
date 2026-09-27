#!/usr/bin/env python3
"""Repeatable local unit/architecture/coverage gate, with opt-in real Linux E2E.

Requires cargo-llvm-cov 0.9.1 and rustup component llvm-tools-preview.
E2E config files contain local machine paths and must not be committed.
"""
import argparse
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--linux-config')
    p.add_argument('--tray-config')
    p.add_argument('--three-config')
    p.add_argument('--lifecycle-config')
    p.add_argument('--offline-config')
    p.add_argument('--allow-clipboard-overwrite', action='store_true')
    p.add_argument('--allow-reboot', action='store_true')
    p.add_argument('--allow-network-interruption', action='store_true')
    args = p.parse_args()
    if (args.linux_config or args.three_config or args.offline_config) and not args.allow_clipboard_overwrite:
        p.error('real E2E needs --allow-clipboard-overwrite and dedicated test desktops')
    if args.lifecycle_config and not args.allow_reboot:
        p.error('lifecycle E2E needs --allow-reboot and dedicated test VMs')
    if args.offline_config and not args.allow_network_interruption:
        p.error('offline startup E2E needs --allow-network-interruption')
    commands = [
        ['cargo', 'fmt', '--all', '--check'],
        ['cargo', 'clippy', '--workspace', '--all-targets', '--locked', '--', '-D', 'warnings'],
        ['cargo', 'test', '--workspace', '--locked'],
        [sys.executable, 'tools/check_architecture.py'],
        [sys.executable, '-m', 'unittest', 'discover', '-s', 'tools/tests', '-v'],
        [sys.executable, '-m', 'compileall', '-q', 'tools', 'crates/native-ui/linux'],
        ['cargo', 'llvm-cov', '-p', 'shuttli-core', '--locked', '--json', '--output-path', 'target/core-coverage.json'],
        [sys.executable, 'tools/check_core_coverage.py', 'target/core-coverage.json', '--summary-output', 'target/core-coverage-summary.json'],
    ]
    for config, script, consent in [(args.linux_config, 'linux_hosts', '--allow-clipboard-overwrite'),
                                    (args.three_config, 'three_hosts', '--allow-clipboard-overwrite'),
                                    (args.lifecycle_config, 'linux_lifecycle', '--allow-reboot')]:
        if config:
            commands.append([sys.executable, f'tools/integration/{script}.py', '--config', str(Path(config).resolve()),
                             '--report', f'target/e2e/{script}.json', consent])
    if args.tray_config:
        commands.append([sys.executable, 'tools/integration/linux_tray.py', '--config', str(Path(args.tray_config).resolve()),
                         '--report', 'target/e2e/linux-tray.json'])
    if args.offline_config:
        commands.append([sys.executable, 'tools/integration/linux_offline_start.py', '--config', str(Path(args.offline_config).resolve()),
                         '--report', 'target/e2e/linux_offline_start.json', '--allow-network-interruption', '--allow-clipboard-overwrite'])
    for command in commands:
        print('RUN', ' '.join(command), flush=True)
        subprocess.run(command, cwd=ROOT, check=True)
    if not args.linux_config:
        print('Unit/coverage gates passed. Real Linux E2E NOT RUN (no --linux-config).')


if __name__ == '__main__':
    main()
