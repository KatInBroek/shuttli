#!/usr/bin/env python3
"""Install a locally built release without enabling synchronization/autostart."""
import argparse
from pathlib import Path
import os
import plistlib
import shutil
import stat
import subprocess
import sys

p = argparse.ArgumentParser()
p.add_argument('--prefix', type=Path, default=Path.home()/'.local')
p.add_argument('--app-dir', type=Path, default=Path.home()/'Applications')
a = p.parse_args()
root = Path(__file__).resolve().parents[1]
binary = root/'target/release/shuttli'
if not binary.is_file():
    raise SystemExit('Build first: cargo build --release -p shuttli-host --locked')
if sys.platform not in ('linux', 'darwin'):
    raise SystemExit('Only Linux and macOS hosts are implemented in this release')
product_name = subprocess.check_output([str(binary), '--print-product-name'], encoding='utf-8').rstrip('\r\n')
if not product_name or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in product_name):
    raise SystemExit('Invalid product name from build; rebuild the host')
bindir = a.prefix/'bin'
bindir.mkdir(parents=True, exist_ok=True)
target = bindir/'shuttli'
staged = bindir/('.shuttli-install-' + str(os.getpid()))
shutil.copy2(binary, staged)
staged.chmod(0o755)
staged.replace(target)
if sys.platform == 'darwin':
    bundle = a.app_dir/'Shuttli.app'
    macos = bundle/'Contents/MacOS'
    macos.mkdir(parents=True, exist_ok=True)
    executable = macos/'shuttli'
    shutil.copy2(target, executable)
    executable.chmod(0o755)
    version = subprocess.check_output([str(binary), '--version'], encoding='utf-8').strip().rsplit(' ', 1)[-1]
    info = dict(CFBundleName=product_name, CFBundleDisplayName=product_name, CFBundleIdentifier='org.shuttli.app',
                CFBundleExecutable='shuttli', CFBundlePackageType='APPL', CFBundleVersion=version,
                CFBundleShortVersionString=version, LSUIElement=True,
                NSHighResolutionCapable=True, NSLocalNetworkUsageDescription='Discover and synchronize clipboards with devices you allow.')
    (bundle/'Contents/Info.plist').write_bytes(plistlib.dumps(info))
    # Local development signature; this is not Developer ID signing/notarization.
    subprocess.run(['codesign','--force','--deep','--sign','-',str(bundle)], check=True)
    print('Application:', bundle)
else:
    apps=a.prefix/'share/applications'
    apps.mkdir(parents=True, exist_ok=True)
    icons=a.prefix/'share/icons/hicolor/scalable/apps'
    icons.mkdir(parents=True, exist_ok=True)
    shutil.copy2(root/'crates/native-ui/assets/app-icon.svg', icons/'org.shuttli.Control.svg')
    escaped=str(target).replace('\\','\\\\').replace('"','\\"').replace('`','\\`').replace('$','\\$').replace('%','%%')
    (apps/'org.shuttli.Control.desktop').write_text('[Desktop Entry]\nType=Application\nName='+product_name.replace('\\','\\\\')+'\nComment=Clipboard sync across your devices\nExec="'+escaped+'" ui\nIcon=org.shuttli.Control\nTerminal=false\nCategories=Utility;\n', encoding='utf-8')
print('Executable:', target)
print('Run `shuttli ui`, verify device fingerprints, then enable outgoing sync for selected devices.')
print('Start-at-login settings and existing device permissions were preserved.')
