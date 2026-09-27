"""Presentation contract: direction indicators and bounded user intents."""
import itertools
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'crates/native-ui/linux'))
from tray_model import TrayState, icon_pixels


class TrayModelTests(unittest.TestCase):
    def test_all_directions_have_distinct_icons_and_consistent_controls(self):
        states = [TrayState(send, receive, True) for send, receive in itertools.product((False, True), repeat=2)]
        for size in (16, 24, 32):
            icons = [icon_pixels(s, size) for s in [*states, TrayState()]]
            self.assertEqual(len(set(icons)), 5)
            self.assertTrue(all(len(p) == size * size * 4 for p in icons))
        for state in states:
            with self.subTest(state=state):
                response = {'type': 'status', 'status': {'settings': {'send': state.send, 'receive': state.receive}}}
                self.assertEqual(TrayState.from_answer(response), state)
                items = {i: (enabled, checked) for i, _, enabled, checked in state.menu()}
                self.assertEqual(items[20], (True, state.send))
                self.assertEqual(items[21], (True, state.receive))
                self.assertEqual(items[30][0], state.send)
                self.assertEqual(state.action(20), {'command': 'set_directions', 'send': not state.send})
                self.assertEqual(state.action(21), {'command': 'set_directions', 'receive': not state.receive})
                self.assertEqual(state.action(30), {'command': 'send'} if state.send else None)

    def test_unavailable_service_keeps_window_access_but_disables_mutations(self):
        state = TrayState.from_answer({'type': 'error', 'message': 'offline'})
        items = {i: enabled for i, _, enabled, _ in state.menu()}
        self.assertTrue(items[10])
        for ident in (20, 21, 30):
            self.assertFalse(items[ident])
            self.assertIsNone(state.action(ident))
        self.assertIsNone(state.action(999))
        with self.assertRaises(ValueError):
            icon_pixels(state, 1024)


class TraySchedulingTests(unittest.TestCase):
    def test_user_actions_survive_a_status_request_without_unbounded_queue(self):
        import importlib.util
        import os
        import types
        from unittest.mock import Mock, patch
        spec = importlib.util.spec_from_file_location('shuttli_tray_under_test', Path(__file__).resolve().parents[2] / 'crates/native-ui/linux/tray.py')
        module = importlib.util.module_from_spec(spec)
        repository = types.ModuleType('gi.repository')
        repository.Gio = Mock()
        repository.GLib = Mock()
        with patch.dict(sys.modules, {'gi': types.ModuleType('gi'), 'gi.repository': repository}):
            spec.loader.exec_module(module)
        tray = module.Tray.__new__(module.Tray)
        from i18n import Translator
        tray.locale = Translator(environ={'LANG': 'en'})
        tray.locale_changed = False
        tray.parent = os.getppid()
        tray.children = []
        tray.state = TrayState(True, True, True)
        tray.error = ''
        tray.busy = True
        tray.pending_directions = {}
        tray.pending_send = False
        for _ in range(100):
            tray.update({'command': 'set_directions', 'send': False})
            tray.update({'command': 'set_directions', 'receive': False})
            tray.update({'command': 'send'})
        self.assertEqual(tray.pending_directions, {'send': False, 'receive': False})
        self.assertTrue(tray.pending_send)
        answer = {'type':'status', 'status':{'settings':{'send':True, 'receive':True}}}
        tray.update = Mock()
        tray.updated(answer, '')
        tray.update.assert_called_once_with({'command':'set_directions','send':False,'receive':False})
        tray.update.reset_mock()
        tray.updated(answer, '')
        tray.update.assert_called_once_with({'command':'send'})
        self.assertFalse(tray.pending_send)
        tray.update.reset_mock()
        tray.updated(answer, '')
        tray.update.assert_not_called()
