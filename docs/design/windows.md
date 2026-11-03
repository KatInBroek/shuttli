# Windows desktop design adaptation

The Windows window follows the approved desktop v2 delivery and the current
[desktop behavior contract](desktop.md). It uses WPF and Windows notification-area
menus, without a WebView. Editable presentation source is in
`crates/native-ui/windows/Desktop.cs`; the four shared translation catalogs and
approved linked-sheets artwork remain the brand source.

## Design mapping

| Area | Windows presentation |
| --- | --- |
| Shell | Persistent sidebar, selected navigation state, four primary views, compact page header, native window chrome; 850 × 620 default, 720 × 520 minimum |
| Status | Direction hero, discovered/send/receive counts, independent switches, automatic mode, gated Send now, recent events and expandable diagnostics |
| Devices | New-device explanation, connection state and local directions; full identity and per-device text/image/history/quiet controls in details |
| Consent | Full selectable fingerprint and an unchecked confirmation; Allow sending stays disabled until checked |
| History | Complete bounded metadata paging before grouping, local copy plus every destination result, distinct missing-content state, explicit preview and all three copy/resend actions |
| Settings | Sync/content/notification preferences, retention and quotas, preserved drafts, actual login-start result, four languages and system/light/dark appearance |
| Tray | Approved T01–T05 state artwork, independent direction controls, counts from the same status snapshot, disabled unavailable actions, single-window activation and shared Quit lifecycle |

Windows uses Segoe UI, native window borders and standard platform focus/keyboard
behavior. Rounded groups, spacing, blue switches and action hierarchy follow the
design. Prototype export overlaps are corrected by wrapping and scrolling.
Unsupported unread indicators, platform/latency guesses and file controls are not
invented. App version and product name come from the build.

Text size follows the Windows accessibility text scale up to 200%. Main content
and navigation can scroll independently; larger text never removes a primary view.
Light/dark use the same color roles, and Windows high-contrast colors override
the main surfaces/text. There are no continuous decorative animations.

## Reproducible visual and control checks

```powershell
cargo build -p shuttli-host --locked
.\tools\integration\windows_ui.ps1 -BinaryDirectory (Resolve-Path .\target\debug) -OutputDirectory "$env:TEMP\shuttli-ui-review" -PythonExecutable python
```

The explicit fixture driver exchanges only synthetic API data. It renders the
actual WPF visual tree for four languages, light/dark, minimum size, 2× DPI and
200% text. Outputs include the four primary pages, device/history details,
image/missing previews, both confirmations, paused and service-unavailable states.
It checks event grouping with mixed outcomes, missing-body action gating,
fingerprint confirmation, named keyboard-focusable navigation and retention draft
preservation across direction refreshes.

The render driver does not capture the desktop, modify the system clipboard or
grant trust to a real device. Desktop capture, mouse/keyboard interaction,
notification-area placement and native screen-reader verification require an
unlocked interactive desktop and are separate acceptance evidence.
