# iOS foreground app specification

Status: proposed; not implemented or device-validated. [Issue #51](https://github.com/KatInBroek/shuttli/issues/51). Prerequisite: [shared SDK](shared-sdk.md). Android has its [own specification](android.md).

## Scope and platform

Use Swift/SwiftUI and the shared Rust SDK with the separately installed official Tailscale app. Proposed minimum iOS 16 must be confirmed by the build spike. Prioritize phone layouts; validate iPad separately.

When open, discover desktops without QR codes or addresses, explicitly import/send text or images, receive into an inbox, preview, copy locally, resend history, and manage global/per-device controls. No background clipboard monitoring, lock-screen delivery, automatic send on launch, share extension, shortcuts, APNs, files, LAN, phone-only initial discovery, embedded VPN, cloud history or autostart setting is included.

## Discovery and consent

An upgraded desktop obtains the phone address from Tailscale and connects to its foreground listener on the application port. Display a device only after TLS/application authentication. Deduplicate public-key identities; name/address changes preserve policy, key changes create a new device. Receiving defaults on, sending to new peers defaults off. Offer full-fingerprint comparison before allowing each outgoing direction; no bidirectional pairing ceremony is required.

Explain that received content enters the app and does not automatically overwrite the OS clipboard. Desktop-assisted discovery targets 40 seconds on a healthy network; phone refresh does not force unknown desktops to rescan. Diagnose blocked inbound policy accurately without assuming an empty list means Tailscale is logged out.

## Explicit import and send

Use a native PasteButton/UIPasteControl entry. Opening the app must not repeatedly read the clipboard or demand permanent access. After user paste, show a bounded snapshot preview, type, size and allowed destinations. Import itself does not send. Send publishes that snapshot to all currently eligible targets; no per-send target picker is required initially.

A later clipboard change does not replace a displayed draft. Another explicit paste replaces it. Release drafts on background entry or clear; resuming never replays them. Text is limited to 1 MiB; internal encoded images to 8 MiB, 4,194,304 decoded pixels and 16,384 per edge. Preflight sizes before allocation/decoding. Adapter conversion accepts supported source image representations; it does not promise every file encoding.

Distinguish cancellation, unsupported/empty content, sensitive markers, import failure, size limits, no target, disabled policy and network failure. Rejecting paste access must leave reception and device management usable.

## Inbox, copy and history

Validated incoming content enters the bounded active inbox and returns Received. Show source, time, content type, preview and actual delivery result. Only an explicit Copy locally action requests an OS write through a fresh core permit. Independently verify readback where available; otherwise display unverified. Never substitute app buffers for OS readback or change the earlier network receipt to Applied. No automatic forwarding follows reception or local copy.

Render text as plain text. Decode image previews on demand and release large objects when leaving details. Evicted, replaced or cleared content cannot silently become another item's body. Test actual text/image paste into other apps.

Defaults: global send/receive on, new-peer send off/receive on, manual sending. Global/per-peer block, type, history and quiet controls apply to every send/resend/receive path. Failed persistence returns actual state and preserves fail-closed behavior.

History defaults to 20 events (0-10,000, subject to quotas); modes Off/Status/Content; preview, local copy, resend and clear. Text/list data is session memory and images are encrypted temporary objects. History off retains only one bounded active inbox item. History and active items share capacity. Clear removes app drafts/inbox/cache, not system clipboards or remote deliveries. Process restart restores identity/policy/replay/receipts but no history bodies.

## UI and lifecycle

Provide Send/Receive, Devices, History and Settings using native navigation and the shared logo. Support system language plus English, Dutch, German and French; VoiceOver, dynamic type, light/dark themes and long names. States must not depend only on color.

Only active state admits new content operations; background entry closes listeners and network tasks. Temporary inactivity, such as a system paste prompt, must not erase the entire session history. Discard late callbacks using session IDs. Lock, termination, network changes and resume never replay a draft. Interrupted transfers become cancelled/failed/unknown according to actual evidence; completed receipts retain their historical meaning.

Use in-app hints by default; system notification permission is not required. Never include bodies or thumbnails in system notifications. No background keep-alive, autostart or notification-permission loop. Protect sandbox identity/policy files and exclude them from backups/logs; cache encryption uses a memory-only session key. Uninstall or lost identity creates a new peer, never inherited send consent. Validate actual Tailscale interface restrictions on a real device.

Design waiting/discovery, identity/consent, draft, per-target progress, inbox, copy outcome, history, settings and failure screens. Differentiate Received, desktop Applied, unknown, cancelled, superseded, missing content and persistence/cleanup failure. Unknown diagnostics must remain unknown. Coordinate assets through [desktop design #49](https://github.com/KatInBroek/shuttli/issues/49).

## Independent tasks

| ID | Acceptance |
| --- | --- |
| I01 | Real iPhone foreground listener discovered by Linux without QR; TLS identity and bidirectional synthetic data over Wi-Fi/cellular and resume. This is a feasibility gate. |
| I02 | Xcode/SDK integration, MainActor safety, explicit text/image import and independent copy/readback tests. |
| I03 | Four views, permission/history states, four languages, accessibility and bounded layouts. |
| I04 | Full device matrix below, OS paste verification and separate native/Rust test reports. |
| I05 | Reproducible signed archive/install/upgrade preserving identity; credentials stay external. TestFlight/App Store publication is a separate operation. |

Depend on M01-M05, not Android UI completion. Track app/SDK versions and device evidence independently.

## Acceptance matrix

| ID | Required result |
| --- | --- |
| IOS-01 | Authenticated first discovery without QR, target <=40 seconds, new-peer outgoing disabled. |
| IOS-02 | Multiple computers and duplicate sessions show one row per identity; directions remain independent. |
| IOS-03 | Explicit paste plus send yields matching independent Linux clipboard readback; opening alone sends nothing. |
| IOS-04 | Incoming preview is correct, sender reports Received and phone clipboard is unchanged. |
| IOS-05 | Other apps paste copied text/images; result is truthful and nothing is forwarded automatically. |
| IOS-06 | Global/per-peer/type restrictions cover sends, resends and reception; re-enabling never replays old content. |
| IOS-07 | History count 20/0, status mode, policy tightening, quota/clear and ciphertext deletion behave correctly; unavailable actions cannot substitute bodies. |
| IOS-08 | Background, lock, termination and restart cause no background synchronization/replay; identity persists, content does not survive restart. |
| IOS-09 | Disconnects, lost receipts and duplicate events remain truthful; queries do not reapply and clearing history preserves replay protection. |
| IOS-10 | Cancelled paste, oversized or corrupt images fail within bounds without disabling unrelated pages. |
| IOS-11 | Tailscale, ACL and version failures distinguish known from unknown causes; never fabricate connected/Applied. |
| IOS-12 | Four languages, VoiceOver, large text, dark theme and repeated lifecycle cycles work without sustained resource growth. |

All mobile acceptance remains pending. Store device/version/configuration, synthetic digests and reproduction evidence outside the repository. Core coverage and simulator builds are not iOS acceptance.

References: [system paste controls](https://developer.apple.com/documentation/uikit/uipastecontrol), [background execution](https://developer.apple.com/forums/thread/685525).
