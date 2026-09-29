#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
destination="$repo_dir/target/mobile-tools"
archive="$destination/xcodegen-2.46.0.zip"
mkdir -p "$destination"
curl -fsSL --retry 3 \
  https://github.com/yonaskolb/XcodeGen/releases/download/2.46.0/xcodegen.zip \
  -o "$archive"
expected=4d9e34b62172d645eed6457cac13fc222569974098ef4ee9c3368bedf0196806
actual=$(shasum -a 256 "$archive" | cut -d ' ' -f 1)
if [[ "$actual" != "$expected" ]]; then
  echo "XcodeGen archive checksum mismatch" >&2
  exit 1
fi
unzip -q -o "$archive" -d "$destination"
echo "XcodeGen installed at $destination/xcodegen/bin/xcodegen"
