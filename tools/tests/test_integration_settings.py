"""Acceptance failures must not leave user devices in manual mode."""
import contextlib
import copy
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'integration'))
import linux_hosts


class SettingsRestorationTests(unittest.TestCase):
    def test_failed_real_host_suite_restores_all_settings_on_both_nodes(self):
        class FixtureFailure(Exception):
            pass

        original = dict(automatic=True, send=False, receive=True, notifications=True,
                        text=False, png=True, history='status', history_limit=7,
                        peers={'user-device': {'send': True, 'receive': False}})
        nodes = []

        class Node(linux_hosts.Node):
            def __init__(self, spec):
                self.label = spec['label']
                self.children = []
                self.settings = copy.deepcopy(original)
                self.configurations = []
                nodes.append(self)

            def api(self, *args, **kwargs):
                if args[0] == 'request':
                    self.settings = json.loads(args[1])['action']['settings']
                    self.configurations.append(copy.deepcopy(self.settings))
                elif args[0] == 'status':
                    return {'status': {'clipboard_available': True, 'device': self.label}}
                elif args[0] == 'devices':
                    return {'devices': [{'id': x, 'online': True} for x in ('a', 'b')]}
                elif args[0] == 'history':
                    return {'entries': []}
                elif args[0] == 'peer':
                    self.settings['peers'][args[1]] = {'send': args[3] == 'on'}
                elif args[0] == 'set':
                    key = args[1].replace('-', '_')
                    self.settings[key] = (args[2] == 'on') if args[2] in ('on', 'off') else args[2]
                return {'type': 'settings', 'settings': copy.deepcopy(self.settings)}

            def fixture(self, kind='text'):
                raise FixtureFailure('synthetic clipboard unavailable')

        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / 'config.json'
            config.write_text(json.dumps({'nodes': [{'label': x} for x in ('a', 'b')]}))
            argv = ['linux_hosts.py', '--config', str(config), '--report', str(Path(directory) / 'report.json'),
                    '--allow-clipboard-overwrite']
            with patch.object(linux_hosts, 'Node', Node), patch.object(sys, 'argv', argv), contextlib.redirect_stdout(io.StringIO()):
                with self.assertRaises(FixtureFailure):
                    linux_hosts.main()
        self.assertEqual(len(nodes), 2)
        for node in nodes:
            self.assertFalse(node.configurations[0]['automatic'])
            self.assertEqual(node.configurations[0]['peers'], {})
            self.assertEqual(node.settings, original)


if __name__ == '__main__':
    unittest.main()
