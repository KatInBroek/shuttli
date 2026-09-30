#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo_dir"
if [[ $(uname -s) != Darwin ]]; then
  echo "iOS builds require macOS and Xcode" >&2
  exit 1
fi
export PATH="$repo_dir/target/mobile-tools/xcodegen/bin:$PATH"
command -v xcodegen >/dev/null || { echo "Install XcodeGen 2.46.0 first" >&2; exit 1; }
command -v xcodebuild >/dev/null || { echo "Xcode is required" >&2; exit 1; }

case $(uname -m) in
  arm64) simulator_target=aarch64-apple-ios-sim; simulator_arch=arm64 ;;
  x86_64) simulator_target=x86_64-apple-ios; simulator_arch=x86_64 ;;
  *) echo "Unsupported Mac architecture" >&2; exit 1 ;;
esac
device_target=aarch64-apple-ios
rustup target add "$device_target" "$simulator_target"

generated="$repo_dir/apps/ios/Generated"
mkdir -p "$generated/Headers"
python3 - "$repo_dir/branding/name.txt" "$generated/Brand.xcconfig" <<'PY'
from pathlib import Path
import sys
import json
import subprocess
name = Path(sys.argv[1]).read_text(encoding='utf-8').strip()
if not name or any(c in name for c in '\r\n$\\'):
    raise SystemExit('Invalid product display name for Xcode build setting')
metadata = json.loads(subprocess.check_output(
    ['cargo', 'metadata', '--locked', '--format-version', '1', '--no-deps'], text=True))
version = next(package['version'] for package in metadata['packages']
               if package['name'] == 'shuttli-mobile-ffi')
Path(sys.argv[2]).write_text(f'SHUTTLI_PRODUCT_NAME = {name}\nMARKETING_VERSION = {version}\nCURRENT_PROJECT_VERSION = 1\n', encoding='utf-8')
PY

for target in "$device_target" "$simulator_target"; do
  cargo build --locked --release -p shuttli-mobile-ffi --target "$target"
done
sim_library="target/$simulator_target/release/libshuttli_mobile_ffi.a"
device_library="target/$device_target/release/libshuttli_mobile_ffi.a"
cargo run --locked -p shuttli-mobile-ffi --features bindgen --bin uniffi-bindgen -- \
  generate "$sim_library" --language swift --out-dir "$generated"
cp "$generated/shuttli_mobile_ffiFFI.h" "$generated/Headers/"
cp "$generated/shuttli_mobile_ffiFFI.modulemap" "$generated/Headers/module.modulemap"
rm -rf "$generated/ShuttliMobile.xcframework"
xcodebuild -create-xcframework \
  -library "$sim_library" -headers "$generated/Headers" \
  -library "$device_library" -headers "$generated/Headers" \
  -output "$generated/ShuttliMobile.xcframework"

(cd apps/ios && xcodegen generate)
xcodebuild -project apps/ios/Shuttli.xcodeproj -scheme Shuttli \
  -destination 'generic/platform=iOS Simulator' -sdk iphonesimulator \
  -derivedDataPath target/ios-derived \
  CODE_SIGNING_ALLOWED=NO "ARCHS=$simulator_arch" ONLY_ACTIVE_ARCH=YES build
