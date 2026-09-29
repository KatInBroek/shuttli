#!/usr/bin/env bash
set -euo pipefail

repo_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$repo_dir"
if [[ -n ${ANDROID_SERIAL:-} ]]; then
  serial=$ANDROID_SERIAL
else
  mapfile -t online < <(adb devices | awk '$2 == "device" {print $1}')
  [[ ${#online[@]} -eq 1 ]] || { echo "Expected exactly one online emulator" >&2; exit 1; }
  serial=${online[0]}
fi
[[ $serial == emulator-* ]] || { echo "This script only runs on an Android emulator" >&2; exit 1; }
export ANDROID_SERIAL=$serial

(cd apps/android && ./gradlew --no-daemon -PpasteProbe=true :paste-probe:assembleDebug)
adb install -r apps/android/app/build/outputs/apk/debug/app-debug.apk
adb install -r apps/android/app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
adb install -r apps/android/paste-probe/build/outputs/apk/debug/paste-probe-debug.apk

result=$(adb shell am instrument -w org.katinbroek.shuttli.test/androidx.test.runner.AndroidJUnitRunner)
[[ $result == *'OK (4 tests)'* ]] || { echo "$result"; exit 1; }
printf '%s\n' "$result"

adb logcat -c
result=$(adb shell am instrument -w -e class \
  'org.katinbroek.shuttli.MobileInstrumentedTest#exportedImageStaysReadableAfterHistoryClear' \
  org.katinbroek.shuttli.test/androidx.test.runner.AndroidJUnitRunner)
[[ $result == *'OK (1 test)'* ]] || { echo "$result"; exit 1; }
expected=$(adb logcat -d -s ShuttliPasteTest:I | sed -n 's/.*PNG_SHA256=\([0-9a-f]\{64\}\).*/\1/p' | tail -1)
[[ ${#expected} -eq 64 ]] || { echo "Synthetic image digest missing" >&2; exit 1; }
adb shell am start -n org.katinbroek.shuttli.pasteprobe/.PasteProbeActivity --es expected "$expected"
for attempt in 1 2 3 4 5; do
  adb shell uiautomator dump /sdcard/shuttli-paste-probe.xml >/dev/null
  view=$(adb shell cat /sdcard/shuttli-paste-probe.xml)
  if [[ $view == *'text="MATCH '* ]]; then
    echo "Independent cross-app PNG clipboard readback passed"
    exit 0
  fi
  sleep 1
done
echo "Independent cross-app PNG clipboard readback failed" >&2
exit 1
