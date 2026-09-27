# Contributing

Use issues to discuss bugs and feature scope, and pull requests for reviewable
changes. Include the motivation, compatibility impact and relevant test coverage.
Use fictional device names and synthetic clipboard content in examples. Read
[AGENTS.md](AGENTS.md) for the repository privacy rules and [SECURITY.md](SECURITY.md)
for private vulnerability reporting. Contributions are licensed under MIT.

See the README's build/validation commands and `tools/check_architecture.py` for
the layer boundaries. CI checks Rust formatting, Clippy, workspace tests, Python
tests, desktop contracts and core coverage greater than 95%. Native OS behavior
must also be verified on the relevant platform; mocked tests alone do not prove
desktop clipboard integration. Keep machine-specific execution records outside
the repository and public issue/PR text.

## Packaging and releases

The [Ubuntu packaging guide](docs/install/ubuntu.md) describes local builds.
Release artifacts must be built from an identified clean commit by the public
workflow, include checksums and build metadata, and pass package smoke tests.
Do not bundle personal profiles, credentials or test environments. A preview
package is not a stable release. Never claim unsupported OS compatibility,
independent security certification or reproducible builds without evidence.

## Architecture and extension boundaries

| Layer | Responsibility |
| --- | --- |
| model / core | Platform-independent values and deterministic permission, provenance and loop-prevention decisions. Only core constructs publication/write permits. |
| application / api / ports | Shared use cases, public DTOs and execution contracts; no concrete UI, clipboard, database or network dependency. |
| adapters / runtime | Execute approved work and return bounded results; never create their own publication eligibility. |
| host | Assemble services, lifecycle and authenticated local control. |
| native-ui / cli | Call the public API and display actual state; do not access peer transport, clipboard internals or the database. |

Architecture checks cover transitive, build, development and platform-specific dependencies. Replay the same event/command traces when replacing an adapter or UI. Changing history or notification presentation must not change core publication counts. An OS write and its independent readback are separate operations; returning the input buffer is not verification. Durable replay/receipt records remain independent of disposable session history.

### Control and effect execution

`api::control` is the single production client contract (local API version 2).
Full settings and peer replacements must include the exact `expected` snapshot
used to prepare the change. Conflicts fail without mutation; clients refresh and
ask the user to retry instead of silently replaying stale edits. Direction-only
commands are atomic patches. Per-device send and receive are the only direction
controls; there is no separate device-block switch. Legacy persisted blocked
policies migrate to both directions off, preserving the denial. Autostart state and delivery outcomes are typed;
display messages never determine permissions or transfer outcomes.

The application polls cooperative operations on one thread. Clipboard, storage
and platform effects run on three dedicated workers with bounded queues. Status,
discovery and policy controls remain available while an effect waits. Configuration
revokes old permits before awaiting persistence; enabling requires a successful
save and a fresh clipboard baseline. A newer configuration supersedes an older
pending result. An OS write already in progress cannot promise rollback.

Terminal network events wait for bounded channel capacity. Receives reserve a
completion slot before starting, so cancellation can report its outcome without
blocking the async runtime or allocating an unbounded retry queue. Only optional
notifications may be dropped under overload.

Explicit quit is a lifecycle command, distinct from pause or closing a window.
It invalidates pending permits, drains bounded work, clears disposable history and
preserves settings/startup registration. A local host lifecycle marker distinguishes
an intentional stop from a crashed/unreachable service so presentation clients
close only for the former; the next daemon start removes the marker.

Contract tests exercise the production Store and Clipboard interfaces. SQLite
contracts run on all CI platforms; the independent X11/xclip contract runs only
in isolated Xvfb. Core coverage still includes every executable production item;
declaration-only module manifests are checked separately because LLVM emits no
regions for them. Runtime workers contain no application policy or composition.

## Localization and branding

Keep English documentation and identifiers. UI catalogs intentionally support English, Dutch, German and French. Use stable semantic keys in `crates/native-ui/locales`, identical placeholders across catalogs and explicit plural rules. Device names, clipboard bodies and technical diagnostics are not translated. Language preferences are local and must not change sync policy.

For another UI language, add the complete catalog, register Python/Rust resources and plural handling, then run existing key/placeholder/fallback tests and check long labels in the actual window. Use `{app_name}` for product copy and follow [branding configuration](branding/README.md).

## Documentation maintenance

README is the user entry point. Keep installation guidance and specifications still needed for unfinished work under `docs/`. Track current scope, dependencies and acceptance in the corresponding English issues and grouped roadmap. Remove obsolete implementation diaries, completed planning documents and duplicate indexes; fix repository and issue links when moving a maintained specification. Private execution evidence belongs outside the repository.
