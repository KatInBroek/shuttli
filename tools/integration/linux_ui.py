#!/usr/bin/env python3
"""GTK interaction/visual regression using a synthetic public API fixture.

Run with GDK_BACKEND=x11 python3 tools/integration/isolated_session.py --
  xvfb-run -a /usr/bin/python3 tools/integration/linux_ui.py.
No user clipboard, synchronization settings, or autostart files are changed.
"""
import base64
import copy
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import time
from isolated_session import require_isolated_session

require_isolated_session()

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'crates/native-ui/linux'))
spec = importlib.util.spec_from_file_location('native_window', ROOT / 'crates/native-ui/linux/shuttli.py')
ui = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ui)
from gi.repository import Gtk, GLib


def pump(predicate=lambda: False, timeout=.15):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        while GLib.MainContext.default().pending():
            GLib.MainContext.default().iteration(False)
        if predicate():
            return True
        time.sleep(.01)
    return predicate()


def descendants(widget):
    yield widget
    child = widget.get_first_child()
    while child:
        yield from descendants(child)
        child = child.get_next_sibling()


class Fixture(ui.Window):
    def request(self, action):
        command = action['command']
        if command.startswith('_'):
            return super().request(action)
        self.actions.append(copy.deepcopy(action))
        if command == 'status':
            if self.unavailable:
                return {'type': 'error', 'message': 'fixture service unavailable'}
            return {'type': 'status', 'status': dict(device='01'*32, clipboard='Synthetic clipboard', clipboard_available=True,
                    settings=copy.deepcopy(self.fixture_settings), devices=self.fixture_counts.copy(), sequence=self.seq, policy_revision=self.revision, last_error=None)}
        if command in ('settings', 'configure', 'set_directions'):
            if command == 'configure':
                if action['expected'] != self.fixture_settings:
                    return {'type': 'error', 'message': 'settings changed; refresh before retrying'}
                self.fixture_settings = copy.deepcopy(action['settings'])
                self.revision += 1
            if command == 'set_directions':
                self.fixture_settings.update({k: v for k, v in action.items() if k != 'command'})
                self.revision += 1
            return {'type': 'settings', 'settings': copy.deepcopy(self.fixture_settings)}
        if command == 'devices':
            return {'type': 'devices', 'devices': copy.deepcopy(self.fixture_peers), 'settings': copy.deepcopy(self.fixture_settings)}
        if command == 'peer':
            current = dict(ui.DEFAULT_POLICY, **self.fixture_settings['peers'].get(action['id'], {}))
            if action['expected'] != current:
                return {'type': 'error', 'message': 'device settings changed; refresh before retrying'}
            self.fixture_settings['peers'][action['id']] = copy.deepcopy(action['policy'])
            self.revision += 1
            return {'type': 'settings', 'settings': copy.deepcopy(self.fixture_settings)}
        if command == 'history':
            return {'type': 'history', 'entries': copy.deepcopy(self.entries[action['offset']:action['offset']+action['limit']])}
        if command == 'preview':
            return dict(type='preview', format='text', base64=base64.b64encode(b'Synthetic clipboard preview\nSecond line').decode())
        if command == 'clear_history':
            self.entries.clear()
        if command == 'autostart':
            return {'type': 'autostart', 'status': {'state': 'disabled', 'message': 'disabled'}}
        return {'type': 'done', 'message': 'Synthetic operation completed'}


