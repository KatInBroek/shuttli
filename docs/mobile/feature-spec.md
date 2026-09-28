# Mobile feature specification

Status: proposed. No iOS or Android app, mobile SDK, or mobile history protocol is implemented or device-validated yet. This is the product requirement source for [shared SDK #50](https://github.com/KatInBroek/shuttli/issues/50), [iOS #51](https://github.com/KatInBroek/shuttli/issues/51), and [Android #52](https://github.com/KatInBroek/shuttli/issues/52). The [shared SDK](shared-sdk.md), [iOS](ios.md), and [Android](android.md) specifications define technical and platform acceptance details.

## Goal and release scope

Let a person use Shuttli briefly on an iPhone or Android phone to retrieve recent retained copies from their computers, inspect them in one history, and explicitly copy a chosen item into the phone's clipboard. The person can also explicitly import the phone's current clipboard content and send that fixed snapshot to allowed computers. The app works only while foregrounded, using the separately installed official Tailscale client and the existing peer-to-peer Shuttli trust model. It requires no account, central clipboard server, QR code, or manual address entry.

The first release covers text and supported images. It includes one shared Rust SDK, independent native iOS and Android apps, and the desktop protocol changes needed for both. Each platform has its own release and real-device acceptance gate.

## User journeys

### First use and device discovery

1. The user connects Tailscale and opens the phone app. An upgraded computer discovers the foreground phone and establishes the existing mutually authenticated Shuttli session.
2. Both devices show the authenticated identity. Receiving is on by default; sending to a newly seen device is off. The user allows each computer to send to the phone using the existing per-device control. The phone can independently allow or deny sending to each computer.
3. A computer whose global and per-device sending to the phone are both enabled sends a bounded list of other Shuttli devices it has authenticated. The list supplies connection candidates, not permission. The phone connects to each candidate directly or reuses an existing session. Every target verifies the phone's identity and checks its own permissions before returning history.
4. The phone deduplicates device identities across lists and sessions. A changed address/name retains the device's policy; a changed identity starts as a new device. Unreachable candidates remain visible only with an honest connection state.

### Retrieve computer history after the phone was closed

1. Each computer continues retaining bounded local-copy history under its existing history mode and quotas, whether the phone was online, known, or allowed when the copy occurred. No per-phone offline eligibility marker is recorded. Explicit manual sends remain separate new events.
2. On each foreground connection, the phone asks that computer for a bounded recent history list. The computer checks the phone's authenticated identity and **current** global/per-device send, content-type, and history policy, then returns its own retained local-origin events. It never exports another peer's original received event; a deliberate manual resend is a new local-origin event.
3. The phone merges lists from all reachable computers with live receipts and its own sent events. It retrieves bounded bodies for the visible/useful window and fetches other available bodies on demand. Fetching, opening, and previewing do not read or write the phone's system clipboard.
4. While the app stays open, authorized computers notify it of relevant history changes. The phone fetches incremental changes and updates the same list. Reconnection or a missed change triggers a bounded reconciliation. Closing or backgrounding the app stops this work; reopening starts catch-up again.
5. The user opens an item, previews it, and taps **Copy to this phone**. Only this action writes to the phone clipboard. The user then pastes into another app.

### Send from the phone

1. The user copies text or an image in another app, returns to Shuttli, and deliberately invokes the platform paste/import control.
2. Shuttli creates a bounded immutable draft. It shows content preview, type/size, and the currently allowed destination names. A later clipboard change does not alter the draft.
3. The user taps **Send to N devices**. The SDK checks current global/per-device/type policy for each target and reports separate sending, success, failure, cancelled, or unknown outcomes. The draft is discarded on background entry; returning does not resend it.

## Product requirements

| ID | Requirement |
| --- | --- |
| MOB-01 | Reuse existing TLS peer identity and per-device/global send/receive controls. New devices default to outgoing off and incoming on; discovering or advertising a peer grants no permission. |
| MOB-02 | A computer may share bounded, short-lived identity/endpoint/capability hints with a phone it is effectively allowed to send to. Do not expose the raw Tailscale inventory or clipboard data in the device list. Verify each suggested peer directly. |
| MOB-03 | Foreground catch-up from every independently authorized computer is automatic; the phone never depends on one computer relaying another computer's clipboard history. |
| MOB-04 | History list and body export require current effective send, content-type, and history permission for the authenticated phone; the phone checks its current receive permission before queries. History Off, revocation, eviction, or a missing source body must not leak or substitute content. No capture-time, offline, or per-event recipient eligibility ledger is needed. |
| MOB-05 | Merge by stable wire event ID, not by content. A live receipt and later history result for the same event form one row; separate copies with identical content remain separate. |
| MOB-06 | While open, use bounded change hints and incremental requests. After missed updates, reconcile the retained window. Stop network/content work in the background and never monitor the phone clipboard there. |
| MOB-07 | Show one newest-first history with source device, copy time, text/image type, availability, freshness, and truthful transfer state. Filters: All, From devices, Sent from this phone, and source device. Group each phone-origin send with its per-target outcomes. |
| MOB-08 | Distinguish a fetched item available in the app, a live item Received into the app, a desktop item Applied to its OS clipboard, and an item explicitly copied to the phone OS clipboard. Neither a history query nor a platform write request alone proves OS application. |
| MOB-09 | Explicit import creates a frozen draft; a second explicit action sends it. Explicit local copy is required for every remote item. No app open, resume, discovery, history fetch, preview, or receive action sends the phone clipboard or overwrites it. |
| MOB-10 | Defaults: manual phone sending, global send/receive on, new-peer outgoing off/incoming on, recent history limit 20. Support global/per-device direction, content type, history Off/Status/Content, retention, and clear controls. |
| MOB-11 | Text/history bodies stay in process memory; image cache objects are encrypted with a session-only key and cleaned under bounded quotas. History Off retains only the bounded current live inbox item. Clear local history does not recall remote deliveries or clear OS clipboards; remote copies may be found again on a later catch-up while still retained and authorized. Explain this in the clear action. |
| MOB-12 | Show Updating, last checked time per source, unavailable devices, incomplete history, expired bodies, permission denial, and unknown delivery honestly. Do not describe a current retained window as proof that every offline-period copy was recovered. |
| MOB-13 | Use Home, History, item detail, Devices, and Settings flows with native navigation, the shared brand, English/Dutch/German/French, accessibility, and light/dark layouts. Content never appears in system notifications. |
| MOB-14 | Bound peers, message sizes, pagination, body transfers, cache, work queues, and foreground resource use. Reuse the existing authenticated transport and version its new device-list, history-list/get, and change messages. |

## Platform-specific requirements

| Platform | Native responsibility | Release gate |
| --- | --- | --- |
| iOS | SwiftUI with the shared Rust SDK; native system paste control; foreground listener and lifecycle; independent clipboard copy/readback checks. | Real iPhone discovery, Tailscale routing, text/image cross-app paste, resume and bounded history tests in [iOS acceptance](ios.md). |
| Android | Jetpack Compose with the same Rust SDK; explicit foreground clipboard import; bounded ContentResolver reads; restricted image clipboard URI export whose OS-use lifetime is separate from history cache. | Real Android discovery, clipboard/provider security, cross-app text/image paste, rotation/process lifecycle and bounded history tests in [Android acceptance](android.md). |

## Privacy, reliability, and limits

- Connections remain end-to-end between devices on the authenticated Shuttli transport. A device list is a discovery hint only. Each source decides for itself whether the phone may see an event; a permitted source cannot grant access to another source.
- Enabling or re-enabling sending to a phone also permits that phone to request **all currently retained local-origin history** allowed by the present type/history settings, including copies made before permission was granted or while automatic sending was off. The device-permission UI must state this clearly. Automatic sending controls live publication of new copies; it does not restrict an authorized history request. Current revocation blocks new list/body requests. Re-enabling phone receiving permits the next foreground catch-up; no query writes the phone clipboard. Content already delivered to a phone cannot be recalled.
- The existing desktop session-history limits apply. A desktop restart, history Off, retention eviction, disconnection, or a phone remaining offline longer than retention may leave gaps. No server or durable text-history archive is added.
- The app makes no claim of automatic phone clipboard sync, background reception, or guaranteed delivery while closed. A requested phone copy is checked through the platform adapter and reported as verified or uncertain according to real evidence.
- Exclude file transfer, standalone LAN discovery, background services, push delivery, share extensions, shortcuts, embedded VPN, phone-only initial discovery, and store publication from this first release.

## Kernel and lower-layer change map

The desktop baseline already has `EventId`, per-device/global policy, mutual TLS, a bidirectional session on the application port, local-copy history, an encrypted temporary image cache, and bounded receipt recovery. Its current network frame is strict v1, its history API is local to the daemon, and successful reception means an OS clipboard write. The mobile feature needs the following explicit changes; none is provided by native UI alone. Current code anchors: [model](../../crates/model/src/sync.rs), [core](../../crates/core/src/sync.rs), [application](../../crates/application/src/service.rs), [ports](../../crates/ports/src/sync.rs), [history cache](../../crates/adapters/src/recent.rs), [durable store](../../crates/adapters/src/storage.rs), [network](../../crates/adapters/src/network.rs), [discovery](../../crates/adapters/src/discovery.rs), and [local API](../../crates/api/src/control.rs).

| Layer | Reuse | Required addition or change | Requirement |
| --- | --- | --- | --- |
| `model` | `DeviceId`, `EventId`, content metadata, peer policy, history modes. | Add typed peer hints, per-source history metadata/cursor and source freshness, and a distinct `Received` receipt state. Keep **Available in app** as local availability, separate from a network receipt or OS-copy result. Version serialized state. | MOB-02, MOB-05, MOB-07, MOB-08 |
| `core` | Existing publication and write permits, policy revision, no-forwarding rules. | Make an inbox-admission decision independent of desktop clipboard baseline/write authorization. Add a history-export decision using only the authenticated requester, current global/per-peer send and type policy, source origin and current history mode; no copy-time recipient snapshot or offline eligibility permit. Authorize peer-list disclosure separately. Explicit local copy needs a fresh policy-aware write permit and must not produce an outgoing publication. | MOB-01–06, MOB-08–09 |
| `application` and `ports` | Serial core decisions, pluggable network/store/clipboard ports and configuration generation. | Add frozen import draft, SendDraft, inbox admission, CopyToPhone, per-source catch-up and merge use cases. Add typed network events and store queries for event-ID history and revisions. Recheck authority after any async store/network yield, especially between history list and body transfer. Use event ID to keep live receipt and history fetch in one row. | MOB-03–09 |
| `recent` history and durable store | RAM-only text, encrypted session image objects, retention quotas, persistent identity/policy/replay/receipts. | Index retained local events by `EventId` rather than exposing local numeric history IDs on the wire. Return metadata pages and exact retained bodies only after current policy checks; advance a bounded source revision on local history changes, clear and eviction. No per-target eligibility metadata or new history archive is stored. Persist only the minimal `Received` receipt/replay evidence needed for recovery; never persist text history. | MOB-03–05, MOB-10–12 |
| wire and network adapter | TLS 1.3 identity, one application port, bounded frames, session selection, Poll/Idle loop and payload integrity checks. | Negotiate v2 capabilities before parsing new messages. Add bounded `PEER_LIST`, `HISTORY_LIST`, `HISTORY_GET`, `HISTORY_CHANGED` and `Received` response handling on the existing session. Schedule/coalesce change hints through the existing message loop; page metadata under frame limits and transfer bodies under existing content bounds. Keep v1 desktop interoperation and reject unknown/unsupported mobile capabilities clearly. | MOB-02–06, MOB-08, MOB-14 |
| discovery adapter | Desktop Tailscale peer enumeration and authenticated Shuttli sessions. | Export only fresh, authenticated Shuttli peer hints to a recipient currently allowed by global/per-device send; cap count/age and reject self, duplicate or non-Tailscale endpoints. The phone may connect to a hinted endpoint, but validates that target's TLS identity before storing it or asking for history. A hint never imports another peer's consent. | MOB-01–03 |
| mobile SDK and FFI | Shared Rust source and the existing application/core boundaries. | Compile one mobile SDK instance per app; bind typed, bounded commands and subscriptions through UniFFI. Inject platform clipboard, lifecycle, storage and Tailscale route adapters. Cancel foreground tasks and stale callbacks on background/lock; keep image bytes out of repeated UI JSON/base64 snapshots. | MOB-06–14 |
| native adapters and UI | Shared brand and semantic language catalogs. | iOS uses native paste and clipboard APIs; Android uses focused clipboard import and a restricted image-export provider. Both render merged history and truthful per-source/transfer states through the SDK API only. | MOB-07–13 |

Implementation order: first prove real-device Tailscale routing and compile the shared SDK; then land v2 negotiation/receipt and inbox semantics; then authenticated current-policy history queries over existing retained local events; then peer hints and foreground incremental updates; finally complete the native flows and independent device tests. Keep the v1 desktop path working at each stage.

Lower-layer gates before UI acceptance: core tests prove current permission denies an unknown/disabled phone and grants retained local history after explicit allowance, regardless of the copy-time permission or automatic-send state; authorization tests revoke list and body access even between metadata and `GET`; storage tests prove event-ID lookup, retention/clear, restart gaps, and durable `Received` recovery without durable text; wire tests reject forged peer hints, wrong identities, stale cursors, oversized pages, and v1/v2 confusion; three-device tests prove direct-source history and no automatic forwarding. Core production function, line and region coverage must each remain above 95%.

## Acceptance scenarios and issue ownership

| Scenario | Required observation | Owner |
| --- | --- | --- |
| MOB-T01 | Computer A advertises B only after A may send to the phone; the phone verifies B directly, and B returns history only when B itself allows sending to that phone. Forged/stale hints grant no access. | #50, #51, #52 |
| MOB-T02 | A and B make copies while the phone is closed, including before it was authorized or while automatic sending was off. After each source currently allows sending, the phone shows both sources' retained copies in one history; matching live/history events appear once and identical separate copies remain separate. | #50, #51, #52 |
| MOB-T03 | During an open session, new retained local copies update the list incrementally for an allowed phone; rapid hints coalesce, reconnect reconciles missed changes, and background entry stops traffic. | #50, #51, #52 |
| MOB-T04 | Current permission denial, revocation, history-Off, eviction, and offline-source cases never expose bodies while denied or claim a complete history. Granting permission can reveal earlier retained copies, as stated in device settings. | #50, #51, #52 |
| MOB-T05 | Opening, fetching, receiving, and previewing do not change the phone OS clipboard or send its contents; explicit Copy and explicit Paste then Send work in real third-party apps. | #51, #52 |
| MOB-T06 | Text/image limits, old wire versions, lost receipts, duplicate events, three-node no-forwarding, cache clear, restart, and resource measurements produce truthful bounded results. | #50, #51, #52 |

Issue [#50](https://github.com/KatInBroek/shuttli/issues/50) owns shared core/SDK/protocol and desktop changes. [#51](https://github.com/KatInBroek/shuttli/issues/51) and [#52](https://github.com/KatInBroek/shuttli/issues/52) own independent native apps and real-device acceptance. Simulator or core-only results do not complete a platform issue. Keep machine-specific test evidence outside the repository.
