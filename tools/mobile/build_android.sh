#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo_dir"
sdk_dir=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}
ndk_dir="$sdk_dir/ndk/27.0.12077973"
llvm_bin="$ndk_dir/toolchains/llvm/prebuilt/linux-x86_64/bin"
[[ -x "$llvm_bin/llvm-ar" ]] || { echo "Android NDK 27.0.12077973 is required" >&2; exit 1; }
export ANDROID_HOME="$sdk_dir"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$llvm_bin/aarch64-linux-android29-clang"
export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
export AR_aarch64_linux_android="$llvm_bin/llvm-ar"
export CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER="$llvm_bin/x86_64-linux-android29-clang"
export CC_x86_64_linux_android="$CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER"
export AR_x86_64_linux_android="$llvm_bin/llvm-ar"
rustup target add aarch64-linux-android x86_64-linux-android
for target in aarch64-linux-android x86_64-linux-android; do
    cargo build --release --locked -p shuttli-mobile-ffi --target "$target"
done
out="$repo_dir/apps/android/app/build/generated/uniffi"
mkdir -p "$out"
cargo run --locked -p shuttli-mobile-ffi --features bindgen --bin uniffi-bindgen -- \
    generate "target/aarch64-linux-android/release/libshuttli_mobile_ffi.so" \
    --language kotlin --out-dir "$out" --no-format
for mapping in aarch64-linux-android:arm64-v8a x86_64-linux-android:x86_64; do
    target=${mapping%%:*}
    abi=${mapping##*:}
    dest="$repo_dir/apps/android/app/src/main/jniLibs/$abi"
    mkdir -p "$dest"
    cp "target/$target/release/libshuttli_mobile_ffi.so" "$dest/"
done
(cd apps/android && ./gradlew --no-daemon --stacktrace testDebugUnitTest assembleDebug assembleRelease bundleRelease)
python3 - "$repo_dir" <<'PY'
import hashlib
import json
from pathlib import Path
import subprocess
import sys

repo = Path(sys.argv[1])
files = {
    "debug_apk": repo / "apps/android/app/build/outputs/apk/debug/app-debug.apk",
    "unsigned_release_apk": repo / "apps/android/app/build/outputs/apk/release/app-release-unsigned.apk",
    "unsigned_release_bundle": repo / "apps/android/app/build/outputs/bundle/release/app-release.aab",
    "arm64_library": repo / "apps/android/app/src/main/jniLibs/arm64-v8a/libshuttli_mobile_ffi.so",
    "x86_64_library": repo / "apps/android/app/src/main/jniLibs/x86_64/libshuttli_mobile_ffi.so",
    "cargo_lock": repo / "Cargo.lock",
}
out = repo / "target/mobile-android"
out.mkdir(parents=True, exist_ok=True)
manifest = {
    "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip(),
    "sdk_api": 6,
    "wire_version": 2,
    "min_sdk": 29,
    "target_sdk": 36,
    "files": {name: {"sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "bytes": path.stat().st_size}
              for name, path in files.items()},
}
(out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
PY