def run():
    report = Path(os.environ.get('SHUTTLI_UI_REPORT', '/tmp/shuttli-ui-qa'))
    report.mkdir(parents=True, exist_ok=True)
    app = Gtk.Application(application_id='org.shuttli.UiFixture')
    app.register(None)
    results = []
    with tempfile.TemporaryDirectory() as directory:
        # Set fixtures before __init__ schedules async reads.
        window = Fixture.__new__(Fixture)
        window.actions = []
        window.unavailable = False
        window.seq = window.revision = 1
        window.fixture_counts = dict(discovered=2, send=0, receive=2)
        window.fixture_settings = dict(version=1, send=True, receive=True, automatic=False, text=True, png=True,
                notifications=True, history='content', history_limit=20, history_days=7,
                history_bytes=134217728, history_memory_bytes=16777216, peers={})
        window.fixture_peers = [dict(id='02'*32, name='Laptop', address='100.64.0.2', online=True),
                               dict(id='03'*32, name='build-server-eu-west-ci-runner-03', address='100.64.0.3', online=False)]
        window.entries = [dict(id=i, event=dict(origin=[1]*32, epoch=[2]*16, seq=1 if i<3 else 2), peer=('02' if i!=2 else '03')*32,
                direction='send' if i<3 else 'receive', state='unknown' if i==2 else 'applied', format='text', bytes=39,
                time=1790520000+i, available=True, detail='remote OS readback and durable receipt completed' if i!=2 else 'Receipt unconfirmed') for i in (1,2,3)]
        Fixture.__init__(window, app, str(Path(directory)/'control.sock'))
        window.present()
        def wait_for(predicate):
            assert pump(predicate, 4), 'timed out waiting for GTK response'
        def labels():
            return [w.get_text() for w in descendants(window) if isinstance(w, Gtk.Label)]
        def click(text, root=None):
            candidates = [w for w in descendants(root or window) if isinstance(w, Gtk.Button) and w.get_label() == text]
            assert candidates, text
            candidates[0].emit('clicked')
            pump()
        def screenshot(name):
            from PIL import ImageGrab
            pump(timeout=.25)
            ImageGrab.grab(bbox=(0, 0, window.get_width(), window.get_height()), xdisplay=os.environ['DISPLAY']).save(report/(name+'.png'))
        wait_for(lambda: window.t('ui.manual') in labels())
        wait_for(lambda: window.t('status.allowed_send') in labels())
        def statistics():
            result = {}
            for widget in descendants(window.content):
                if isinstance(widget, Gtk.Label) and widget.get_text() in [window.t(k) for k in ('status.discovered', 'status.allowed_send', 'status.allowed_receive')]:
                    parent = widget.get_parent()
                    result[widget.get_text()] = [w.get_text() for w in descendants(parent) if isinstance(w, Gtk.Label)]
            return result
        assert '0' in statistics()[window.t('status.allowed_send')]
        assert '2' in statistics()[window.t('status.allowed_receive')]
        assert labels().index(window.t('status.discovered')) < labels().index(window.t('ui.directions'))
        window.fixture_counts = dict(discovered=3, send=1, receive=3)
        window.poll()
        wait_for(lambda: '1' in statistics().get(window.t('status.allowed_send'), []))
        window.fixture_settings['send'] = False
        window.fixture_counts['send'] = 0
        window.revision += 1
        window.poll()
        wait_for(lambda: '0' in statistics().get(window.t('status.allowed_send'), []))
        window.fixture_settings['send'] = True
        window.fixture_counts = dict(discovered=2, send=0, receive=2)
        window.revision += 1
        window.poll()
        wait_for(lambda: '2' in statistics().get(window.t('status.discovered'), []))
        results.append('device statistics precede directions and refresh for discovery and global permission changes')
        screenshot('status-light')
        results.append('status shows manual mode independently from send permission')
        # Navigate using real sidebar buttons, then open device through row click.
        window.nav_buttons[1][1].emit('clicked')
        wait_for(lambda: 'Laptop   ·   '+window.t('devices.online') in labels())
        screenshot('devices-light')
        device_buttons = [w for w in descendants(window.content) if isinstance(w, Gtk.Button) and w.has_css_class('row-button')]
        device_buttons[0].emit('clicked')
        wait_for(lambda: len([w for w in descendants(window) if isinstance(w, Gtk.Switch)]) == 5)
        screenshot('device-light')
        switches = [w for w in descendants(window) if isinstance(w, Gtk.Switch)]
        switches[0].set_active(True)
        pump()
        dialogs = [w for w in Gtk.Window.list_toplevels() if w.get_modal()]
        assert len(dialogs) == 1
        dialog = dialogs[0]
        allow = next(w for w in descendants(dialog) if isinstance(w, Gtk.Button) and w.get_label() == window.t('ui.allow'))
        assert not allow.get_sensitive()
        assert not any(a['command'] == 'peer' for a in window.actions)
        screenshot('allow-sending')
        check = next(w for w in descendants(dialog) if isinstance(w, Gtk.CheckButton))
        check.set_active(True)
        assert allow.get_sensitive()
        allow.emit('clicked')
        wait_for(lambda: window.fixture_settings['peers'].get('02'*32, {}).get('send'))
        results.append('allow sending requires fingerprint checkbox and explicit confirmation')
        window.navigate('history')
        wait_for(lambda: window.t('ui.entries', count=2) in labels())
        screenshot('history-light')
        assert window.t('state.unknown') in labels() and window.t('state.applied') in labels()
        rows = [w for w in descendants(window.content) if isinstance(w, Gtk.Button) and w.has_css_class('row-button')]
        rows[1].emit('clicked')
        wait_for(lambda: 'Synthetic clipboard preview\nSecond line' in labels())
        screenshot('history-detail-light')
        click(window.t('history.copy_local'))
        assert any(a == {'command': 'copy', 'id': 2, 'local_only': True} for a in window.actions)
        click(window.t('history.copy'))
        assert any(a == {'command': 'copy', 'id': 2, 'local_only': False} for a in window.actions)
        click(window.t('history.resend'))
        assert any(a == {'command': 'resend', 'id': 2} for a in window.actions)
        results.append('history groups destinations and preserves unconfirmed result; three actions send distinct API intents')
        window.navigate('settings')
        wait_for(lambda: any(isinstance(w, Gtk.SpinButton) for w in descendants(window)))
        count = next(w for w in descendants(window) if isinstance(w, Gtk.SpinButton))
        count.set_value(37)
        window.fixture_settings['send'] = False
        window.revision += 1
        window.poll()
        pump(timeout=.3)
        assert count.get_value_as_int() == 37
        click(window.t('ui.apply'))
        wait_for(lambda: window.fixture_settings['history_limit'] == 37)
        assert not window.fixture_settings['send']
        results.append('retention draft survives external direction update; applying limit preserves external change')
        screenshot('settings-light')
        for code in ('en','nl','de','fr'):
            language = next(w for w in descendants(window) if isinstance(w, Gtk.DropDown) and w.get_model().get_n_items() == len(ui.LANGUAGES))
            language.set_selected([key for key, _ in ui.LANGUAGES].index(code))
            wait_for(lambda: window.locale.language == code and window.t('locale.label') in labels())
            assert window.t('nav.settings') in labels()
            window.set_default_size(720,520)
            pump(timeout=.2)
            screenshot('settings-small-'+code)
            assert window.get_width() <= 720, (code, window.get_width())
        results.append('four languages switch live, translate navigation and fit 720px window')
        Gtk.Settings.get_default().set_property('gtk-application-prefer-dark-theme', True)
        language = next(w for w in descendants(window) if isinstance(w, Gtk.DropDown) and w.get_model().get_n_items() == len(ui.LANGUAGES))
        language.set_selected([key for key, _ in ui.LANGUAGES].index('en'))
        wait_for(lambda: window.locale.language == 'en' and window.t('locale.label') in labels())
        window.navigate('history')
        wait_for(lambda: window.t('ui.entries', count=2) in labels())
        screenshot('history-dark')
        click(window.t('ui.clear'))
        dialog = next(w for w in Gtk.Window.list_toplevels() if w.get_modal())
        click(window.t('ui.cancel'), dialog)
        assert len(window.entries)==3
        click(window.t('ui.clear'))
        dialog = next(w for w in Gtk.Window.list_toplevels() if w.get_modal())
        click(window.t('ui.clear'), dialog)
        wait_for(lambda: window.t('ui.entries', count=0) in labels())
        results.append('clear requires confirmation; cancellation retains history')
        window.entries = [dict(id=4, event=dict(origin=[1]*32, epoch=[2]*16, seq=3), peer='01'*32,
                direction='local', state='applied', format='text', bytes=39,
                time=1790520004, available=True, detail='Local clipboard copy')]
        window.navigate('history')
        wait_for(lambda: window.t('ui.local_copy') in labels())
        assert window.t('ui.not_sent') in labels()
        assert window.t('state.applied') not in labels()
        window.entries.append(dict(window.entries[0], id=5, peer='02'*32, direction='send', state='unknown'))
        window.navigate('history')
        wait_for(lambda: window.t('state.unknown') in labels())
        assert window.t('ui.entries', count=1) in labels()
        assert window.t('ui.local_copy') in labels()
        screenshot('local-copy-with-delivery')
        results.append('local copies show unsent honestly and share one event with automatic delivery results')
        window.unavailable = True
        window.poll()
        wait_for(lambda: window.t('common.unavailable') == window.summary_directions.get_text())
        assert not window.content.get_sensitive()
        screenshot('service-unavailable')
        window.unavailable = False
        window.poll()
        wait_for(lambda: window.content.get_sensitive())
        results.append('service outage disables controls and reconnect restores them')
        window.close()
        pump()
    (report/'results.json').write_text(json.dumps(results,indent=2)+'\n')
    print('\n'.join('PASS '+r for r in results))


if __name__ == '__main__':
    run()
