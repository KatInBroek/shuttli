#!/usr/bin/env python3
"""Package a native Linux release using an explicit file allowlist."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def output(*args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs).strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/shuttli')
    parser.add_argument('--output', type=Path, default=ROOT / 'target/packages')
    parser.add_argument('--version', help='Debian package version; defaults to Cargo workspace version')
    args = parser.parse_args()
    if output('git', '-C', str(ROOT), 'status', '--porcelain', '--untracked-files=normal'):
        parser.error('commit or stash source changes before packaging; metadata must identify a clean checkout')
    binary = args.binary.resolve()
    header = binary.read_bytes()[:20]
    if header[:6] != b'\x7fELF\x02\x01':
        parser.error('expected a 64-bit little-endian Linux ELF binary')
    architecture = {62: 'amd64', 183: 'arm64'}.get(struct.unpack('<H', header[18:20])[0])
    if architecture != output('dpkg', '--print-architecture'):
        parser.error('package on the native architecture so dependency inspection is reliable')
    version = args.version or tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
    if not re.fullmatch(r'[0-9][0-9A-Za-z.+~\-]*', version):
        parser.error('invalid Debian version')
    subprocess.run(['dpkg', '--validate-version', version], check=True)
    name = output(str(binary), '--print-product-name')
    expected = (ROOT / 'branding/name.txt').read_text().strip()
    if name != expected or any(ord(c) < 32 for c in name):
        parser.error('binary product name differs from this source tree; rebuild first')
    commit = output('git', '-C', str(ROOT), 'rev-parse', 'HEAD')
    epoch = int(os.environ.get('SOURCE_DATE_EPOCH') or output('git', '-C', str(ROOT), 'show', '-s', '--format=%ct', 'HEAD'))
    args.output.mkdir(parents=True, exist_ok=True)
    package = args.output.resolve() / f'shuttli_{version}_{architecture}.deb'
    with tempfile.TemporaryDirectory(prefix='clipboard-package-') as temp:
        stage = Path(temp) / 'root'
        def write(path, data, mode=0o644):
            target = stage / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data if isinstance(data, bytes) else data.encode())
            target.chmod(mode)
        write('usr/bin/shuttli', binary.read_bytes(), 0o755)
        write('usr/share/icons/hicolor/scalable/apps/org.shuttli.Control.svg',
              (ROOT / 'crates/native-ui/assets/app-icon.svg').read_bytes())
        write('usr/share/applications/org.shuttli.Control.desktop',
              '[Desktop Entry]\nType=Application\nName=' + name.replace('\\', '\\\\') +
              '\nComment=Clipboard sync across your devices\nExec=/usr/bin/shuttli ui\n'
              'Icon=org.shuttli.Control\nTerminal=false\nCategories=Utility;\n')
        write('usr/share/doc/shuttli/copyright', (ROOT / 'LICENSE').read_bytes())
        write('usr/share/doc/shuttli/README.md', (ROOT / 'docs/install/ubuntu.md').read_bytes())
        write('usr/share/doc/shuttli/SECURITY.md', (ROOT / 'SECURITY.md').read_bytes())
        changelog = f'shuttli ({version}) unstable; urgency=medium\n\n  * Desktop preview built from {commit}.\n'
        write('usr/share/doc/shuttli/changelog.gz', gzip.compress(changelog.encode(), mtime=epoch))
        manifest = dict(product=name, package='shuttli', version=version, architecture=architecture,
                        source_commit=commit, source_repository='https://github.com/KatInBroek/shuttli',
                        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
                        rustc=output('rustc', '--version'), source_date_epoch=epoch)
        write('usr/share/doc/shuttli/build.json', json.dumps(manifest, indent=2) + '\n')
        # Use the distro's symbol database, rather than guessing the glibc baseline.
        debian = Path(temp) / 'debian'
        debian.mkdir()
        (debian / 'control').write_text('Source: shuttli\n\nPackage: shuttli\nArchitecture: any\n')
        deps = output('dpkg-shlibdeps', '-O', '-e' + str(stage / 'usr/bin/shuttli'), cwd=temp)
        deps = next(line.removeprefix('shlibs:Depends=') for line in deps.splitlines() if line.startswith('shlibs:Depends='))
        size = sum(p.stat().st_size for p in stage.rglob('*') if p.is_file())
        write('DEBIAN/control', f'Package: shuttli\nVersion: {version}\nArchitecture: {architecture}\n'
              f'Maintainer: {name} contributors <334561011+KatInBroek@users.noreply.github.com>\n'
              f'Installed-Size: {(size + 1023) // 1024}\nSection: utils\nPriority: optional\n'
              f'Depends: {deps}, python3 (>= 3.10), python3-gi, gir1.2-gtk-4.0 (>= 4.6), libnotify-bin\n'
              'Recommends: xwayland\nSuggests: tailscale, gnome-shell-extension-appindicator\n'
              'Homepage: https://github.com/KatInBroek/shuttli\n'
              f'Description: {name} peer-to-peer clipboard synchronization\n'
              ' Text and image clipboard sync over Tailscale with native desktop controls.\n'
              ' Explicit per-device sending permissions and encrypted transport.\n')
        for path in sorted(stage.rglob('*'), reverse=True):
            os.utime(path, (epoch, epoch))
        os.utime(stage, (epoch, epoch))
        subprocess.run(['dpkg-deb', '--root-owner-group', '-Zxz', '--build', str(stage), str(package)],
                       env={**os.environ, 'SOURCE_DATE_EPOCH': str(epoch)}, check=True)
    digest = hashlib.sha256(package.read_bytes()).hexdigest()
    package.with_suffix('.deb.sha256').write_text(f'{digest}  {package.name}\n')
    package.with_suffix('.build.json').write_text(json.dumps(manifest, indent=2) + '\n')
    print(package)


if __name__ == '__main__':
    main()
