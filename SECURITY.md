# Security policy

Clipboard data can contain passwords, tokens and private messages. This project
is open source under MIT so its behavior and release process can be inspected.
Open source is not a guarantee of security or an independent security audit.

## Report a vulnerability privately

Use [GitHub private vulnerability reporting](https://github.com/KatInBroek/shuttli/security/advisories/new).
Include the affected commit/package version, OS, a description of the impact and
a minimal reproduction using synthetic clipboard data. Do not put real clipboard
contents, credentials, device identifiers or machine-specific logs into public
issues. Do not publish an exploit before maintainers have had an opportunity to
investigate and coordinate disclosure. There is no guaranteed response SLA yet.

Only the latest development revision is currently maintained; no stable release
or long-term security support commitment has been made.

## Security boundaries and limitations

- Clipboard transport is direct TLS 1.3; Tailscale discovery does not itself grant
  outgoing permission. Compare device fingerprints before allowing sending.
- Receiving defaults to on and can be disabled globally or per device. Received
  clipboard contents are untrusted input; review content before pasting it into
  a shell or another sensitive destination.
- Automatic sending to an allowed device includes ordinary copied content.
  There is no promise that password/token content is detected or filtered.
  Pause sending or limit device permissions when handling sensitive content.
- Text history/list entries are kept in RAM; temporary PNG history is encrypted
  with a session-only key and deleted on eviction/cleanup. Device identity and
  settings persist. Memory/cache handling does not protect a compromised account,
  root access, swap, crash dumps or a compromised authorized receiving device.
- Linux currently relies on X11/XWayland clipboard access. Other applications
  in that desktop session may also access the system clipboard.
- No central clipboard storage server is required. Tailscale infrastructure and
  network policy remain part of the deployment's trust/dependency model.

Security-sensitive changes should describe their threat model and add meaningful
regression tests. Keep cryptographic handling, device policy and loop prevention
in their existing core/adapter boundaries; UI changes must not bypass them.
