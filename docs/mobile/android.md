# Android foreground app specification

Status: proposed; not implemented or device-validated. [Issue #52](https://github.com/KatInBroek/shuttli/issues/52). Prerequisite: [shared SDK](shared-sdk.md); [iOS specification](ios.md) is independent.

## Scope and platform

Use Kotlin/Jetpack Compose with UniFFI bindings to the shared Rust SDK. Proposed minSdk 29 must be confirmed in the build spike; fix target/compile SDK against implementation and distribution requirements. Prioritize phones; tablet/foldable layouts are separate work.

Use the official external Tailscale client, not an embedded VpnService. Support foreground discovery, explicit text/image import/send, inbox preview/local copy, limited history/resend, per-device/global consent and four languages. Do not require a foreground service, background clipboard listener, default IME, accessibility service, root/Shizuku, persistent notification, wake lock or battery-optimization exemption. Share intents, files, LAN, phone-only discovery, FCM, cloud accounts and autostart are outside this first delivery.

## Discovery and explicit sending

An upgraded desktop discovers the phone through Tailscale, probes the application port and authenticates the foreground listener. One connection carries both directions. Deduplicate application identity; Tailscale online state alone does not prove the application is connected. Target first discovery within 40 seconds on a healthy network; refresh cannot force an unknown desktop to scan. Diagnose absent desktops, policy denial, listener failure and VPN split-tunneling where known. Never scan the full address space or modify VPN settings automatically.

Read the clipboard only after an explicit paste action while the Activity is resumed and has window focus. A foreground service does not grant background clipboard access. Do not pre-read content to populate device lists or notifications, and never send on launch/resume.

Import plain text or supported image ClipData MIME/URI through bounded ContentResolver reads. Do not treat a URI as an arbitrary filesystem path. Permission denial, expiry, slow/remote providers, cancellation and timeout require bounded errors. Preflight sizes before decoding and normalize supported images internally to PNG: text <=1 MiB, encoded image <=8 MiB, <=4,194,304 pixels, <=16,384 per edge. Reject sensitive markers, invalid bodies and unsupported representations.

Show a draft snapshot, type, size and allowed targets. Send publishes the displayed snapshot to every currently eligible target; no initial per-send picker. A later clipboard change cannot silently replace a draft. Core policy checks apply even if UI controls are disabled. Explain no target, global/type restrictions, offline and size errors distinctly.

## Inbox and image clipboard exports

Received means validated content admitted to the app inbox, not an OS clipboard write. Show source/time/preview. Explicit local copy requests a fresh core write permit and never automatically forwards. Explicit resend creates another event. Copy result does not change a historical network receipt to Applied.

Android image clipboard sharing requires a native readable URI, not raw image bytes, a private path or an encrypted cache filename. Provide a restricted ContentProvider or equivalent:

- Use unpredictable object IDs, correct MIME and minimal read grants. No directory enumeration or access to identity/cache/other history entries.
- Validate whether to stream decryption or create a separately controlled active export in A02.
- An exported clipboard image has an independent OS-use lifetime. Clearing history must not immediately break an already copied image when the user switches to another app to paste.
- Prefer retaining only the latest export initially, with a fixed quota and explicit expiry/replacement rules. A new copy may replace it; an unrecoverable URI after restart must become unavailable, never resolve to different content.
- A provider started by the OS may serve only explicitly exported content. It must not start discovery, networking, synchronization or clipboard reads.
- Finalize cleanup after clipboard replacement, open-reader handling and conservative bounds when callbacks are unavailable. Image-copy delivery is blocked until this lifecycle is defined and validated.

Readback must use an independent clipboard/provider path and verify readable bytes, not compare a buffer to itself. Text and image copy require separate cross-app tests.

## Policy, history and UI

Defaults: global send/receive on, new-device send off/receive on, manual sending. Apply per-peer block, direction, text/image, tighter retention and quiet rules. Offer full-fingerprint comparison; discovery cannot grant outgoing consent. Persistence failure must report actual state and fail closed.

History defaults to 20 events, configurable 0-10,000 subject to quotas; Off/Status/Content modes. Keep text/list data in session memory and images in a temporary encrypted cache. With history off, one bounded current inbox item remains usable. Active/history content shares one budget. Clear removes app inbox/drafts/previews/cache; explicitly explain that system clipboard exports follow their separate lifecycle. Missing bodies disable preview/copy/resend rather than substituting current content. Preserve identity/policy/replay/receipts separately across restart.

Provide Send/Receive, Devices, History and Settings with native Compose navigation and shared assets. Support system language plus English, Dutch, German and French; TalkBack, large fonts, light/dark themes, long content/names and back navigation. Do not expose misleading automatic-background or autostart switches. In-app hints are sufficient; system notification consent is not a prerequisite. Ask only for required network and specific URI access, never broad storage, contacts, camera or accessibility permission for synchronization.

Design waiting/discovery, fingerprint/consent, draft, per-target transfer, inbox/copy outcomes, history/unavailable bodies, settings/clear and failure states. Distinguish Received from Applied; show percentages only when byte progress is real. Permission refusal must leave other features usable and must not trigger repeated prompts.

## Lifecycle and security

Model visibility and focus separately. Losing active state blocks new content actions; background entry stops listeners/connections/network work, discards drafts and hides sensitive previews. Rotation, theme/language changes and Activity recreation must not create a second SDK/runtime. Use a lifecycle owner independent of individual pages. Surviving process memory may retain history/inbox; force-stop/process restart starts with empty bodies and clears orphan ciphertext. Obscure recent-task previews where possible and verify on devices.

Late callbacks cannot mutate a new session. Resume may rediscover/reconnect/query receipts but never resend old bodies. No FGS, WorkManager or boot receiver keeps synchronization running. Provider invocation is limited to already exported clipboard data.

Protect private identity/policy/cache files, exclude them from cloud backup/logs, and keep history keys only in memory. Uninstall/clear-data creates a new peer identity without inherited outgoing trust. Confirm the foreground Tailscale listener restriction on actual devices.

## Independent tasks

| ID | Acceptance |
| --- | --- |
| A01 | Real Android discovery from Linux without QR; authenticated traffic across networks, split-tunneling, denied policy and resume. |
| A02 | Gradle/SDK bindings, threading/disposal, native clipboard/provider; finalize URI grants, independent readback, export quota and lifecycle. |
| A03 | Four Compose views, settings/history/language/in-app hints; Activity recreation never duplicates the SDK. |
| A04 | Complete device matrix below and real cross-app image paste; emulators are supplemental. |
| A05 | Reproducible APK/AAB with traceable ABI/SDK versions; upgrade preserves identity/policy, signing credentials stay external. Store publication is separate. |

Depend on M01-M05, not iOS UI completion. Aim to test a near-stock device and a mainstream vendor device; one OS/version is not evidence for all Android devices.

## Acceptance matrix

| ID | Required result |
| --- | --- |
| AND-01 | Upgraded desktop discovers the foreground app without QR, target <=40 seconds, authenticated identity and outgoing disabled by default. |
| AND-02 | Multiple computers deduplicate by identity with independent directions; a desktop-initiated session also carries mobile sends. |
| AND-03 | Explicit import/send produces matching independent Linux OS readback; launch/resume sends nothing. |
| AND-04 | Inbox preview is correct, OS clipboard is unchanged and the sender sees Received rather than Applied. |
| AND-05 | Other apps can paste copied text/images; granted URIs expose no unrelated data and do not cause forwarding. |
| AND-06 | Clearing history removes history ciphertext/previews while active exported images remain pasteable according to their policy; expired/restarted URIs never return a different object. |
| AND-07 | Global/per-peer/type/block rules cover every send/resend/receive path; enabling never replays content. |
| AND-08 | Default 20, zero, status mode, quota eviction and cleanup failures preserve correct availability. |
| AND-09 | Focus, rotation, background, force-stop and resume cause no background reads/networking, duplicate SDK or old-draft replay. |
| AND-10 | Split-tunneling, ACL, offline and old-version failures report real or unknown causes without fabricated connection/success. |
| AND-11 | Duplicate events, lost receipts and three-node operation preserve query-only recovery, replay protection and no-forwarding. |
| AND-12 | URI denial, corrupt/oversized content and provider timeout fail within bounds without arbitrary path access. |
| AND-13 | Four languages, TalkBack and large layouts work; 20 image transfers and lifecycle loops show no unbounded resource growth. |

All mobile acceptance remains pending. Keep device/vendor/version/configuration and synthetic reproduction evidence outside the repository. Rust coverage and emulator builds do not substitute for device acceptance.

References: [Android clipboard restrictions](https://developer.android.com/about/versions/10/privacy/changes#clipboard-data), [Tailscale split tunneling](https://tailscale.com/docs/features/client/android-app-split-tunneling).
