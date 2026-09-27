# Shared mobile SDK and desktop-assisted discovery

Status: proposed, not implemented. Track [shared SDK #50](https://github.com/KatInBroek/shuttli/issues/50), [iOS #51](https://github.com/KatInBroek/shuttli/issues/51) and [Android #52](https://github.com/KatInBroek/shuttli/issues/52).

## Scope

The first mobile apps work while open: explicitly import and send the current clipboard, receive remote content into an inbox, preview it, and copy it locally on request. Use the separately installed official Tailscale client. No QR code, manual address, application account, central clipboard server or Tailscale administrative token is required.

A desktop discovers the phone through its Tailscale peer list and connects to the foreground app. Each desktop is an ordinary peer; none is a coordinator or trust broker. A received item does not automatically replace the phone's system clipboard. Mobile sending is manual; launching, resuming, discovering a device or enabling a direction never sends existing content.

Background clipboard monitoring, lock-screen delivery, push notifications, share extensions, shortcuts, files, independent LAN discovery and phone-only initial discovery are outside this first mobile delivery.

## Shared code and boundaries

Shared means the same Rust source compiled for each target through Cargo path dependencies. It does not mean copies translated into Swift/Kotlin or remote execution on a desktop. Each phone has its own identity, policy, state and core instance.

| Component | Reuse and required changes |
| --- | --- |
| model, core | Same source; separate permission to receive a body from permission to write the OS clipboard. Preserve desktop invariants. |
| application, ports, api | Same use cases and contracts; add explicit import, draft, inbox, local copy, capabilities and foreground lifecycle. |
| runtime | Bounded serial scheduling with explicit start, pause and stop. |
| protocol | Extract shared version dispatch, framing and receipt contracts. |
| adapters-common | Extract TLS, normalization, identity, encryption and storage from desktop-specific adapters. |
| desktop adapters | Keep Tailscale enumeration, X11, macOS helper, autostart and desktop notifications here. |
| mobile-sdk | Composition root assembling application/runtime/common adapters and injected native services. |
| mobile-ffi | Generated UniFFI boundary and Swift/Kotlin bindings. |
| native apps | Independent SwiftUI and Compose UIs, clipboard/lifecycle adapters, packaging and real-device tests. |

The existing desktop service captures a clipboard baseline and treats successful receipt as OS application. Mobile must change these shared contracts explicitly; an in-memory inbox must never masquerade as the OS clipboard.

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
5. Multiple desktops connect independently; deduplicate by public-key identity. No peer propagates another device's permission or clipboard content.

The first-run prerequisite is at least one upgraded desktop. Tailnet policy must allow desktop-to-phone application connections. A phone refresh can retry its own listener or known connections; it cannot force an unknown desktop to scan. With the current approximately 30-second desktop refresh, target discovery within 40 seconds on a healthy network, subject to real-device validation.

Do not access another app's private LocalAPI/container, scan the whole address space, require root/jailbreak, or accept every public/LAN interface merely to make the listener reachable. Verify Tailscale interface binding, IPv4/IPv6, cellular routing, VPN changes and permissions on devices. Address filtering never replaces TLS identity and application policy. Tailscale connectivity without external network access is not guaranteed; standalone LAN support is separate.

## Import, receive and local copy

`ImportContent -> bounded draft -> SendDraft` describes the intended use case; final names belong to the SDK contract. Importing never sends. Explicit send publishes the displayed immutable snapshot to currently allowed targets; a later clipboard change cannot silently replace that draft. Reject recognizable sensitive/transient content. Missing sensitive markers do not imply password detection.

| Receive mode | Completion meaning |
| --- | --- |
| Desktop clipboard | Applied only after conflict checks, OS write, independent OS readback and durable receipt. |
| Mobile inbox | Received after validation and admission into the bounded app cache; no OS clipboard read/write is implied. |

Copying an inbox item is a separate local operation with a fresh core write permit and current platform checks. It never automatically broadcasts on mobile. Explicit resend creates a new event and rechecks permissions. Local copy success/failure/unverified state does not retroactively turn a Received receipt into Applied. A platform API accepting bytes is not independent readback; report uncertainty honestly.

## History and active content

Keep at most one active incoming item and a separately bounded draft. Replacing an active item must not make an already open detail view act on different content. History off still permits viewing/copying the current inbox item.

History defaults to 20 events, configurable from 0 to 10,000 subject to shared metadata, RAM and image quotas. Text/list data stays in the SDK process session; images use a temporary encrypted cache and a session-only key. Active content and history share the same object budget. Per-peer retention can only tighten the global mode.

Clearing history also clears app drafts/inbox/previews and cancels references before deletion. It does not clear system clipboards or recall remote deliveries. Hide previews on temporary inactivity; clear drafts on background entry. A surviving process may retain inbox/history in RAM, but process restart restores no content and clears orphan ciphertext. Identity, policy, replay protection and necessary receipts are persisted separately. Historical Received remains historical even after its body is gone; receipt queries never redeliver content.

Android clipboard image exports have an independent bounded OS-use lifetime, described in the Android spec. Exclude private identities, policy ledgers and cache objects from cloud backup and logs. History does not require Keychain; device identity protection is a separate platform decision.

## Protocol and API compatibility

Wire v1 has strict parsing and only Applied semantics. Introduce explicit Wire v2 capability negotiation before the mobile UI:

- Upgraded desktops retain v1 interoperability; mobile inbox mode requires v2.
- Dispatch by version before strict parsing. Reject invalid lengths, fields and identity bindings.
- Prefer v2. Fall back to a fresh v1 desktop connection only after an explicit version-incompatibility result, never after TLS/identity failure.
- Mobile cannot downgrade to v1 and fabricate Applied. Older desktops must receive an actionable upgrade diagnosis where possible.
- Version local IPC and FFI separately. Unknown receipt states cannot default to success.
- Show Received distinctly in desktop CLI/UI. Lost receipts permit bounded sender-scoped STATUS queries, never automatic retransmission or clipboard reapplication.

## Independent tasks and gates

| ID | Deliverable and independent acceptance |
| --- | --- |
| M01 | Linux discovers real iOS and Android devices; authenticated bidirectional synthetic traffic, cellular/resume and denied-policy cases. Verify actual listener routing. |
| M02 | Minimal Swift/Kotlin calls into the same Rust code; thread/cancel/release tests, fixed toolchains/ABIs and review of generated unsafe boundaries. Public bindings cannot expose internal permits. |
| M03 | Shared import/inbox/receipt contracts across core, application, ports, API and Wire v2; permissions, replay, no-forwarding and old-desktop behavior. Every core coverage metric remains above 95%. |
| M04 | Extract common adapters without changing desktop behavior, then assemble a mobile SDK with explicit capabilities and lifecycle. No desktop dependency leaks into mobile. |
| M05 | Desktop v1 regression, phone probing, Received UI/CLI, disconnect results and policy handling, testable without mobile UI. |
| M06 | Traceable desktop/SDK/wire/mobile version matrix and independent release evidence for each platform. |

M01/M02 resolve feasibility first; M03-M05 establish shared behavior. iOS and Android UI releases proceed independently after their shared prerequisites. Use one repository/workspace; do not create empty app projects as evidence of implementation.

Use the same core test suite, with function/line/region coverage each strictly above 95%. Native suites cover lifecycle, clipboard, bindings and UI. Simulators cannot replace real Tailscale VPN routing, permissions or cross-app image paste. Minimum device E2E: Linux plus iPhone, and Linux plus Android; also exercise three-node no-forwarding, lost receipts, cleanup and restart. Use synthetic content and keep machine-specific evidence outside the repository.

Measure idle usage, 20 image transfers and repeated foreground/background cycles before fixing mobile memory budgets. Unbounded growth, duplicate SDKs or background scanning block release. No clipboard polling while idle; no background service or wake lock to keep synchronization alive. Share the existing brand and English/Dutch/German/French semantic catalogs, with native accessibility/plural handling.

References: [UniFFI](https://mozilla.github.io/uniffi-rs/latest/), [Rust iOS targets](https://doc.rust-lang.org/rustc/platform-support/apple-ios.html), [Rust Android targets](https://doc.rust-lang.org/rustc/platform-support/android.html).
