"""Negative graph fixtures: the gate must fail, not merely pass today's graph."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("architecture", ROOT / "tools/check_architecture.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ArchitectureTests(unittest.TestCase):
    def setUp(self):
        self.policy = json.loads((ROOT / "tools/architecture.json").read_text())
        self.graph = {
            "packages": [{"id": name, "name": name, "license": "MIT",
                          "metadata": {"shuttli": {"layer": role}}}
                         for name, role in self.policy["packages"].items()],
            "workspace_members": list(self.policy["packages"]),
            "resolve": {"nodes": [{"id": name, "dependencies": []}
                                   for name in self.policy["packages"]]},
        }

    def edge(self, src, dst):
        for node in self.graph["resolve"]["nodes"]:
            if node["id"] == src:
                node["dependencies"].append(dst)

    def external(self, name):
        self.graph["packages"].append({"id": name, "name": name})
        self.graph["resolve"]["nodes"].append({"id": name, "dependencies": []})

    def test_valid_ui_api_dependency(self):
        self.edge("shuttli-cli", "shuttli-api")
        self.assertEqual(module.violations(self.graph, self.policy), [])

    def test_ui_cannot_access_concrete_adapter(self):
        self.edge("shuttli-cli", "shuttli-adapters")
        self.assertTrue(any("ui may not reach adapter" in e for e in module.violations(self.graph, self.policy)))

    def test_core_cannot_import_runtime_or_database(self):
        for dependency in ["tokio", "rusqlite"]:
            with self.subTest(dependency=dependency):
                self.external(dependency)
                self.edge("shuttli-core", dependency)
                self.assertTrue(any(dependency in e for e in module.violations(self.graph, self.policy)))

    def test_transitive_allowed_helper_cannot_smuggle_core_into_ui(self):
        self.external("helper")
        self.policy["external_by_layer"]["api"] = ["helper"]
        self.policy["external_by_layer"]["ui"] = ["helper"]
        self.edge("shuttli-cli", "shuttli-api")
        self.edge("shuttli-api", "helper")
        self.edge("helper", "shuttli-core")
        self.assertTrue(any("ui may not reach core" in e for e in module.violations(self.graph, self.policy)))

    def test_production_host_cannot_depend_on_fixtures(self):
        self.edge("shuttli-host", "shuttli-contract-tests")
        self.assertTrue(module.violations(self.graph, self.policy))

    def test_relabeling_adapter_as_value_is_rejected(self):
        for package in self.graph["packages"]:
            if package["name"] == "shuttli-adapters":
                package["metadata"]["shuttli"]["layer"] = "value"
        self.assertTrue(any("differs from policy" in e for e in module.violations(self.graph, self.policy)))

    def test_metadata_without_resolve_is_rejected(self):
        self.graph["resolve"] = None
        self.assertTrue(module.violations(self.graph, self.policy))

    def test_unregistered_workspace_package_is_rejected(self):
        package = copy.deepcopy(self.graph["packages"][0])
        package["id"] = package["name"] = "unregistered"
        self.graph["packages"].append(package)
        self.graph["workspace_members"].append(package["id"])
        self.assertTrue(any("unregistered workspace" in e for e in module.violations(self.graph, self.policy)))

    def test_aliases_use_opaque_ids_not_dependency_display_names(self):
        old = "shuttli-adapters"
        new = "opaque-id-for-adapter"
        self.graph["workspace_members"].remove(old)
        self.graph["workspace_members"].append(new)
        for package in self.graph["packages"]:
            if package["id"] == old:
                package["id"] = new
        for node in self.graph["resolve"]["nodes"]:
            if node["id"] == old:
                node["id"] = new
        self.edge("shuttli-cli", new)
        self.assertTrue(any("ui may not reach adapter" in e for e in module.violations(self.graph, self.policy)))


if __name__ == "__main__":
    unittest.main()
