# Remaining desktop design and UI requirements

Track [design #49](https://github.com/KatInBroek/shuttli/issues/49) and the [desktop UI milestone](https://github.com/KatInBroek/shuttli/milestone/4).

Linux has an implemented native design and initial logo/tray assets. This brief covers the ongoing design contract, platform parity and unfinished acceptance. It is not a claim that macOS, Windows or every state has been delivered. Mobile has separate specifications and native layouts.

## Product and constraints

A lightweight resident clipboard utility for multiple devices. Keep sending, receiving, automatic mode and per-device consent understandable without protocol knowledge. Use native UI; no Electron or permanent WebView is needed. UI/CLI consumes the application API and cannot introduce another permission, clipboard or network implementation.

The display name comes from [branding/name.txt](../../branding/name.txt). Keep editable wordmarks and a shared symbol across languages. Refine assets where needed without replacing established product behavior. Prioritize clear states, restrained motion and readable content over dashboard decoration.

Current scope is Tailscale, desktop text/images, permissions, bounded session history, native tray/window and a normal CLI. LAN, files, Windows and mobile are separately tracked extensions; do not present unavailable controls as functioning. SSH/tmux is a usage context: this application updates the system clipboard and does not inject Ctrl+V or guarantee terminal-specific image ingestion.

## Required product semantics

| Control or state | Meaning |
| --- | --- |
| Global send and receive | Independently enabled by default; each can pause its direction while preserving per-peer settings. |
| Automatic mode | Enabled on desktop by default; independent of send permission. Enabling any switch never sends previous content. |
| New device | Send off, receive on. Explicitly disclose that enabled desktop reception may replace the local clipboard. |
| Allow sending | Local, per identity and direction. A-to-B consent does not grant B-to-A or authorize C. |
| Block/type restrictions | Combine with global and per-peer controls. Manual/history actions cannot bypass them. |
| History | Default 20 recent events, bounded RAM text/list and encrypted temporary image cache; restart clears content. |
| Notifications | Optional, with per-peer quiet controls; never grant permission or expose bodies/thumbnails. |
| Start at login | Off by default, local interactive user only; actual platform result must be visible. |

Show full fingerprint comparison before allowing a new outgoing identity. Names/IPs do not establish trust, and key changes do not inherit consent. Peer history may inherit or tighten the global mode, never loosen it. Remote content is never automatically forwarded.

Explain privacy accurately: application TLS protects transport, no central clipboard store, and Tailscale itself may use relays. Do not promise absolute security, that every byte stays off disk, that history clearing erases other apps, or that historical success guarantees current paste readiness. Put advanced fingerprints/backend diagnostics in details rather than forcing implementation terminology into primary flows.

## Brand and asset delivery

Provide editable symbol, wordmark and combinations; color, monochrome and reversed variants; clear space/minimum sizes and usage rules; application icons and tray assets separately. Preserve source/license attribution suitable for open-source redistribution. If proposing a new direction, compare a small set of distinct concepts and small-size/monochrome results before a complete asset replacement.

Tray exports must include 16/24/32-pixel variants, light/dark surfaces and platform-native template/resource needs. Use semantic names and traceable dimensions/themes/formats. Do not borrow another product's trademark or rely solely on a shield/lock to communicate trust.

## Tray contract

| ID | State |
| --- | --- |
| T01 | Sending and receiving enabled |
| T02 | Sending only |
| T03 | Receiving only |
| T04 | Both paused; the tray entry remains visible |
| T05 | Agent status unavailable; opening the window remains possible but sync actions are disabled |

Use shape as well as color. The icon describes global directions, not peer count, automatic/manual mode or successful delivery. Service unavailability takes priority over optional activity overlays; define overlay recovery and stop animations when idle. Do not merge network offline, backend unavailable and service disconnected into one misleading state.

Native menu: device permission counts (`↑` send, `↓` receive), Open window, Enable sending, Enable receiving, Send clipboard now, Quit. Counts use the same status snapshot as the window: discovered devices whose local per-device and global direction switches are both on, including offline devices. A disabled global switch yields zero; stored policies for undiscovered devices are excluded. These counts do not imply remote permission or successful delivery. Send now is disabled when sending is off and still requires allowed targets. Read actual results before showing a saved toggle; failures restore service truth. Changes made in CLI/window update tray/menu. Current tray refresh is approximately two seconds. Opening repeatedly focuses one window; closing it releases previews and leaves tray/agent running. Quit calls the shared lifecycle API (`shuttli quit` in CLI), revokes pending synchronization and exits the agent/tray while preserving preferences and login-start registration.

## Four primary views

**Status:** actual service/backend availability, a compact discovered / allowed-to-send / allowed-to-receive statistics area before Directions, global directions, automatic/manual mode and Send now; recent result links and expandable diagnostics. Command acceptance is not delivery success. Do not automatically expose current clipboard content without an explicit preview API.

**Devices:** name, distinguishable identity, connection state and directional consent. Details show full fingerprint, address, send/receive, text/images, quiet and tighter history policy. Explain effective global restrictions without erasing peer preferences. Preserve long names and distinguish identical names. Only display OS, network path, last-seen or latency when the API supplies them. Rename/forget controls require an API rather than presentation-only behavior.

**History:** local copies, outgoing deliveries and incoming source; timestamp, type, outcome and body availability. Group by event while exposing every target's result; partial failure is never all-success. API pages may split one event, so grouping must account for the complete bounded metadata set. Count retention by event, subject to total metadata/body limits. Render text safely as plain text; images decode on demand. Empty/off/status/cleared/restarted states must be distinct.

| Action | Required behavior |
| --- | --- |
| Copy | Write locally; may publish under current desktop automatic/policy rules. Explain this at the action. |
| Copy locally | Write locally without publication for this operation. |
| Resend | New sending event, same selected retained body, current permissions; preserve the original result and do not alter local clipboard. |
| Clear history | Confirm deletion of app history/cache; no clearing of system clipboards or recall of deliveries. |

Disable unavailable preview/copy/resend with a reason. Never replace the selected missing body with current clipboard content. Viewing, deleting or closing history does not publish. Preserve historical success separately from current readiness.

**Settings:** global directions, automatic mode and types; notifications; Off/Status/Content history; 0-10,000 event limit with default 20; advanced RAM/image quotas; clear; actual login-start state; system/English/Nederlands/Deutsch/Français language. Preserve unsubmitted history-limit edits when external direction updates arrive. Show pending, failed or system-blocked operations truthfully.

## Layout, states and accessibility

Use the native window shell and menus. Target an approximately 850x620 logical-pixel default and validate around 720x520 minimum, larger windows, high DPI and 200% text. Scroll where needed without hiding essential actions. Support light/dark and high contrast; adequate focus, keyboard traversal, semantic screen-reader labels and reduced motion. Long German/French text and device names must fit without losing the primary action.

Use real localized strings and stable semantic keys. Language preference is local, never synchronized, and must not change sync policy. Unknown technical diagnostics remain accurate rather than receiving invented translations.

Cover sending, receiving, success, failure, cancelled, superseded and unknown, plus partial multi-target outcomes. Byte progress must come from the service; writing socket bytes cannot mean remote application. Update/withdraw current-ready indications after replacement or disconnect without erasing historical success. Notification permission denial or lack of update support must not create a popup storm. Status remains accessible in the window.

First-use flows explain external Tailscale, running the app on another device, new-peer defaults and consent. Include no peers, refresh, unknown diagnostics, offline targets, missing clipboard backend, service loss, save/cleanup failures and recovery. Discovery may take a scan interval; keep existing rows during refresh. Persistent new-device/read markers require an explicit model/API.

## Platform and extension boundaries

Adapt macOS menus, shortcuts, window chrome, notifications and login integration natively. Windows has separate host/installer acceptance. Share assets and semantics without introducing a heavy cross-platform runtime solely for pixel matching.

Reserve file type/count/size/progress/unavailable layouts, but do not expose working file actions until the file contract is delivered. File preview must not execute or fetch arbitrary content. Active file-export cleanup differs from session history. Mobile uses native foreground inbox/send flows defined in its own specifications.

## Deliverables and acceptance

Deliver editable components/tokens/layouts, logo/icon/tray sources and exports, light/dark and four-language states, keyboard/screen-reader notes, clickable failure/success flows, API bindings and missing dependencies. Screenshots alone are not an editable handoff. Use only synthetic content, names, images and fingerprints; keep private device information out of assets and issues.

| ID | Required evidence |
| --- | --- |
| D01 | Symbol, app icon and tray roles are distinct; small/monochrome/light/dark exports remain recognizable. |
| D02 | T01-T05 and native menus, including a visible fully paused entry. |
| D03 | New-peer outgoing off, incoming on, clear initial consent explanation. |
| D04 | Independent A-to-B/C directions and preserved peer settings under global pause. |
| D05 | Automatic mode independent of send switch; enabling does not replay. |
| D06 | Text/image transfer states and notifications include failure/unknown. |
| D07 | Mixed target success/failure/unknown is represented honestly. |
| D08 | All three history actions, missing bodies and long text/image preview. |
| D09 | Default 20, quota tightening, clear confirmation/failure and restart-empty behavior. |
| D10 | Tailscale unready, no peers, clipboard unavailable and lost agent. |
| D11 | Save failure, external CLI updates and unsubmitted field edits. |
| D12 | Actual login-start state and window/background lifecycle. |
| D13 | Four real languages, long names and 200% text. |
| D14 | Keyboard, reader labels, contrast, grayscale and reduced motion. |
| D15 | Notifications exclude bodies/thumbnails; disabling does not affect synchronization. |
| D16 | Native platform differences and explicit unsupported LAN/file capabilities. |
| D17 | Each action maps to an API or a recorded missing dependency; no invented progress/online status. |
| D18 | Editable sources, reproducible exports and redistributable asset licenses. |

Prototype first consent/success, tray pause/resume, manual send/failure, history preview/local-copy/resend, language/history/login settings, service outage and recovery. Track design coverage separately from implemented/verified behavior.
