# <!-- product-name:start -->Shuttli<!-- product-name:end -->

Peer-to-peer clipboard sync with per-device permissions and verified delivery.

**Preview:** Tailscale discovery, macOS ↔ Linux text and image synchronization, and foreground iOS/Android apps are implemented. LAN discovery, Windows, and file transfer remain later work. Machine-specific configuration, screenshots and run reports are kept outside the repository.

## Download

**[Download packages and release notes](https://github.com/KatInBroek/shuttli/releases)**

| Platform | Preview package |
| --- | --- |
| Ubuntu 24.04+ · Intel/AMD 64-bit | `.deb` installer |
| Android 10+ · ARM64 / x86_64 | Debug-signed test APK; unsigned release APK/AAB for external signing |
| iOS 16+ · iPhone / iPad | Unsigned device IPA; sign for your device before installation |

[Ubuntu installation](docs/install/ubuntu.md) · [Android](docs/mobile/android.md) · [iOS signing and installation](docs/mobile/ios.md)

This is a **preview release**. macOS packages, Windows packages and Linux ARM64
packages are not yet published. Tailscale must be installed separately.

## Features

- Direct TLS 1.3 connections; no central clipboard server.
- New devices have outgoing sync disabled. Allow individual device fingerprints; receiving is enabled by default.
- Independent global/per-device send, receive and content controls; automatic or manual sync.
- Core-owned publication/write permissions and conservative clipboard echo suppression.
- Live transfer succeeds only after the receiver verifies and commits the content through its reception adapter and acknowledges the result. Desktop reception applies the OS clipboard; mobile reception retains content in foreground history until you choose Copy.
- Ordinary local copies enter history even when sending is paused; received items retain their source and outgoing items retain each target's result.
- Configurable recent history (20 events by default): RAM-only text/list, encrypted temporary image cache, preview/copy/resend, and automatic/manual cache cleanup.
- macOS menu bar/native window; Linux system tray with global send/receive controls and an on-demand GTK4 window; the same local API powers CLI and UI.
- Linux UI languages: English, Nederlands, Deutsch, Français, or follow the system. Switch in **Settings → Language**; menus, windows and common notifications update without restarting.
- Optional notifications and start at graphical login. Installation never enables autostart.
- Native iOS and Android foreground apps: merged recent history, explicit clipboard import/send/copy, per-device permissions and four languages. Incoming content stays in app history until you choose Copy; synchronization stops in the background.

## Ubuntu package

Download the tested preview from [GitHub Releases](https://github.com/KatInBroek/shuttli/releases).
The [Ubuntu installation guide](docs/install/ubuntu.md) covers device setup,
upgrading and source builds. Each release includes a `.deb`, its SHA-256 checksum
and source/build metadata.

## Build and run

Requires Rust stable and C build tools. macOS additionally needs Xcode Command Line Tools/Swift. Linux currently needs X11 or XWayland, Python 3 + GTK4/PyGObject for the window, `notify-send` for notifications. Tailscale must already be running on each device.

```sh
cargo build --release -p shuttli-host --locked
python3 tools/install.py
shuttli ui
```

On Linux, the tray icon opens a menu with **Open window**, **Enable sending**, **Enable receiving**, **Send clipboard now**, and **Quit**. The icon distinguishes both directions enabled, send only, receive only, and both paused. Closing the window keeps the tray and agent running. **Quit** or `shuttli quit` stops the background agent and tray without changing synchronization preferences or login-start registration. A StatusNotifier-compatible panel is required (tested on Xfce); desktops without a tray host retain the launcher and CLI.

The installer writes to your user account (`~/.local/bin`; also `~/Applications/Shuttli.app` on macOS). Start `shuttli daemon` in the graphical login environment if you prefer CLI-only operation. The default CLI talks to that single agent, including when called from SSH/tmux.

```sh
shuttli devices
# Compare the full fingerprint shown on the other device before allowing sends.
shuttli peer <fingerprint> send on
shuttli set automatic on
# Optional: send the current clipboard immediately.
shuttli send
shuttli set history content
shuttli set history-limit 20
shuttli history
shuttli history copy <id> --local-only
shuttli history resend <id>
shuttli set receive off
shuttli autostart on
# Stop the background agent and tray.
shuttli quit
```

`shuttli --help` lists commands. Add `--json` for structured output. Settings and data live in `~/.local/share/shuttli` on Linux or `~/Library/Application Support/Shuttli` on macOS. `SHUTTLI_DATA_DIR` selects an isolated profile for development. History defaults to recent content: text/list in memory and temporary encrypted images on disk, with a session-only key. No Keychain/Secret Service is required. Restart clears history; eviction and `history clear` remove image files. Existing settings are preserved.

The desktop refreshes its Tailscale device list every 30 seconds and probes cached addresses every 5 seconds, skipping connected peers. Manual refresh updates the list and probes immediately.

The agent binds only its Tailscale IPv4 address, port 45987. At least one connection direction must be reachable; an established connection carries both directions. Device permission follows its public key, not its IP. The application does not alter firewall rules. If Tailscale is not ready at login, the network worker retries while local controls remain available; failed offline sends are not automatically replayed.

## Clipboard images

Image controls apply to clipboard image content. The wire format and encrypted cache use lossless PNG internally; users do not select an encoding. Linux currently reads `image/png` offered by the source application. macOS also converts TIFF clipboard data to PNG. Other source representations require adapter support; copying an image file in a file manager is part of the planned file-transfer feature.

## Validation

Core production function/line/region coverage must each be **strictly greater than 95%** in CI; test harness code is excluded.

```sh
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --version 0.9.1 --locked
python3 tools/verify_local.py
```

Real desktop E2E is explicitly selected with machine configs and consent flags; the verifier reports it as not run when no config is supplied. Run each driver with --help for its portable interface.

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all --check
python3 tools/check_architecture.py
python3 -m unittest discover -s tools/tests -v
```

[Real two-host test driver](tools/integration/two_hosts.py) requires explicit clipboard-overwrite opt-in and uses synthetic content. Platform-independent tests also run on Windows; its actual clipboard/host implementation is not included yet.

## Roadmap and planned features

Use the [grouped roadmap](https://github.com/KatInBroek/shuttli/issues/1) or [milestones](https://github.com/KatInBroek/shuttli/milestones) to find remaining work. Issues contain the current scope, dependencies and independent acceptance criteria.

- [Desktop design and UI requirements](docs/design/desktop.md)
- [Windows and regular-file extension contracts](docs/planned/windows-and-files.md)
- Mobile implementation and remaining acceptance: [feature requirements](docs/mobile/feature-spec.md), with [shared Rust SDK](docs/mobile/shared-sdk.md), [iOS](docs/mobile/ios.md), and [Android](docs/mobile/android.md) details

The specifications distinguish implemented features from remaining device acceptance and planned work. Mobile previews are for testing; store distribution and production signing remain separate.

Contributions are welcome: see [CONTRIBUTING.md](CONTRIBUTING.md). Report vulnerabilities privately using [SECURITY.md](SECURITY.md).

History success is a past delivery result, not a promise that another application has not since replaced the clipboard. Uncertain delivery triggers a bounded receipt query, never an automatic clipboard reapplication; an explicit history resend creates a new event.

## License

[MIT](LICENSE). Dependencies retain their respective licenses.

## Product name

The display name is maintained only in [`branding/name.txt`](branding/name.txt). See [branding configuration](branding/README.md) for the one-edit build workflow and stable technical identifiers.
