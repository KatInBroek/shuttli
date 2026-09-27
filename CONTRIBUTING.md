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

## Localization and branding

Keep English documentation and identifiers. UI catalogs intentionally support English, Dutch, German and French. Use stable semantic keys in `crates/native-ui/locales`, identical placeholders across catalogs and explicit plural rules. Device names, clipboard bodies and technical diagnostics are not translated. Language preferences are local and must not change sync policy.

For another UI language, add the complete catalog, register Python/Rust resources and plural handling, then run existing key/placeholder/fallback tests and check long labels in the actual window. Use `{app_name}` for product copy and follow [branding configuration](branding/README.md).

## Documentation maintenance

README is the user entry point. Keep installation guidance and specifications still needed for unfinished work under `docs/`. Track current scope, dependencies and acceptance in the corresponding English issues and grouped roadmap. Remove obsolete implementation diaries, completed planning documents and duplicate indexes; fix repository and issue links when moving a maintained specification. Private execution evidence belongs outside the repository.
