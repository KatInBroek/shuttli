# Ubuntu installation

The current desktop preview targets **Ubuntu 24.04 or newer, amd64** (Intel/AMD).
Ubuntu 22.04 and ARM64 release packages are not yet validated. The package builder
also accepts native ARM64 binaries, but this is not a tested release promise.

## Install

Download the `.deb` and its `.deb.sha256` from the
[current preview release](https://github.com/KatInBroek/shuttli/releases/tag/v0.1.0-preview.1).
Choose the `.deb` under **Assets**; GitHub's automatic source-code archives are
not installers. Public release downloads do not require a GitHub account.
In their directory, run:

```sh
sha256sum --check shuttli_0.1.0_amd64.deb.sha256
sudo apt install ./shuttli_0.1.0_amd64.deb
shuttli ui
```

The application is also available in the desktop application menu. `shuttli` is
the stable command/package identifier; the visible product name comes from
`branding/name.txt`. No Rust compiler or source checkout is needed to install.

APT installs GTK4, PyGObject and notification dependencies. Dependencies must be
available from the configured Ubuntu repositories or an offline APT mirror.
The application does not install or log into Tailscale on your behalf. Install
[Tailscale](https://tailscale.com/download/linux) separately and connect the devices
to the same tailnet. Discovery currently uses Tailscale; standalone LAN discovery
is not implemented. Tailnet policy/firewall rules must permit the application
connection on TCP 45987 between the selected devices.

1. Open the application in a graphical login on both devices.
2. Compare the complete device fingerprints, then allow sending to the intended
   device. Outgoing permission is off for newly discovered devices.
3. Automatic sync and global receiving default to on. For two-way sync, allow
   sending on both sides. Global/per-device switches can pause either direction.
4. Copy synthetic text first and check the history/delivery status.
5. Enable start at login in Settings if desired. Installation does not enable it.

On Ubuntu GNOME, enable the Ubuntu AppIndicators extension if the tray icon is
missing. Other desktops need a StatusNotifier-compatible panel. The full window
and CLI remain available without a tray host. The Linux clipboard backend uses
X11/XWayland: Wayland desktop clipboard bridging depends on the compositor and
must be checked on the target desktop; pure Wayland without XWayland is unsupported.

## Upgrade and remove

Run `shuttli quit` before upgrading, install the new `.deb` with APT,
then reopen it. If migrating from a source/user install, disable its autostart,
stop that instance, and remove or rename the old `~/.local/bin/shuttli` and its
user desktop launcher so they do not shadow `/usr/bin/shuttli`. Re-enable
autostart from the packaged application afterwards.

```sh
shuttli autostart off
sudo apt remove shuttli
```

Removing the package does not delete per-user identities or settings. To remove
those too, first stop the application and explicitly delete its profile yourself.
The default profile is `~/.local/share/shuttli`; `SHUTTLI_DATA_DIR` overrides it.

## Build the package from source

Use a clean checkout on Ubuntu 24.04 with Rust stable, C build tools, Python 3.11+
and `dpkg-dev`. Build on the oldest supported release; packaging a binary built
against newer glibc does not make it compatible with older Ubuntu.

```sh
sudo apt install build-essential dpkg-dev python3
cargo build --locked --release -p shuttli-host
python3 tools/package_deb.py
```

Outputs go to `target/packages/`: package, SHA-256 checksum and public build
metadata (source commit, compiler, architecture and binary hash). Only explicit
program/icon/launcher/documentation paths enter the package; user profiles,
device permissions, keys, history and local test data are excluded. The package
has no install/remove maintainer scripts and does not run the application as root.

CI builds and smoke-tests packages, and a manual packaging workflow exports the
same artifacts. Checksums detect file corruption; they are not a publisher
signature. Builds are traceable to source; bit-for-bit reproduction across
different compilers and environments is not currently guaranteed.
