# Windows and regular-file extension contracts

Status: planned. Windows native integration and file transfer are not implemented. Track [Windows tasks](https://github.com/KatInBroek/shuttli/milestone/7) and [file tasks](https://github.com/KatInBroek/shuttli/milestone/8). Their delivery gates are independent.

## Shared architecture

Reuse the same core, application, model, protocol and storage behavior. Windows implements clipboard/platform adapters, the host, native presentation, packaging and real-platform tests; it must not introduce another permission or loop-prevention implementation.

Core and shared contracts cannot contain HWND, COM, NSPasteboard, D-Bus, Unix file descriptors or platform paths. UI/CLI calls the public application API. Only the composition root assembles concrete adapters. Platform conditionals stay at adapter/host boundaries. No dynamic plugin ABI or universal GUI framework is required.

## Clipboard adapter responsibilities

| Operation | Contract |
| --- | --- |
| capabilities | Report read/write formats, semantic versions, observation/readback strength and permission state separately. |
| observe | Supply bounded revision/owner/type/sensitive evidence; never grant publication or eagerly load full images/files. |
| capture | Create an immutable bounded snapshot for a specific revision; return stale if the clipboard changes during capture. |
| apply | Consume a core-issued permit with operation, baseline and policy generation; reject revoked/stale work and report actual OS results. |
| verify | Read through an independent OS path and report representation/proof strength; never return the input buffer as verification. |
| release | Retain content while selection, transfer, preview or active export still uses it. |

Revision combines backend generation and an opaque platform marker. It is not a network sequence or a globally comparable clock. Report unknown, wrapped or coalesced evidence conservatively. Errors include unavailable, permission_denied, busy, stale/superseded, unsupported_type, invalid_content, too_large, write_failed, verification_failed and cancelled.

Do not hold a system clipboard lock while waiting for networking, storage or large image encoding. After entering an uncancellable OS call, report its actual completion rather than promising rollback. Application serialization does not make an external application's clipboard access atomic.

## Text and image representation

Wire text is strict UTF-8. Preserve transmitted bytes; semantic text.v1 comparison only maps CRLF to LF. Preserve isolated CR, Unicode sequences and whitespace; do not normalize NFC/NFKC or trim. Reject invalid UTF-8, invalid UTF-16 and embedded wire NUL.

For CF_UNICODETEXT, find the first UTF-16 NUL within a bounded buffer; trailing allocation slack is not content and missing termination is invalid. On output, insert CR before LF only when not already preceded by CR. Mixed CRLF/LF/isolated CR must not become CRCRLF. Use the same semantic vectors on every OS.

Images use lossless PNG internally and versioned normalized pixel verification. The UI exposes one Images category. Native DIB/DIBV5, macOS and Linux representations are converted at adapter boundaries, never serialized as OS handles. Apply the common encoded-byte, decoded-pixel, dimension, concurrency and allocation limits before decoding.

Windows testing must cover stride, row direction, color profiles, alpha and equivalent native representations. Candidate output is registered PNG plus compatible DIBV5; finalize this against actual screenshot sources and paste targets. Record which representation was read back. Successful PNG verification cannot conceal a promised DIB failure, and an all-zero-alpha heuristic cannot silently redefine transparency. Invalid, animated, truncated and oversized inputs must fail safely.

## Windows platform implementation

- Run a hidden-window message loop in the interactive user's session; use AddClipboardFormatListener/WM_CLIPBOARDUPDATE and GetClipboardSequenceNumber/owner evidence. Avoid full-content polling.
- Handle sequence wrap, delayed rendering, notification coalescing and external managers as platform evidence, not guaranteed user-copy counts.
- Use bounded retries around clipboard contention. Initially publish data eagerly; do not introduce custom delayed rendering without a separate contract.
- Cover lock/unlock, logout, RDP transitions and external clipboard redirection. Do not mix users or sessions; no Session 0 service operating a user's clipboard.
- Use a local named pipe with explicit user/session ACLs, peer identity checks and remote-client rejection. A guessable pipe name is not authentication.
- Resolve user data/cache paths through platform services. Respect Windows open-handle, sharing, atomic rename and deferred deletion behavior.
- Identity storage may use an appropriate per-user protection mechanism after validation. Session history encryption does not require a persistent OS credential store. Never silently replace a missing identity or fall back to plaintext history.
- Keep singleton lifecycle, tray, notifications, login startup and installation behind platform ports. Existing shared Rust CI is not evidence of a working Windows desktop backend.

Autostart reports requested intent separately from observed enabled/disabled/requires_user_action/unavailable/unknown. Requery after mutation and when settings opens. Do not silently restore an entry disabled by the OS. Turning autostart off does not stop the current agent or alter sync/history policy. Upgrade preserves consent and startup preference; uninstall removes only this application's entry. Simultaneous manual/login startup must yield one agent and establish a fresh clipboard baseline without publishing residual content.

Native APIs: [clipboard use](https://learn.microsoft.com/en-us/windows/win32/dataxchg/using-the-clipboard), [standard formats](https://learn.microsoft.com/en-us/windows/win32/dataxchg/standard-clipboard-formats), [named-pipe security](https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipe-security-and-access-rights).

## Regular-file extension: scope gate

Finalize X01 before advertising files.v1. The initial candidate is one or more regular local files. Directories, symlinks/reparse points, virtual files, remote mounts, cut/delete semantics and automatic execution are excluded unless separately designed and accepted. Windows FILEDESCRIPTOR/FILECONTENTS is not equivalent to an accessible local file and may be explicitly unsupported.

The wire manifest contains logical item ID, display name, length and content digest plus file bytes. Never send source absolute paths as payload, interpret remote file://, UNC, drive letters or custom formats as permission to read local files, or inherit sender directories, ACLs, executable bits or deletion commands.

Create bounded disk snapshots before offering a transfer. Check source type, permissions and changes before/after reading; a changing/deleted source fails rather than yielding inconsistent content. This is not an atomic filesystem snapshot against arbitrary concurrent writers. Stream immutable objects with fixed buffers, integrity checks, backpressure and cancellation. Reject incomplete or corrupt sets; never publish a partial file reference.

Separate display names from generated storage names. Freeze portable handling of separators, traversal, absolute paths, reserved Windows names, ADS/colon, case collisions, trailing spaces/dots, length and unrepresentable characters. Defend against symlink/reparse races and never overwrite or clean outside the private receive directory.

## Native exports and retention

| Platform | Candidate local reference |
| --- | --- |
| macOS | File URL list |
| Linux | Supported file URI list plus copy semantics |
| Windows | CF_HDROP pointing to fully received local files |

Stored bytes are only STORED. APPLIED requires native clipboard reference write plus independent readback, and still does not prove a target app has pasted the files.

Active file exports have a separate lifetime from history. Keep referenced files valid while the current clipboard or an active reader/transfer uses them. Content retained for history follows the bounded encrypted-object policy; an explicitly exported plaintext copy in a private directory is a necessary, disclosed exception and counts against total disk quota. History off/status must not retain extra long-lived bodies.

Recopy may materialize a new controlled export from retained encrypted content. Finalize expiry, crash cleanup, open-reader behavior and platform download-origin/security markers before shipping. Do not use history eviction to delete a currently pasteable object. Echo matching uses registered local references/revisions; it must not open arbitrary paths merely to calculate a fingerprint.

## Versioning and independent acceptance

Negotiate connection requirements and per-type semantic versions explicitly. Reject unknown required features at connection setup; unknown optional capabilities can be ignored. Reject an unsupported file offer without destroying text/image capability or degrading a file into path text. Platform MIME/UTI/CF identifiers stay local.

Use the same text/image/permission/replay/receipt/cancellation vectors on all desktop CI targets. Real Windows validation must additionally cover busy/delayed rendering, invalid UTF-16, DIB conversion, session isolation, IPC identity, actual login startup and two/three-platform no-loop tests. File tests cover source mutation, malicious names/paths, partial writes, process termination, quota exhaustion, active exports and real file-manager paste. Report resource usage across all helper/UI processes, not only the Rust host.
