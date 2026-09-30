#!/usr/bin/env bash
set -euo pipefail
repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo_dir"
# Select an installed iPhone runtime rather than pinning a runner-specific name.
simulator_id=$(xcrun simctl list devices available -j | python3 -c '
import json,sys
for devices in json.load(sys.stdin)["devices"].values():
    for device in devices:
        if "iPhone" in device["name"] and device["isAvailable"]:
            print(device["udid"]);sys.exit(0)
raise SystemExit("No available iPhone simulator")
')
xcodebuild -project apps/ios/Shuttli.xcodeproj -scheme Shuttli \
    -destination "platform=iOS Simulator,id=$simulator_id" \
    -derivedDataPath target/ios-derived -resultBundlePath target/ios-tests.xcresult \
    CODE_SIGNING_ALLOWED=NO -enableCodeCoverage YES test
xcrun xccov view --report --json target/ios-tests.xcresult > target/ios-coverage.json
