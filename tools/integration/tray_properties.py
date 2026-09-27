#!/usr/bin/env python3
"""Real D-Bus regression for panels that cache existing menu properties.

Run: dbus-run-session -- /usr/bin/python3 tools/integration/tray_properties.py
Uses an isolated synthetic API; never touches the user's daemon or clipboard.
"""
import hashlib
import json
import os
from pathlib import Path
import socketserver
import subprocess
import sys
import tempfile
import threading
import time
from gi.repository import Gio, GLib

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'crates/native-ui/linux'))
from i18n import Translator


def wait(predicate, timeout=8):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        while GLib.MainContext.default().pending():
            GLib.MainContext.default().iteration(False)
        if predicate():
            return
        time.sleep(.02)
    raise AssertionError('panel cache did not receive current menu properties')


def main():
    state = dict(send=True, receive=True)
    available = True
    quit_requested = threading.Event()

    class Api(socketserver.StreamRequestHandler):
        def handle(self):
            action = json.loads(self.rfile.read())['action']
            if action['command'] == 'set_directions':
                state.update({k: v for k, v in action.items() if k != 'command'})
            answer = dict(type='status', status=dict(settings=state.copy())) if available else dict(type='error')
            if action['command'] == 'quit':
                quit_requested.set()
                answer = {'type': 'done', 'message': 'Agent is stopping'}
            self.wfile.write(json.dumps(answer).encode())

    with tempfile.TemporaryDirectory() as directory:
        path = directory + '/control.sock'
        server = socketserver.UnixStreamServer(path, Api)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        child = subprocess.Popen([sys.executable, str(ROOT / 'crates/native-ui/linux/tray.py'),
                                  path, '/bin/true', str(os.getpid())], env={**os.environ, 'LANG': 'C.UTF-8'})
        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        name = 'org.shuttli.Tray.profile_' + hashlib.sha256(path.encode()).hexdigest()[:16]
        interface = 'com.canonical.dbusmenu'

        def call(method, signature, args):
            return bus.call_sync(name, '/Menu', interface, method, GLib.Variant(signature, args),
                                 None, Gio.DBusCallFlags.NONE, 3000, None).unpack()

        def ready():
            try:
                return call('GetProperty', '(is)', (20, 'enabled'))[0]
            except GLib.Error:
                return False

        cache = {}
        active = True
        dirty = False
        updates = []

        def properties():
            return dict(call('GetGroupProperties', '(aias)', ([], []))[0])

        def signal(_bus, _sender, _path, _interface, member, params):
            nonlocal dirty
            if member == 'ItemsPropertiesUpdated':
                updates.append(member)
                if not active:
                    dirty = True
                    return
                changed, removed = params.unpack()
                for ident, props in changed:
                    cache.setdefault(ident, {}).update(props)
                for ident, keys in removed:
                    for key in keys:
                        cache[ident].pop(key, None)
            elif member == 'LayoutUpdated':
                # GNOME only fetches structural properties for existing items.
                layout = call('GetLayout', '(iias)', (0, -1, ['type', 'children-display']))
                for ident, props, _children in layout[1][2]:
                    cache.setdefault(ident, {}).update(props)

        subscription = bus.signal_subscribe(name, interface, None, '/Menu', None,
                                            Gio.DBusSignalFlags.NONE, signal)
        try:
            wait(ready)
            cache.update(properties())
            for send, receive in [(False, True), (True, False), (False, False), (True, True)]:
                state.update(send=send, receive=receive)
                wait(lambda: (cache[20]['toggle-state'], cache[21]['toggle-state']) == (int(send), int(receive)))
                assert cache[30]['enabled'] == send
            print('PASS open panel receives all direction changes without refetching values', flush=True)

            active = False
            dirty = False
            state.update(send=False, receive=False)
            wait(lambda: dirty)
            active = True
            cache.update(properties())
            assert cache[20]['toggle-state'] == cache[21]['toggle-state'] == 0
            call('Event', '(isvu)', (20, 'clicked', GLib.Variant('i', 0), 0))
            wait(lambda: state['send'] and cache[20]['toggle-state'] == 1 and cache[30]['enabled'])
            print('PASS closed panel invalidates cache; menu click returns confirmed state', flush=True)

            translator = Translator(directory, environ={'LANG': 'en'})
            translator.select('de')
            wait(lambda: cache[10]['label'] == translator('tray.open'))
            available = False
            wait(lambda: not cache[20]['enabled'] and not cache[21]['enabled'] and not cache[30]['enabled'])
            assert cache[10]['enabled']
            available = True
            wait(lambda: cache[20]['enabled'] and cache[20]['toggle-state'] == 1)
            count = len(updates)
            until = time.monotonic() + 3
            wait(lambda: time.monotonic() >= until, timeout=4)
            assert len(updates) == count, 'unchanged polls must not broadcast menu updates'
            print('PASS locale, unavailable/recovery and unchanged-poll behavior', flush=True)
            assert cache[40]['label'] == translator('tray.quit')
            call('Event', '(isvu)', (40, 'clicked', GLib.Variant('i', 0), 0))
            wait(lambda: quit_requested.is_set() and child.poll() == 0)
            print('PASS localized Quit menu invokes the lifecycle API and exits the tray', flush=True)
        finally:
            bus.signal_unsubscribe(subscription)
            child.terminate()
            child.wait(timeout=5)
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)


if __name__ == '__main__':
    main()
