#!/usr/bin/env python3
"""Small Gio-only StatusNotifier/DBusMenu client; GTK is loaded on demand.

All synchronization operations go through the public local API. No clipboard,
database, peer transport, or core implementation is imported by this process.
"""
from brand import NAME
from concurrent.futures import ThreadPoolExecutor
import hashlib
from control_client import request as control_request
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from i18n import Translator

from gi.repository import Gio, GLib
from tray_model import TrayState, icon_pixels

ITEM = 'org.kde.StatusNotifierItem'
WATCHER = 'org.kde.StatusNotifierWatcher'
MENU = 'com.canonical.dbusmenu'
ITEM_PATH = '/StatusNotifierItem'
MENU_PATH = '/Menu'
ITEM_XML = '''<node><interface name="org.kde.StatusNotifierItem">
<property name="Category" type="s" access="read"/>
<property name="Id" type="s" access="read"/>
<property name="Title" type="s" access="read"/>
<property name="Status" type="s" access="read"/>
<property name="WindowId" type="u" access="read"/>
<property name="IconName" type="s" access="read"/>
<property name="IconPixmap" type="a(iiay)" access="read"/>
<property name="OverlayIconName" type="s" access="read"/>
<property name="OverlayIconPixmap" type="a(iiay)" access="read"/>
<property name="AttentionIconName" type="s" access="read"/>
<property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
<property name="AttentionMovieName" type="s" access="read"/>
<property name="ToolTip" type="(sa(iiay)ss)" access="read"/>
<property name="ItemIsMenu" type="b" access="read"/>
<property name="Menu" type="o" access="read"/>
<method name="ContextMenu"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
<method name="Activate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
<method name="SecondaryActivate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
<method name="Scroll"><arg type="i" direction="in"/><arg type="s" direction="in"/></method>
<signal name="NewIcon"/><signal name="NewToolTip"/><signal name="NewTitle"/>
<signal name="NewStatus"><arg type="s"/></signal>
</interface></node>'''
MENU_XML = '''<node><interface name="com.canonical.dbusmenu">
<property name="Version" type="u" access="read"/>
<property name="TextDirection" type="s" access="read"/>
<property name="Status" type="s" access="read"/>
<property name="IconThemePath" type="as" access="read"/>
<method name="GetLayout"><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/><arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/></method>
<method name="GetGroupProperties"><arg type="ai" direction="in"/><arg type="as" direction="in"/><arg type="a(ia{sv})" direction="out"/></method>
<method name="GetProperty"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="out"/></method>
<method name="Event"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/><arg type="u" direction="in"/></method>
<method name="EventGroup"><arg type="a(isvu)" direction="in"/><arg type="ai" direction="out"/></method>
<method name="AboutToShow"><arg type="i" direction="in"/><arg type="b" direction="out"/></method>
<method name="AboutToShowGroup"><arg type="ai" direction="in"/><arg type="ai" direction="out"/><arg type="ai" direction="out"/></method>
<signal name="LayoutUpdated"><arg type="u"/><arg type="i"/></signal>
<signal name="ItemsPropertiesUpdated"><arg type="a(ia{sv})"/><arg type="a(ias)"/></signal>
</interface></node>'''


