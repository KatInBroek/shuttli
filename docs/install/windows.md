# Windows preview

Download the Windows x64 ZIP from [GitHub Releases](https://github.com/KatInBroek/shuttli/releases),
extract the whole folder and open `shuttli-ui.exe`. Tailscale must be installed
and connected separately. The ZIP is unsigned. To upgrade, quit Shuttli from
its notification-area menu and replace all three executables together. Keep
the existing profile directory to preserve identity and device permissions.

The release includes SHA-256 checksums and source/build metadata. Windows
and Linux peers do not have to upgrade simultaneously; upgrading the Linux
receiver is required for the empty-clipboard reception fix.

## Build from source

Windows 10/11 x64 requires Tailscale running in the interactive user's session,
Rust stable, Visual Studio C++ Build Tools and .NET Framework 4.8. The native
helper uses WPF for the window, Win32 clipboard APIs and user/session restricted named
pipes. Rust application policy, TLS, permissions and storage are shared with the
other desktop platforms.

```powershell
cargo build --release -p shuttli-host --locked
.\target\release\shuttli.exe ui
```

Keep `shuttli.exe`, `shuttli-windows.exe` and `shuttli-ui.exe` in the same directory.
Double-click `shuttli-ui.exe` to open the desktop app without a console. `ui` opens
the window and starts the agent if needed. Closing the window keeps it in the
notification area; **Quit agent** exits both. CLI-only use starts with
`shuttli.exe daemon`; use another terminal for `status`, `devices` and `quit`.

Data defaults to `%LOCALAPPDATA%\Shuttli`, restricted to the current user.
`SHUTTLI_DATA_DIR` selects an explicit profile. New devices require outgoing
fingerprint approval. Compare the full fingerprint before allowing sends.
Text and PNG images are supported, with DIBV5 output and bounded 24/32-bit DIB
input. Unsupported image masks/color profiles fail rather than silently changing
pixel interpretation. Files are not supported.

Autostart is opt-in: `shuttli.exe autostart on`. If Windows has disabled the
entry, enable it in **Settings → Apps → Startup**. `autostart off` removes only
Shuttli's entry. Copying/building never enables autostart.

The native window follows the approved desktop design with Status, Devices,
History and Settings views. See the [Windows design mapping and reproducible
visual checks](../design/windows.md).

## Opt-in local acceptance

These tests temporarily replace the Windows clipboard with synthetic fixtures.
Close other Shuttli agents first. Use a fresh profile outside the repository:

```powershell
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build -p shuttli-host --locked
cargo build -p shuttli-adapters --example windows_acceptance --locked
.\tools\integration\windows.ps1 -BinaryDirectory (Resolve-Path .\target\debug) -ProfileDirectory "$env:TEMP\shuttli-test-$([guid]::NewGuid())"
```

The acceptance driver creates a temporary synthetic peer, verifies the running
agent's TLS identity, exchanges text/images over the local Tailscale interface,
and checks actual OS clipboard readback and APPLIED receipts. It also checks
history copy, singleton enforcement, shutdown and persisted settings/identity.
This local test does not establish cross-device, RDP or real-login acceptance.

The original supported text/image content is restored by the native/TLS tests;
arbitrary rich clipboard formats are not preserved. The history-copy test leaves
its synthetic text on the clipboard. No clipboard bodies are written to reports.
