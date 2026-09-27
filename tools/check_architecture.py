#!/usr/bin/env python3
"""Check all-feature Cargo metadata, including transitive/build/dev/target edges.

Explicit admission of every package and external dependency keeps new adapters,
test helpers, aliases and platform-specific edges from bypassing the policy.
This is a code-structure gate, not a sandbox against arbitrary same-user code.
"""
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def violations(metadata, policy):
    if not metadata.get("resolve"):
        return ["metadata has no resolved dependency graph; do not use --no-deps"]
    packages = {p["id"]: p for p in metadata["packages"]}
    members = set(metadata["workspace_members"])
    graph = {node["id"]: node["dependencies"] for node in metadata["resolve"]["nodes"]}
    errors = []
    roles = {}
    for package_id in members:
        package = packages[package_id]
        name = package["name"]
        role = policy["packages"].get(name)
        declared = (package.get("metadata") or {}).get("shuttli", {}).get("layer")
        if role is None or role not in policy["reachable_layers"]:
            errors.append(f"{name}: unregistered workspace package/layer")
            continue
        if declared != role:
            errors.append(f"{name}: declared layer {declared!r} differs from policy {role!r}")
        roles[package_id] = role
        if package.get("license") != "MIT":
            errors.append(f"{name}: workspace license must be MIT")
    for origin, role in roles.items():
        seen = {origin}
        pending = [(target, [packages[origin]["name"]]) for target in graph.get(origin, [])]
        while pending:
            target, path = pending.pop()
            if target in seen:
                continue
            seen.add(target)
            package = packages[target]
            path = [*path, package["name"]]
            if target in members:
                reached = roles.get(target)
                if reached not in policy["reachable_layers"][role]:
                    errors.append(f"{' -> '.join(path)}: {role} may not reach {reached}")
            elif package["name"] not in policy["external_by_layer"].get(role, []):
                errors.append(f"{' -> '.join(path)}: external dependency not admitted for {role}")
            # Always traverse, even an already forbidden intermediate node.
            pending.extend((dep, path) for dep in graph.get(target, []))
    return sorted(errors)


def main():
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--all-features"], cwd=ROOT))
    policy = json.loads((ROOT / "tools/architecture.json").read_text())
    errors = violations(metadata, policy)
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    external = [p for p in metadata["packages"] if p["id"] not in metadata["workspace_members"]]
    print(f"Architecture OK: {len(metadata['workspace_members'])} workspace packages, "
          f"{len(external)} admitted external dependencies; all-feature resolved graph checked.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