class Tray:
    def __init__(self, path, executable, parent):
        self.path, self.executable, self.parent = path, executable, parent
        self.locale = Translator(Path(path).parent)
        self.locale_changed = False
        self.state = TrayState(locale=self.locale)
        self.error = ''
        self.revision = 1
        self.busy = False
        self.pending_directions = {}
        self.pending_send = False
        self.pending_quit = False
        self.pool = ThreadPoolExecutor(max_workers=1)
        self.children = []
        self.last_open = 0
        self.loop = GLib.MainLoop()
        self.connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        self.identifier = 'shuttli-' + hashlib.sha256(path.encode()).hexdigest()[:16]
        self.name = 'org.shuttli.Tray.profile_' + self.identifier.split('-')[1]
        self.connection.register_object(ITEM_PATH, Gio.DBusNodeInfo.new_for_xml(ITEM_XML).interfaces[0],
                                        self.item_method, self.item_property, None)
        self.connection.register_object(MENU_PATH, Gio.DBusNodeInfo.new_for_xml(MENU_XML).interfaces[0],
                                        self.menu_method, self.menu_property, None)
        self.owner = Gio.bus_own_name_on_connection(self.connection, self.name, Gio.BusNameOwnerFlags.NONE,
                                                   self.name_acquired, lambda *_: self.loop.quit())
        self.watcher = None

    def name_acquired(self, *_):
        # Keep waiting if the desktop panel starts after the clipboard daemon.
        self.watcher = Gio.bus_watch_name_on_connection(self.connection, WATCHER, Gio.BusNameWatcherFlags.NONE,
                                                        self.register, lambda *_: None)
        self.update()
        GLib.timeout_add_seconds(2, self.update)

    def register(self, *_):
        def completed(connection, result):
            try:
                connection.call_finish(result)
            except GLib.Error as error:
                print(NAME + ' tray registration:', error.message, file=sys.stderr)
        self.connection.call(WATCHER, '/StatusNotifierWatcher', WATCHER, 'RegisterStatusNotifierItem',
                             GLib.Variant('(s)', (self.name,)), None, Gio.DBusCallFlags.NONE, 3000, None, completed)

    def request(self, action):
        return control_request(self.path, action, timeout=3, max_response=1024 * 1024)

    def update(self, action=None):
        if os.getppid() != self.parent:
            self.loop.quit()
            return False
        if self.locale.reload():
            self.locale_changed = True
        self.children = [child for child in self.children if child.poll() is None]
        if self.busy:
            # Bounded/coalesced explicit intents survive a concurrent status poll.
            if action and action['command'] == 'quit':
                self.pending_quit = True
                self.pending_directions.clear()
                self.pending_send = False
            elif action and action['command'] == 'set_directions':
                self.pending_directions.update({k: v for k, v in action.items() if k != 'command'})
            elif action and action['command'] == 'send':
                self.pending_send = True
            return True
        self.busy = True
        def work():
            error = ''
            try:
                if action:
                    answer = self.request(action)
                    if answer['type'] == 'error':
                        error = answer['message']
                    elif action['command'] == 'quit':
                        GLib.idle_add(self.loop.quit)
                        return
                answer = self.request({'command': 'status'})
            except Exception:
                answer = {'type': 'error'}
                error = self.locale('common.unavailable')
            GLib.idle_add(self.updated, answer, error)
        self.pool.submit(work)
        return True

    def updated(self, answer, error):
        self.busy = False
        if answer.get('type') == 'stopped':
            self.loop.quit()
            return False
        state = TrayState.from_answer(answer, self.locale)
        changed = state != self.state or error != self.error or self.locale_changed
        self.locale_changed = False
        self.state, self.error = state, error
        if self.pending_quit:
            self.pending_quit = False
            self.pending_directions.clear()
            self.pending_send = False
            self.update({'command': 'quit'})
        elif self.pending_directions:
            action = {'command': 'set_directions', **self.pending_directions}
            self.pending_directions.clear()
            self.update(action)
        elif self.pending_send:
            self.pending_send = False
            self.update({'command': 'send'})
        if not changed:
            return False
        self.revision = self.revision % (2**32-1) + 1
        for signal in ('NewIcon', 'NewToolTip', 'NewTitle'):
            self.connection.emit_signal(None, ITEM_PATH, ITEM, signal, None)
        self.connection.emit_signal(None, ITEM_PATH, 'org.freedesktop.DBus.Properties', 'PropertiesChanged',
                                    GLib.Variant('(sa{sv}as)', (ITEM, {
                                        'Title': self.item_value('Title'),
                                        'IconPixmap': self.item_value('IconPixmap'),
                                        'ToolTip': self.item_value('ToolTip'),
                                    }, [])))
        # Existing item IDs and structure stay fixed. Panels such as GNOME only
        # fetch type/children-display on LayoutUpdated and cache other values.
        # Notify property changes explicitly, including when the menu is closed.
        self.connection.emit_signal(None, MENU_PATH, MENU, 'ItemsPropertiesUpdated',
                                    GLib.Variant('(a(ia{sv})a(ias))', (
                                        [(item[0], self.properties(item[0])) for item in self.state.menu()], [])))
        return False

    def pixmaps(self):
        return [(size, size, icon_pixels(self.state, size)) for size in (16, 24, 32)]

    def item_value(self, name):
        fixed = {'Category': ('s', 'ApplicationStatus'), 'Id': ('s', self.identifier),
                 'Title': ('s', self.state.title), 'Status': ('s', 'Active'), 'WindowId': ('u', 0),
                 'ItemIsMenu': ('b', True), 'Menu': ('o', MENU_PATH)}
        if name in fixed:
            return GLib.Variant(*fixed[name])
        if name == 'IconPixmap':
            return GLib.Variant('a(iiay)', self.pixmaps())
        if name.endswith('Pixmap'):
            return GLib.Variant('a(iiay)', [])
        if name == 'ToolTip':
            return GLib.Variant('(sa(iiay)ss)', ('', [], self.state.title,
                                                self.locale.message(self.error) if self.error else self.locale('tray.hint')))
        return GLib.Variant('s', '')

    def item_property(self, connection, sender, path, interface, name):
        return self.item_value(name)

    def item_method(self, connection, sender, path, interface, method, params, invocation):
        # ItemIsMenu and Menu ask the panel to render its native DBusMenu popup.
        # Middle-click remains a convenient direct open action.
        if method == 'SecondaryActivate':
            self.open_window()
        invocation.return_value(GLib.Variant('()', ()))

    def menu_property(self, connection, sender, path, interface, name):
        return {'Version': GLib.Variant('u', 3), 'TextDirection': GLib.Variant('s', 'ltr'),
                'Status': GLib.Variant('s', 'normal'), 'IconThemePath': GLib.Variant('as', [])}[name]

    def properties(self, ident, names=()):
        if ident == 0:
            props = {'children-display': GLib.Variant('s', 'submenu')}
        else:
            item = next((item for item in self.state.menu() if item[0] == ident), None)
            if not item:
                raise ValueError('unknown menu item')
            _, label, enabled, checked = item
            props = {'visible': GLib.Variant('b', True), 'enabled': GLib.Variant('b', enabled)}
            if label is None:
                props['type'] = GLib.Variant('s', 'separator')
            else:
                props['label'] = GLib.Variant('s', label)
            if checked is not None:
                props.update({'toggle-type': GLib.Variant('s', 'checkmark'), 'toggle-state': GLib.Variant('i', int(checked))})
        return {k: v for k, v in props.items() if not names or k in names}

    def layout(self, ident, depth, names):
        children = []
        if ident == 0 and depth != 0:
            children = [GLib.Variant('(ia{sv}av)', (item[0], self.properties(item[0], names), [])) for item in self.state.menu()]
        return (ident, self.properties(ident, names), children)

    def click(self, ident, event):
        if event != 'clicked':
            return
        if ident == 10:
            self.open_window()
        else:
            action = self.state.action(ident)
            if action:
                self.update(action)

    def open_window(self):
        if time.monotonic() - self.last_open < .5:
            return
        self.last_open = time.monotonic()
        self.children.append(subprocess.Popen([self.executable, 'ui'], stdin=subprocess.DEVNULL,
                                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))

    def menu_method(self, connection, sender, path, interface, method, params, invocation):
        args = params.unpack()
        try:
            if method == 'GetLayout':
                value = GLib.Variant('(u(ia{sv}av))', (self.revision, self.layout(*args)))
            elif method == 'GetGroupProperties':
                ids, names = args
                value = GLib.Variant('(a(ia{sv}))', ([(i, self.properties(i, names)) for i in (ids or [0, *[x[0] for x in self.state.menu()]])],))
            elif method == 'GetProperty':
                value = GLib.Variant('(v)', (self.properties(args[0])[args[1]],))
            elif method == 'Event':
                self.click(args[0], args[1])
                value = GLib.Variant('()', ())
            elif method == 'EventGroup':
                for ident, event, _, _ in args[0]:
                    self.click(ident, event)
                value = GLib.Variant('(ai)', ([],))
            elif method == 'AboutToShow':
                self.update()
                value = GLib.Variant('(b)', (False,))
            elif method == 'AboutToShowGroup':
                self.update()
                value = GLib.Variant('(aiai)', ([], []))
            else:
                raise ValueError('unknown method')
            invocation.return_value(value)
        except (ValueError, KeyError, TypeError) as error:
            invocation.return_dbus_error('com.canonical.dbusmenu.Error', str(error))

    def run(self):
        try:
            self.loop.run()
        finally:
            self.pool.shutdown(wait=False, cancel_futures=True)
            Gio.bus_unown_name(self.owner)
            if self.watcher is not None:
                Gio.bus_unwatch_name(self.watcher)


if __name__ == '__main__':
    Tray(sys.argv[1], sys.argv[2], int(sys.argv[3])).run()
