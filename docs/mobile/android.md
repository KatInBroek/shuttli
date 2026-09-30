# Android foreground app specification

Status: Android app and shared SDK implemented; emulator tests and unsigned APK/AAB packages pass, but real-device Tailscale acceptance remains pending. The [mobile feature specification](feature-spec.md) owns product requirements. [Issue #52](https://github.com/KatInBroek/shuttli/issues/52). Prerequisite: [shared SDK](shared-sdk.md); [iOS specification](ios.md) is independent.

## Scope and platform

Use Kotlin/Jetpack Compose with UniFFI bindings to the shared Rust SDK. The current build uses minSdk 29 and target/compile SDK 36. Prioritize phones; tablet/foldable layouts are separate work.

Use the official external Tailscale client, not an embedded VpnService. Support foreground discovery, explicit text/image import/send, recent multi-computer history queries and incremental updates, manual local copy, per-device send/receive consent and four languages. The phone UI has no global direction switches. Do not require a foreground service, background clipboard listener, default IME, accessibility service, root/Shizuku, persistent notification, wake lock or battery-optimization exemption. Share intents, files, LAN, phone-only discovery, FCM, cloud accounts and autostart are outside this first delivery.

## Discovery and explicit sending

An upgraded desktop discovers the phone through Tailscale, probes the application port and authenticates the foreground listener. One connection carries both directions. A newly connected computer is visible after authentication, but sends no roster or history until its own send-to-phone permission is enabled. Every directly connected computer with `peer_hints` capability and effective permission to send to the phone then shares its own bounded snapshot of currently reachable, directly authenticated Shuttli peers after connection or permission enablement, followed by coalesced updates. The phone merges hints by public-key identity, tracks their sources and expiry, and contacts suggested peers directly or reuses their sessions. Each source independently authenticates the phone and checks its own send policy before history export. A computer never relays a hint-only peer. Tailscale online state alone does not prove the application is connected. Target first discovery within 40 seconds on a healthy network; refresh cannot force an unknown desktop to scan. Diagnose absent desktops, policy denial, listener failure and VPN split-tunneling where known. Never scan the full address space or modify VPN settings automatically.

Read the clipboard only after an explicit paste action while the Activity is resumed and has window focus. A foreground service does not grant background clipboard access. Do not pre-read content to populate device lists or notifications, and never send on launch/resume.

Import plain text or supported image ClipData MIME/URI through bounded ContentResolver reads. Do not treat a URI as an arbitrary filesystem path. Permission denial, expiry, slow/remote providers, cancellation and timeout require bounded errors. Preflight sizes before decoding and normalize supported images internally to PNG: text <=1 MiB, encoded image <=8 MiB, <=4,194,304 pixels, <=16,384 per edge. Reject sensitive markers, invalid bodies and unsupported representations.

Show a draft snapshot, type, size and allowed targets. Send publishes the displayed snapshot to every currently eligible target; no initial per-send picker. A later clipboard change cannot silently replace a draft. Core policy checks apply even if UI controls are disabled. Explain no target, per-device/type restrictions, offline and size errors distinctly.

## History, local copy and image clipboard exports

History metadata is **Listed**; validated fetched content cached in the app session is **Available in app**. Neither is a delivery receipt or OS clipboard write. Show source/time/preview. Explicit local copy requests a fresh core write permit and never automatically forwards. Explicit resend of a phone-origin item creates another event. A local copy result does not turn a history query into desktop Applied.

Android image clipboard sharing requires a native readable URI, not raw image bytes, a private path or an encrypted cache filename. Provide a restricted ContentProvider or equivalent:

- Use unpredictable object IDs, correct MIME and minimal read grants. No directory enumeration or access to identity/cache/other history entries.
- Validate whether to stream decryption or create a separately controlled active export in A02.
- An exported clipboard image has an independent OS-use lifetime. Clearing history must not immediately break an already copied image when the user switches to another app to paste.
- Prefer retaining only the latest export initially, with a fixed quota and explicit expiry/replacement rules. A new copy may replace it; an unrecoverable URI after restart must become unavailable, never resolve to different content.
- A provider started by the OS may serve only explicitly exported content. It must not start discovery, networking, synchronization or clipboard reads.
- Finalize cleanup after clipboard replacement, open-reader handling and conservative bounds when callbacks are unavailable. Image-copy delivery is blocked until this lifecycle is defined and validated.

Readback must use an independent clipboard/provider path and verify readable bytes, not compare a buffer to itself. Text and image copy require separate cross-app tests.

## Policy, history and UI

Defaults: new-device send off/receive on, manual sending and manual phone clipboard copy. Apply per-peer direction, text/image, tighter retention and quiet rules. No global mobile direction switches. Offer full-fingerprint comparison; discovery cannot grant outgoing consent. Explain that allowing a device to receive also permits it to request currently retained local history, including copies made before permission was granted or while automatic sending was off. Persistence failure must report actual state and fail closed.

History defaults to 20 merged events, configurable 0-10,000 subject to quotas; Off/Status/Content modes. On opening, query each authorized computer's recent retained local-origin events under its current effective send, type and history settings and the phone's per-device receive permission, regardless of copy-time permission or automatic-send mode; while foreground, query incremental changes when notified. Computers send lightweight `HISTORY_CHANGED` hints, never unsolicited clipboard bodies to a peer without `accept_live_offer`. Merge repeated query results by wire event ID with phone-origin sends, retaining source, copy time, availability and per-target outcomes. A peer hint never grants access to another source, and fetched content never overwrites the OS clipboard. Keep text/list data in session memory and images in a temporary encrypted cache. History Off disables queries and cached history; explicit import/send remains usable. Drafts/history content share one budget. Clear removes app drafts/previews/cache; explicitly explain that still-retained remote items may be fetched again on later opening and that system clipboard exports follow their separate lifecycle. Missing bodies disable preview/copy/resend rather than substituting current content. Preserve identity/policy/replay/outgoing receipts separately across restart.

Provide Home, History, Devices and Settings with native Compose navigation and shared assets. History has All / From devices / Sent from this phone and source filters, plus per-source freshness and incomplete/unavailable states. Support system language plus English, Dutch, German and French; TalkBack, large fonts, light/dark themes, long content/names and back navigation. Do not expose misleading automatic-background or autostart switches. In-app hints are sufficient; system notification consent is not a prerequisite. Ask only for required network and specific URI access, never broad storage, contacts, camera or accessibility permission for synchronization.

Design waiting/discovery, fingerprint/consent, draft, per-target transfer, history/local-copy outcomes, unavailable bodies, settings/clear and failure states. Distinguish Listed, Available in app, local copy result and desktop Applied for phone-origin sends; show percentages only when byte progress is real. Permission refusal must leave other features usable and must not trigger repeated prompts.

## Lifecycle and security

Model visibility and focus separately. Losing active state blocks new content actions; background entry stops listeners/connections/network work, discards drafts and hides sensitive previews. Rotation, theme/language changes and Activity recreation must not create a second SDK/runtime. Use a lifecycle owner independent of individual pages. Surviving process memory may retain cached history; force-stop/process restart starts with empty bodies and clears orphan ciphertext. Obscure recent-task previews where possible and verify on devices.

Late callbacks cannot mutate a new session. Resume may rediscover/reconnect/query receipts but never resend old bodies. No FGS, WorkManager or boot receiver keeps synchronization running. Provider invocation is limited to already exported clipboard data.

Protect private identity/policy/cache files, exclude them from cloud backup/logs, and keep history keys only in memory. Uninstall/clear-data creates a new peer identity without inherited outgoing trust. Confirm the foreground Tailscale listener restriction on actual devices.

## Independent tasks

See the [Android test workflow](android-testing.md) for isolated emulator checks,
opt-in real Tailscale tests and external device configuration.

| ID | Acceptance |
| --- | --- |
| A01 | Real Android discovery from Linux without QR; authenticated traffic across networks, split-tunneling, denied policy and resume. |
| A02 | Gradle/SDK bindings, threading/disposal, native clipboard/provider; finalize URI grants, independent readback, export quota and lifecycle. |
| A03 | Four Compose views, settings/history/language/in-app hints; Activity recreation never duplicates the SDK. |
| A04 | Complete device matrix below and real cross-app image paste; emulators are supplemental. |
| A05 | Reproducible APK/AAB with traceable ABI/SDK versions; upgrade preserves identity/policy, signing credentials stay external. Store publication is separate. |
| A06 | Foreground multi-desktop history catch-up, event-ID merge, incremental refresh, source freshness and explicit phone copy. Verify revoked consent, missing bodies and incomplete history. |

Depend on M01-M05, not iOS UI completion. Aim to test a near-stock device and a mainstream vendor device; one OS/version is not evidence for all Android devices.

## Acceptance matrix

| ID | Required result |
| --- | --- |
| AND-01 | Upgraded desktop discovers the foreground app without QR, target <=40 seconds, authenticated identity and outgoing disabled by default. |
| AND-02 | Multiple computers deduplicate by identity with independent directions; a desktop-initiated session also carries mobile sends. |
| AND-03 | Explicit import/send produces matching independent Linux OS readback; launch/resume sends nothing. |
| AND-04 | Queried history metadata/body show the right preview and availability; querying never changes the Android clipboard or creates a desktop delivery receipt. |
| AND-05 | Other apps can paste copied text/images; granted URIs expose no unrelated data and do not cause forwarding. |
| AND-06 | Clearing history removes history ciphertext/previews while active exported images remain pasteable according to their policy; expired/restarted URIs never return a different object. |
| AND-07 | Per-peer direction, content-type and history rules cover every send/resend/query path; granting desktop send can expose older retained local history, as explained in device settings, while re-enabling phone receipt never writes content to the OS clipboard. No global mobile direction controls appear. |
| AND-08 | Default 20, zero, status mode, quota eviction and cleanup failures preserve correct availability. |
| AND-09 | Focus, rotation, background, force-stop and resume cause no background reads/networking, duplicate SDK or old-draft replay. |
| AND-10 | Split-tunneling, ACL, offline and old-version failures report real or unknown causes without fabricated connection/success. |
| AND-11 | Duplicate events, lost receipts and three-node operation preserve query-only recovery, replay protection and no-forwarding. |
| AND-12 | URI denial, corrupt/oversized content and provider timeout fail within bounds without arbitrary path access. |
| AND-13 | Four languages, TalkBack and large layouts work; 20 image transfers and lifecycle loops show no unbounded resource growth. |
| AND-14 | Offline-phone copies from two desktops currently allowed to send merge on opening without earlier delivery attempts; new retained local copies appear incrementally while foreground, with duplicate event IDs collapsed but separate identical copies preserved. |
| AND-15 | Every permitted `peer_hints` computer sends its own snapshot; multi-source hints merge by public-key identity with source/expiry tracking. Connection or permission enable produces a snapshot, peer join/leave updates it, and reconnect refreshes it. Each suggested source is verified directly and returns history only under its own send policy. Fetching/previewing never changes the Android clipboard or relays content; no platform-kind flag drives policy. |

Real-device mobile acceptance remains pending. Keep device/vendor/version/configuration and synthetic reproduction evidence outside the repository. Rust coverage and emulator builds do not substitute for device acceptance.

References: [Android clipboard restrictions](https://developer.android.com/about/versions/10/privacy/changes#clipboard-data), [Tailscale split tunneling](https://tailscale.com/docs/features/client/android-app-split-tunneling).
