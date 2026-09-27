#!/usr/bin/env python3
"""Inspect a built Debian package without installing or opening a real clipboard."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile

FILES = {
    'usr/bin/shuttli',
    'usr/share/applications/org.shuttli.Control.desktop',
    'usr/share/icons/hicolor/scalable/apps/org.shuttli.Control.svg',
    'usr/share/doc/shuttli/copyright',
    'usr/share/doc/shuttli/README.md',
    'usr/share/doc/shuttli/SECURITY.md',
    'usr/share/doc/shuttli/changelog.gz',
    'usr/share/doc/shuttli/build.json',
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('package', type=Path)
    args = parser.parse_args()
    package = args.package.resolve()
    checksum = package.with_suffix('.deb.sha256').read_text().split()[0]
    assert hashlib.sha256(package.read_bytes()).hexdigest() == checksum
    with tempfile.TemporaryDirectory() as temp:
        root = Path(temp)
        subprocess.run(['dpkg-deb', '-R', str(package), str(root)], check=True)
        files = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file()}
        assert files == FILES | {'DEBIAN/control'}, files.symmetric_difference(FILES | {'DEBIAN/control'})
        assert not any(p.is_symlink() for p in root.rglob('*'))
        binary = root / 'usr/bin/shuttli'
        manifest = json.loads((root / 'usr/share/doc/shuttli/build.json').read_text())
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == manifest['binary_sha256']
        assert binary.stat().st_mode & 0o777 == 0o755
        env = {'PATH': '/usr/bin:/bin', 'HOME': str(root / 'isolated-home'),
               'SHUTTLI_DATA_DIR': str(root / 'isolated-profile')}
        name = subprocess.check_output([str(binary), '--print-product-name'], env=env, text=True).strip()
        assert name == manifest['product']
        assert not (root / 'isolated-profile').exists(), 'metadata must not create a profile'
        subprocess.run([str(binary), '--help'], env=env, check=True, stdout=subprocess.DEVNULL)
        # The CLI resolves its profile directory, but help must not create keys,
        # settings, clipboard history or a running agent.
        assert not list((root / 'isolated-profile').rglob('*'))
        desktop = (root / 'usr/share/applications/org.shuttli.Control.desktop').read_text()
        assert 'Exec=/usr/bin/shuttli ui\n' in desktop
        control = (root / 'DEBIAN/control').read_text()
        for dep in ['libc6 (>= ', 'python3-gi', 'gir1.2-gtk-4.0', 'libnotify-bin']:
            assert dep in control, dep
    print('Package checksum, payload allowlist, metadata, dependencies and CLI smoke checks passed.')


if __name__ == '__main__':
    main()
