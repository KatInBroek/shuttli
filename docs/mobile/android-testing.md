# Android test workflow

Build with `tools/mobile/build_android.sh`: both native ABIs, a debug APK,
an unsigned optimized release APK/AAB, and
`target/mobile-android/manifest.json`. Distribution signing stays external.

## Isolated emulator checks

Run `tools/mobile/verify_android_emulator.sh` with one selected emulator. It
runs clipboard/lifecycle/settings checks and independently reads an exported
image from a separate paste-probe app. This suite needs no tailnet account.

## Real Tailscale checks

`tools/mobile/verify_android_tailnet.py` coordinates the opt-in
`TailnetInstrumentedTest` using the official connected Tailscale client and
a running upgraded Linux desktop. Disable the emulator's own clipboard
sharing first: otherwise its host integration can bypass the network test.

Start the desktop agent with a disposable `SHUTTLI_DATA_DIR` and its intended
graphical clipboard environment. Stop the normal agent while using the same
port. Preserve its process environment, clipboard and settings, and restore
them afterward. Use the real login D-Bus for an intentional desktop test; never
start a private bus with the login session's `XDG_RUNTIME_DIR`.

Disable outgoing and incoming permissions for other peers in the disposable
profile. Keep global send/receive/automatic enabled and history in Content
mode. Add `.tailnet-test-profile` containing `disposable` to that profile; the
driver requires this marker before clearing test history. Install the debug
app and its instrumentation APK first. Build the latter with
`cd apps/android && ./gradlew --no-daemon assembleDebugAndroidTest`.

Store a private JSON configuration outside the checkout:

| Field | Meaning |
| --- | --- |
| `binary` | Upgraded Linux CLI executable. |
| `profile` | Disposable profile directory with the marker. |
| `emulator_serial` | Explicit `emulator-NNNN` target; physical phones are rejected. |
| `desktop_id`, `phone_id` | Authenticated public-key fingerprints from CLI status/devices. |
| `phone_ip` | Emulator's actual Tailscale IPv4 address. |
| `output` | Private directory for synthetic test logs. |
| `environment` | Optional overrides such as DISPLAY/XAUTHORITY. |
| `adb`, `xclip` | Optional tool executable paths. |
| `second_peer` | Optional object with `id` and an external `prepare_command` argument list. |

Requires Python 3, Pillow, ADB and xclip:

```sh
python3 tools/mobile/verify_android_tailnet.py path/to/private-config.json
```

The optional second-source command receives `SHUTTLI_TEST_TOKEN`. It must
configure a separate disposable desktop profile to allow sending to the phone,
clear its test history and copy `second desktop text <token>` into its clipboard.
Credentials, device addresses and connection setup remain in that external
script. Its owner manages stopping/restoring the second desktop agent.

Checks cover explicit text/image import and send, Linux clipboard readback,
exact image digests, history preview and explicit phone copy, background
listener closure, catch-up, permission revocation/re-enablement at both ends,
and optional multi-source merge/source attribution. Negative permission checks
span the transport's periodic refresh interval. The coordinator preserves the
Linux clipboard in memory and restores it and the desktop's send permission on
exit. It clears disposable history and resets the phone's permission for this
test desktop: use a dedicated emulator profile. Keep logs and device settings
outside Git.

The separate paste-probe app can independently verify an image copied from
history: pass its expected SHA-256 using the `expected` intent extra and require
the visible result to be `MATCH`.

Physical Android acceptance is separate: vendor/OS differences, split
tunneling, rotation/process recreation, third-party provider failures, real
cross-app paste and distribution signing need device verification.
