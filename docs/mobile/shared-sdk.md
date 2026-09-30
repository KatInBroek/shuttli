# Shared mobile SDK and desktop-assisted discovery

Status: implementation in progress; protocol, transport and device acceptance remain pending. The [mobile feature specification](feature-spec.md) owns product requirements. Track [shared SDK #50](https://github.com/KatInBroek/shuttli/issues/50), [iOS #51](https://github.com/KatInBroek/shuttli/issues/51) and [Android #52](https://github.com/KatInBroek/shuttli/issues/52).

## Scope

The first mobile apps work while open: explicitly import and send the current clipboard, query currently permitted recent retained history from each source, preview it, and copy a selected item locally on request. Use the separately installed official Tailscale client. No QR code, manual address, application account, central clipboard server or Tailscale administrative token is required.

A desktop discovers the phone through its Tailscale peer list and connects to the foreground app. Each desktop is an ordinary peer; none is a coordinator or trust broker. A fetched item does not automatically replace the phone's system clipboard. Mobile sending is manual; launching, resuming, discovering a device or enabling a direction never sends the phone's existing clipboard content. As phones are often offline, a foreground connection queries bounded recent local copies still held in each desktop's session history when current permissions allow. Online permitted peers exchange live content using the original protocol on every platform. History queries additionally recover retained copies after absence.

Background clipboard monitoring, lock-screen delivery, push notifications, share extensions, shortcuts, files, independent LAN discovery and phone-only initial discovery are outside this first mobile delivery.

## Shared code and boundaries

Shared means the same Rust source compiled for each target through Cargo path dependencies. It does not mean copies translated into Swift/Kotlin or remote execution on a desktop. Each phone has its own identity, policy, state and core instance.

| Component | Reuse and required changes |
| --- | --- |
| model, core | Same source; separate permission to fetch a history body from permission to write the OS clipboard. Gate history export by authenticated identity and current send/type/history policy; preserve desktop invariants without an offline eligibility ledger. |
| application, ports, api | Same use cases and contracts; add explicit import, draft, local copy, bounded history catch-up, negotiated capabilities and foreground lifecycle. |
| runtime | Bounded serial scheduling with explicit start, pause and stop. |
| protocol | Extract shared version dispatch, framing and receipt contracts. |
| adapters-common | Extract TLS, normalization, identity, encryption and storage from desktop-specific adapters. |
| desktop adapters | Keep Tailscale enumeration, X11, macOS helper, autostart and desktop notifications here. |
| mobile-sdk | Composition root assembling application/runtime/common adapters and injected native services. |
| mobile-ffi | Generated UniFFI boundary and Swift/Kotlin bindings. |
| native apps | Independent SwiftUI and Compose UIs, clipboard/lifecycle adapters, packaging and real-device tests. |

The transport shares framing, payload validation, permission rechecks and OFFER/READY/body/APPLIED mechanics through `crates/transport`. Its sender and receiver do not distinguish device kinds. A native reception adapter commits content before APPLIED: the desktop writes/readbacks its clipboard and keeps its existing durable receipt; the mobile app verifies and accepts a bounded history-cache item. The phone OS clipboard is touched only by explicit Copy. A mobile live receipt is session-scoped: clear/eviction/restart may make a subsequent STATUS query Unknown. HISTORY_GET remains a separate request that caches content without creating a live receipt or publishing an event.

Proposed additions:

```text
crates/protocol/
crates/adapters-common/
crates/mobile-sdk/
crates/mobile-ffi/
apps/ios/                 # Xcode project, SwiftUI, platform adapters, tests
apps/android/             # Gradle project, Compose, platform adapters, tests
tools/mobile/             # Reproducible builds and device test drivers
```

Keep the existing workspace and desktop entry points. Register new dependency roles in `tools/architecture.json`. The SDK is a host/composition layer, not a UI layer. Protocol depends on value types and codecs; common adapters cannot import desktop subprocesses, X11, GTK or autostart. Native pages call the SDK API and never manipulate databases, encrypted objects or peer sessions directly.

## FFI, threads and artifacts

Use UniFFI for generated Swift/Kotlin bindings. Target iOS arm64 and required simulator architectures through Xcode; Android arm64 and x86_64 simulator builds through the NDK. Target availability does not prove that every dependency or platform service is portable.

- Run the SDK in-process, without a desktop daemon or local socket requirement.
- Use one bounded scheduler and bounded/coalescing subscriptions. Never duplicate a large image for every UI observer.
- Run network/storage work off the UI thread. Dispatch native clipboard work to MainActor/the Android main thread and return asynchronous results without holding a Rust lock while waiting.
- Do not export constructors or serialization for Publication, Reception or WriteAuthorization. Native adapters receive one-time operation IDs and approved actions; late or cancelled completions cannot affect another session.
- Pass bounded bytes or handles instead of large JSON/base64 copies. Validate imported content and native results again in Rust.
- Test cancellation, release, callback cycles, stale handles and panic containment at the FFI boundary.
- Package an XCFramework for iOS and ABI-specific libraries plus bindings/AAR for Android. Generate artifacts from one commit and lockfile; record SDK API, wire version, architecture and checksums.
- Keep generated bindings/artifacts in build output. Desktop builds must not require mobile SDKs.
- The workspace forbids unsafe code. If generated FFI/JNI needs an exception, confine and review it in mobile-ffi; do not relax core/application or place handwritten business logic in that exception.

## Discovery without pairing codes

1. The official Tailscale client is connected and the mobile app is active with a bounded listener.
2. A desktop probes only the application port, currently TCP 45987, on candidates from its allowed Tailscale discovery scope.
3. Mutual TLS proves application identity; the application handshake exchanges version, name and capabilities.
4. Both devices show the authenticated peer. The established connection carries both directions regardless of its initiator.
5. Every directly connected peer that advertises `peer_hints` and is allowed by the sender's effective send policy receives that sender's bounded snapshot of currently reachable, directly authenticated Shuttli peers. A newly discovered phone has no computer-to-phone send permission by default, so it sees the directly connected computer after authentication but receives no snapshot or history until that computer's user enables sending. Include only identity, Tailscale endpoint, protocol capability and freshness needed to connect; do not expose the raw tailnet inventory or clipboard metadata in the peer list.
6. The phone merges independent snapshots from every eligible connected source by public-key identity, tracks every hint's source and expiry, and contacts each candidate directly or reuses an existing connection. Each target's own authenticated session and outgoing permission decide whether its roster or history can be returned. A desktop never re-advertises a peer learned only from a hint; no peer propagates another device's permission or clipboard content. Revoking one source stops its updates but does not suppress another permitted source's snapshot.

The first-run prerequisite is at least one upgraded desktop. With the separately installed official Tailscale client, the mobile app cannot enumerate that app's private peer list; an authenticated desktop supplies the first connection and permitted roster hints. A roster is only the sender's current authenticated Shuttli view, never a complete tailnet directory. Tailnet policy must allow a usable application connection between the phone and each target; peer hints cannot bypass network policy. A phone refresh can retry its own listener or known connections; it cannot force an unknown desktop to scan. With the current approximately 30-second desktop refresh, target discovery within 40 seconds on a healthy network, subject to real-device validation. An already connected desktop can advertise newly authenticated peers promptly, without waiting for the phone to enumerate Tailscale itself. When its roster snapshot is stale at mobile connection time, it may trigger one rate-limited run of its existing Tailscale discovery refresh.

Do not access another app's private LocalAPI/container, scan the whole address space, require root/jailbreak, or accept every public/LAN interface merely to make the listener reachable. Verify Tailscale interface binding, IPv4/IPv6, cellular routing, VPN changes and permissions on devices. Address filtering never replaces TLS identity and application policy. Tailscale connectivity without external network access is not guaranteed; standalone LAN support is separate.

Reuse the existing bounded TLS transport, identity, session selection, framing and message loop. Negotiate `peer_hints`, `history_pull` and `accept_live_offer` as independent v2 operation capabilities; never add a `mobile` or OS-kind permission branch. After an authenticated session with `peer_hints` is selected and application policy is loaded, send one `PEER_LIST(revision, peers)` snapshot if effective sending to that peer is on. If permission becomes enabled during the session, send it then. Send coalesced replacement snapshots, including an empty one, when directly authenticated online peers join/leave or their verified endpoint/capability changes; resend on reconnect. Include only other peers from the sender's current authenticated sessions, never the sender, recipient, unverified tailnet entries or hint-only candidates. Recheck permission immediately before each write. Each device's receive permission controls its history requests; discovery remains usable. A disabled sender cannot retract a list already delivered, so the phone expires unverified hints after a short lifetime; directly authenticated peers keep their own identity and policy. No separate periodic peer-list poll is needed. Reject duplicates, self entries, non-Tailscale addresses, oversized lists and stale endpoints. The phone verifies the actual TLS peer identity before showing or querying any hinted device. Hints never authorize outgoing sending on the phone or on another desktop.

Capability meaning is per operation: `peer_hints` permits receiving roster snapshots, `history_pull` supports authenticated history requests/responses, and `accept_live_offer` permits unsolicited clipboard `OFFER` delivery. A peer can advertise any valid combination. The session initiator, operating system and form factor do not determine policy. The foreground phone advertises live-offer acceptance as well as history and peer-hint operations. Native lifecycle and per-device policy determine which operations are currently allowed; device type does not. Both peers still apply their own per-device direction settings to each operation.

Roster eligibility uses the sender's effective outgoing permission and the recipient's `peer_hints` capability. The phone's receive-from-source switch controls its history requests, not whether it can learn discovery hints. Peer snapshots are sent on connection or permission enablement, updated after direct authenticated-peer changes, and resent on reconnect; there is no elected roster owner or periodic full-network broadcast.

## Import, history fetch and local copy

`ImportContent -> bounded draft -> SendDraft` describes the intended use case; final names belong to the SDK contract. Importing never sends. Explicit send publishes the displayed immutable snapshot to currently allowed targets; a later clipboard change cannot silently replace that draft. Reject recognizable sensitive/transient content. Missing sensitive markers do not imply password detection.

| Operation | Completion meaning |
| --- | --- |
| Desktop clipboard | Applied only after conflict checks, OS write, independent OS readback and durable receipt. |
| Mobile live reception | Validated content committed to the bounded app cache, followed by APPLIED; no OS clipboard write or durable offline delivery promise. |
| History GET | Requested content was validated and cached in the requesting app; no delivery receipt or OS clipboard write is implied. |

Copying a fetched item is a separate local operation with a fresh core write permit and current platform checks. It never automatically broadcasts on the phone. Explicit resend of a phone-origin item creates a new event and rechecks permissions. Local copy success/failure/unverified state does not turn a history query into an Applied receipt. A platform API accepting bytes is not independent readback; report uncertainty honestly.

## Foreground history catch-up contract

Catch-up requires new Wire v2 requests; the existing desktop localhost history API is not a peer history protocol. A source desktop may export only its own retained local-origin events. Existing desktop history records local copies even when sending is disabled or the phone is unknown/offline. Do not add a per-phone capture-time eligibility marker or offline authorization ledger. An explicit manual send remains a separate new event. Never relay another device's original received event as the source's own history.

At both list and content-fetch time, the source rechecks the authenticated requester identity, current effective outgoing permission, content-type policy, source origin and history mode. The phone checks its per-device incoming permission before either request; its UI has no global direction switch. A denied or revoked request yields no content. Once sending to the phone is allowed, it can request all currently retained local-origin copies permitted by the current type/history settings, including copies made before discovery/consent or while automatic sending was off. The automatic switch controls live publication, not an authorized history request. Explain this scope beside the source's device-send permission. Desktop history Off yields no catch-up records, and Status mode cannot supply bodies. No new archive is created.

Define bounded `HISTORY_LIST(cursor, limit)` and `HISTORY_GET(event_id)` operations on each source's authenticated session, with per-source pagination and byte caps. The list returns metadata and availability, not clipboard bodies; `GET` must recheck policy and object identity immediately before sending. Status-only history cannot provide bodies. Never infer or fabricate a delivery receipt from a list/get response: show **Listed**, **Available in app** after verified body caching, then a separate local copy result only after the user's action. A later history list may report source-side expiry or status-only retention; that cannot recall content separately accepted by the live reception adapter. Local retention/clear still governs that cached item. Repeated results for the same wire event ID merge into one row; content equality is not identity. Keep source, copy time, acquisition path, last refresh, availability and per-target outcomes separate. Show clock-skew uncertainty instead of promising exact order between devices.

Fetch metadata from each connected desktop on foreground entry and explicit refresh; fetch content within the phone's existing RAM/encrypted temporary-image quotas, on demand where necessary. While the phone stays open, a desktop sends a lightweight `HISTORY_CHANGED(revision)` hint when the retained window visible under that phone's current policy changes; a policy change that enables access also triggers a fresh list. Coalesce hints so rapid copies do not create unbounded work. The phone then fetches the changed bounded window and only new/needed bodies. A per-source cursor/revision avoids repeated full body transfers; after reconnect, missed revision, source epoch change or invalid cursor, reconcile that source's current bounded window. Do not rely on a notification as proof that content was received. Cancel on background entry and discard stale callbacks. An offline desktop, source restart, cleared/disabled history, retention eviction or network error can create a gap; show per-source stale/partial state. A successful list call proves only the current retained window was queried. Do not keep a background listener, add a central server or persist desktop text history to promise complete recovery.

Both apps use this contract through the same SDK; each platform validates its own foreground history flow. Mobile UI and clipboard adapters remain platform-specific.

## History and active content

Keep a separately bounded phone draft and bounded cached history items. Replacing or evicting an item must not make an already open detail view act on different content. History Off disables history queries and caching; explicit phone import/send remains usable.

History defaults to 20 merged events, configurable from 0 to 10,000 subject to shared metadata, RAM and image quotas. Text/list data stays in the SDK process session; images use a temporary encrypted cache and a session-only key. Cached content and history share the same object budget. Per-peer retention can only tighten the history mode. History Off disables catch-up.

Clearing history also clears app drafts/previews and cancels references before deletion. It does not clear system clipboards or recall remote deliveries. Hide previews on temporary inactivity; clear drafts on background entry. A surviving process may retain cached history in RAM, but process restart restores no content and clears orphan ciphertext. Identity, per-device policy, replay protection and necessary outgoing receipts are persisted separately. History queries never act as clipboard delivery receipts.

Android clipboard image exports have an independent bounded OS-use lifetime, described in the Android spec. Exclude private identities, policy ledgers and cache objects from cloud backup and logs. History does not require Keychain; device identity protection is a separate platform decision.

## Protocol and API compatibility

Wire v1 retains its original live messages and Poll/Idle ordering and works with desktop or mobile reception adapters. The current extension negotiates HELLO version 3 for live plus history operations; version 2 remains the earlier pull-only extension. The shared extension frame codec retains history operations and adds the original Poll/Idle/STATUS/Receipt operations. These versions describe protocol support, never device kind:

- In current live-plus-history sessions, public-key identity order assigns request turns. Only one operation owns the byte stream at a time; Poll/Idle grants the other peer a turn. History requests yield between bodies, and change notices cannot replace READY or APPLIED. Session initiation and OS do not assign the turn.
- A newer connector tries the extension first. A fresh v1 retry is allowed only after authenticated HELLO incompatibility/closure, with the same certificate identity checked again. TLS or identity failures never trigger downgrade.

- All clients retain v1 live interoperability. History and peer hints additionally require negotiated extension support.
- Dispatch by version before strict parsing. Reject invalid lengths, fields and identity bindings.
- Existing desktop-to-desktop fallback may use a fresh v1 connection only after an explicit version-incompatibility result, never after TLS/identity failure. A v1-only connection remains usable for live delivery and does not pretend to support history catch-up.
- Version local IPC and FFI separately. Unknown receipt states cannot default to success.
- Existing desktop OS confirmation/durable receipt requirements and bounded sender-scoped STATUS queries remain unchanged; history queries never create a delivery receipt or reapply a clipboard.
- Negotiate history-list/get, peer-list, history-change and live-offer acceptance independently. A peer without `accept_live_offer` receives no unsolicited OFFER, and one without history capability cannot be queried. Unknown frames and older peers retain versioned behavior; a hint cannot create permission or bypass identity verification.

## Independent tasks and gates

| ID | Deliverable and independent acceptance |
| --- | --- |
| M01 | Linux discovers real iOS and Android devices; authenticated bidirectional synthetic traffic, peer-list snapshot on connect/permission enable, coalesced join/leave updates, revocation, direct peer verification, cellular/resume and denied-policy cases. Verify actual listener routing. |
| M02 | Minimal Swift/Kotlin calls into the same Rust code; thread/cancel/release tests, fixed toolchains/ABIs and review of generated unsafe boundaries. Public bindings cannot expose internal permits. |
| M03 | Shared explicit import, local copy and live-plus-history contracts across core, application, ports, API and Wire v2; capability routing, permissions, replay, no-forwarding and old-desktop behavior. Every core coverage metric remains above 95%. |
| M04 | Extract common adapters without changing desktop behavior, then assemble a mobile SDK with explicit capabilities and lifecycle. No desktop dependency leaks into mobile. |
| M05 | Platform-neutral v1 regression with mobile and desktop receivers, phone probing, live plus history arbitration, explicit pull-only capability behavior, disconnect results and policy handling, testable without mobile UI. |
| M06 | Traceable desktop/SDK/wire/mobile version matrix and independent release evidence for each platform. |
| M07 | Authenticated history list/get and change hints over existing retained local history with current-policy rechecks; offline-phone catch-up, newly granted access to older retained copies, live increments, multi-source merge, no-forwarding, revoked consent, eviction and partial-history tests. |

M01/M02 resolve feasibility first; M03-M05 establish shared behavior. iOS and Android UI releases proceed independently after their shared prerequisites. Use one repository/workspace; do not create empty app projects as evidence of implementation.

Use the same core test suite, with function/line/region coverage each strictly above 95%. Native suites cover lifecycle, clipboard, bindings and UI. Simulators cannot replace real Tailscale VPN routing, permissions or cross-app image paste. Minimum device E2E: Linux plus iPhone, and Linux plus Android; also exercise three-node no-forwarding, lost receipts, cleanup and restart. Use synthetic content and keep machine-specific evidence outside the repository.

Measure idle usage, 20 image transfers and repeated foreground/background cycles before fixing mobile memory budgets. Unbounded growth, duplicate SDKs or background scanning block release. No clipboard polling while idle; no background service or wake lock to keep synchronization alive. Share the existing brand and English/Dutch/German/French semantic catalogs, with native accessibility/plural handling.

References: [UniFFI](https://mozilla.github.io/uniffi-rs/latest/), [Rust iOS targets](https://doc.rust-lang.org/rustc/platform-support/apple-ios.html), [Rust Android targets](https://doc.rust-lang.org/rustc/platform-support/android.html).
