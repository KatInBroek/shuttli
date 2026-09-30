# iOS foreground app specification

Status: implementation in progress; build, network flow and real-device acceptance remain pending. The [mobile feature specification](feature-spec.md) owns product requirements. [Issue #51](https://github.com/KatInBroek/shuttli/issues/51). Prerequisite: [shared SDK](shared-sdk.md); the temporary-Mac workflow is in the [development and test plan](ios-development-plan.md). Android has its [own specification](android.md).

## Scope and platform

Use Swift/SwiftUI and the shared Rust SDK with the separately installed official Tailscale app. Proposed minimum iOS 16 must be confirmed by the build spike. Prioritize phone layouts; validate iPad separately.

When open, discover desktops without QR codes or addresses, import/send text or images only after user action, fetch currently permitted recent retained copies from those desktops, preview and explicitly copy items to the phone clipboard, resend phone-origin history, and manage per-device send/receive permissions. The phone UI has no global direction switches. No background clipboard monitoring, lock-screen delivery, automatic send on launch, share extension, shortcuts, APNs, files, LAN, phone-only initial discovery, embedded VPN, cloud history or autostart setting is included.

## Phone-first interaction

The phone is an action surface, not a smaller always-on desktop agent. Opening or resuming the app never reads or writes the system clipboard and never sends its contents. Desktop history catch-up is a separate, bounded foreground operation; it must not be described as automatic phone clipboard synchronization.

| Screen | Primary content and actions |
| --- | --- |
| Home | Connection/catch-up state, a native **Paste to send** control and the latest unified history items. Show the number of currently allowed send targets before the paste action. |
| History | One newest-first timeline of phone-origin sends and currently permitted desktop copies fetched on opening. Filter All / From devices / Sent from this phone, then filter by source device. A row shows source, copy time, text/image type, availability and an honest result; identical bytes copied twice remain two rows. |
| Item detail | Preview the exact immutable item. Remote items offer **Copy to this phone**; phone-origin items offer **Resend**. A remote item is never silently relayed to another device. Outgoing details group destination results under one originating copy. |
| Devices and Settings | Per-device send/receive consent and health; history retention, language and privacy controls. No global direction switches. |

The two main user journeys are:

1. Copy in another app → open Shuttli → tap the system paste control → inspect the frozen preview and named allowed destinations → tap **Send to N devices** → see separate sending/success/failed/unknown results for each destination. Switching apps or returning to Shuttli does not perform the paste or send step.
2. Open Shuttli after being away → query each permitted source's recent history while foreground → browse the merged timeline → tap an item to load/inspect its content → tap **Copy to this phone** → paste it into another app. Fetching or previewing does not change the phone clipboard.

The UI must show **Updating**, **Last checked at**, **Some devices unavailable**, and **Content no longer available** as distinct states. A successful metadata refresh is not proof that every offline-period copy was recovered. Keep the last locally cached session rows visible when a peer goes offline; mark their freshness instead of clearing the list.

## Discovery and consent

An upgraded desktop obtains the phone address from Tailscale and connects to its foreground listener on the application port. Reuse the existing TLS identity and per-device send-consent flow. A newly connected computer is visible after authentication, but sends no roster or history until its own send-to-phone permission is enabled. Every directly connected computer with `peer_hints` capability and effective permission to send to the phone then shares its own bounded snapshot of directly authenticated Shuttli peers. The phone merges snapshots by public-key identity, tracks each hint's source and expiry, then contacts each suggested computer directly or reuses its own session with it. That computer independently authenticates the phone and decides whether it may send history. A hint is never a trust or permission grant, and a computer never relays a hint-only peer. Display a device only after TLS/application authentication. Name/address changes preserve policy; key changes create a new device. Receiving from new peers defaults on, sending to new peers defaults off. Offer full-fingerprint comparison before allowing each outgoing direction; no bidirectional pairing ceremony is required. Explain that allowing send also grants access to currently retained local history, including copies made before this permission was granted.

Explain that fetched content enters the app and does not automatically overwrite the OS clipboard. Desktop-assisted discovery targets 40 seconds on a healthy network; phone refresh does not force unknown desktops to rescan. Diagnose disabled per-device receiving accurately without assuming an empty list means Tailscale is logged out. After each authenticated computer connection, query bounded history when that computer is allowed to send and the phone permits receiving from it. Computers send lightweight history-change hints, never unsolicited clipboard bodies to a peer without `accept_live_offer`. Each permitted computer sends a peer snapshot on connection or permission enablement and updates it as its verified online peers change. Unverified hints expire and are never displayed as trusted devices.

## Explicit import and send

Use a native PasteButton/UIPasteControl entry. Opening the app must not read the clipboard or demand permanent access. After user paste, show a bounded snapshot preview, type, size and named allowed destinations. Import itself does not send. Send publishes that snapshot to all currently eligible targets; no per-send target picker is required initially. If there are no targets, explain the missing permission or connection before offering Send. The preview and separate confirmation protect against another app replacing the clipboard while the user switches apps.

A later clipboard change does not replace a displayed draft. Another explicit paste replaces it. Release drafts on background entry or clear; resuming never replays them. Text is limited to 1 MiB; internal encoded images to 8 MiB, 4,194,304 decoded pixels and 16,384 per edge. Preflight sizes before allocation/decoding. Adapter conversion accepts supported source image representations; it does not promise every file encoding.

Distinguish cancellation, unsupported/empty content, sensitive markers, import failure, size limits, no target, disabled policy and network failure. Rejecting paste access must leave reception, history catch-up and device management usable.

## Foreground history catch-up and merge

Because the phone is active only briefly, each authenticated desktop exposes a bounded list of its **own retained local-origin copies**. Existing desktop history records local copies whether automatic sending was on, the phone was online, or the phone was already known. No per-phone offline eligibility marker is needed. At list and body fetch, the desktop checks the phone's authenticated identity and current effective send, content-type and history settings. Enabling send may therefore expose copies made before permission was granted or while automatic sending was off, if they remain in history. The desktop automatic switch controls live publication, not a permitted history request. It must not export another device's original received event. If the phone's per-device receive permission is off, do not fetch metadata or bodies.

On foreground connection, request recent metadata from each eligible desktop, page within limits and fetch bounded content into the phone's existing session history budget. Keep text/list data in RAM and encrypted image objects in a temporary cache. Content can also be fetched on item open if still available at its source; show **Content no longer available** if eviction or disconnection wins that race. While open, use lightweight history-change hints to request incremental updates and update the visible timeline. Reconcile the bounded window after reconnect or a missed change. Refresh on demand and after a new authenticated connection. Do not poll while backgrounded, do not wake a suspended app and do not keep a background listener.

Merge by the wire event ID `(origin public-key identity, epoch, sequence)`, never by text, image hash, name or address. Repeated history results for the same event update one row; distinct copies with identical bytes remain distinct. Sort by the origin's copy time with a stable event-ID tie-breaker; a significant device clock mismatch must be signalled instead of claiming exact cross-device order. Preserve source device and acquisition path in detail. A phone-origin copy groups all destination outcomes under one row; a desktop-origin row must not claim **Applied** on the phone merely because its metadata or body was fetched.

This is recent-history catch-up, not guaranteed archival delivery. Desktop text/history remains session memory and is bounded by its configured count/time/byte limits; a desktop restart, disabled history, eviction or offline source can leave gaps. Show each source's last successful refresh and partial/error state. Never imply that an empty refreshed list proves nothing was copied while the phone was closed. Do not introduce a central server or silently persist text history to meet this feature.

## History, local copy and resend

History metadata is **Listed**; validated content cached in the bounded app session is **Available in app**. Neither is a delivery receipt. Show source, time, content type, preview and availability. Only an explicit Copy to this phone action requests an OS write through a fresh core permit. Independently verify readback where available; otherwise display unverified. Never substitute app buffers for OS readback or claim desktop **Applied** from a local copy. No automatic forwarding follows history fetch or local copy.

Render text as plain text. Decode image previews on demand and release large objects when leaving details. Evicted, replaced or cleared content cannot silently become another item's body. Test actual text/image paste into other apps.

Defaults: new-peer send off/receive on, manual sending and manual phone clipboard copy. Per-peer direction, type, history and quiet controls apply to every send/resend/history query. The phone UI has no global direction switches. Failed persistence returns actual state and preserves fail-closed behavior.

History defaults to 20 merged events (0-10,000, subject to quotas); modes Off/Status/Content; preview, local copy, phone-origin resend and clear. Text/list data is session memory and images are encrypted temporary objects. History Off disables queries and cached history; explicit import/send remains usable. History and drafts share capacity. Clear removes app drafts/cache, not system clipboards or remote deliveries. Process restart restores identity/policy/replay/outgoing receipts but no history bodies; a later foreground catch-up may repopulate still-available authorized desktop items.

## UI and lifecycle

Provide Home (merged history), Devices and Settings using native navigation and the shared logo. Support system language plus English, Dutch, German and French; VoiceOver, dynamic type, light/dark themes and long names. States must not depend only on color.

Only active state admits new content operations; background entry closes listeners and network tasks. Temporary inactivity, such as a system paste prompt, must not erase the entire session history. Discard late callbacks using session IDs. Lock, termination, network changes and resume never replay a draft. Interrupted transfers become cancelled/failed/unknown according to actual evidence; completed receipts retain their historical meaning.

Use in-app hints by default; system notification permission is not required. Never include bodies or thumbnails in system notifications. No background keep-alive, autostart or notification-permission loop. Protect sandbox identity/policy files and exclude them from backups/logs; cache encryption uses a memory-only session key. Uninstall or lost identity creates a new peer, never inherited send consent. Validate actual Tailscale interface restrictions on a real device.

Design waiting/discovery, identity/consent, paste preview, per-target progress, unified history, source freshness, local copy outcome, settings and failure screens. Differentiate Listed, Available in app, local copy result, desktop Applied for phone-origin sends, unknown, cancelled, superseded, missing content and persistence/cleanup failure. Unknown diagnostics must remain unknown. Coordinate assets through [desktop design #49](https://github.com/KatInBroek/shuttli/issues/49).

## Independent tasks

| ID | Acceptance |
| --- | --- |
| I01 | Real iPhone foreground listener discovered by Linux without QR; TLS identity and bidirectional synthetic data over Wi-Fi/cellular and resume. This is a feasibility gate. |
| I02 | Xcode/SDK integration, MainActor safety, explicit text/image import and independent copy/readback tests. |
| I03 | Four views, permission/history states, four languages, accessibility and bounded layouts. |
| I04 | Full device matrix below, OS paste verification and separate native/Rust test reports. |
| I05 | Reproducible signed archive/install/upgrade preserving identity; credentials stay external. TestFlight/App Store publication is a separate operation. |
| I06 | Foreground multi-desktop history catch-up, event-ID merge, source freshness, lazy content and explicit phone copy. Validate restarts, evictions, revoked consent and incomplete history without claiming archival completeness. |

Depend on M01-M05, not Android UI completion. Track app/SDK versions and device evidence independently.

## Acceptance matrix

| ID | Required result |
| --- | --- |
| IOS-01 | Authenticated first discovery without QR, target <=40 seconds, new-peer outgoing disabled. |
| IOS-02 | Multiple computers and duplicate sessions show one row per identity; directions remain independent. |
| IOS-03 | Explicit paste plus send yields matching independent Linux clipboard readback; opening alone sends nothing. |
| IOS-04 | Queried history metadata/body show the right preview and availability; querying never changes the phone clipboard or creates a desktop delivery receipt. |
| IOS-05 | Other apps paste copied text/images; result is truthful and nothing is forwarded automatically. |
| IOS-06 | Per-peer/type/history restrictions cover sends, resends and history queries; granting desktop send can expose older retained local history, as explained beside the permission control, while re-enabling phone receipt never writes content to the OS clipboard. No global mobile direction controls appear. |
| IOS-07 | History count 20/0, status mode, policy tightening, quota/clear and ciphertext deletion behave correctly; unavailable actions cannot substitute bodies. |
| IOS-08 | Background, lock, termination and restart cause no background synchronization/replay; identity persists, content does not survive restart. |
| IOS-09 | Disconnects, lost receipts and duplicate events remain truthful; queries do not reapply and clearing history preserves replay protection. |
| IOS-10 | Cancelled paste, oversized or corrupt images fail within bounds without disabling unrelated pages. |
| IOS-11 | Tailscale, ACL and version failures distinguish known from unknown causes; never fabricate connected/Applied. |
| IOS-12 | Four languages, VoiceOver, large text, dark theme and repeated lifecycle cycles work without sustained resource growth. |
| IOS-13 | Copies made on two authorized desktops while the phone is closed merge into one foreground timeline, even when no delivery attempt occurred; repeated query results deduplicate by event ID, identical content from separate copies does not. While the phone is open, new copies update that timeline through change hints. No desktop relays another origin's history. |
| IOS-14 | Opening, fetching and previewing never read/write the phone clipboard or send its contents; a copied remote item enters the OS clipboard only after the user's Copy action. |
| IOS-15 | Source restart, offline state, eviction, disabled history and changed consent produce honest partial/unavailable states; current denial or revocation blocks list and body fetch, while later authorization can reveal still-retained older local copies. |
| IOS-16 | Every permitted `peer_hints` computer sends its own current snapshot; multi-source hints merge by public-key identity with source/expiry tracking. Connection or permission enable produces a snapshot, peer join/leave updates it, and reconnect refreshes it. The phone verifies a suggested desktop directly and can fetch its history only when that desktop independently permits sending. Forged/stale hints and later permission revocation confer no access; no platform-kind flag drives policy. |

All mobile acceptance remains pending. Store device/version/configuration, synthetic digests and reproduction evidence outside the repository. Core coverage and simulator builds are not iOS acceptance.

References: [system paste controls](https://developer.apple.com/documentation/uikit/uipastecontrol), [background execution](https://developer.apple.com/forums/thread/685525).
