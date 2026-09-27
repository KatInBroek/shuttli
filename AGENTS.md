# Repository contribution rules

- Before GitHub operations, read `LOCAL_MAINTENANCE.md` if present. It is an
  untracked, machine-specific guide to the appropriate authentication/tooling.
  Never publish or copy its contents into tracked files.
- Keep machine-specific configuration, usernames, device names, IP addresses,
  credentials, local absolute paths, screenshots and execution reports outside
  the repository, in untracked project memory.
- Apply the same rule to public issues, pull requests and comments. Public design
  documents may describe portable contracts and acceptance criteria; link source
  code and CI checks instead of local-machine reports.
- Integration drivers take machine configuration from an explicit external path.
  Examples use fictional names and documentation addresses only.
- Do not copy project memory into tracked docs. Local memory locations are machine
  specific and must not be written into this file.
