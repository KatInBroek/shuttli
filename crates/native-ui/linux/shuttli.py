#!/usr/bin/env python3
"""On-demand native GTK presentation. All operations use the public control API."""
from brand import NAME, VERSION
import base64
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
import hashlib
from control_client import request as control_request
import json
from pathlib import Path
import sys
from i18n import Translator, LANGUAGES
from ui_model import DEFAULT_POLICY, group_history, peer_name
import gi
gi.require_version('Gtk', '4.0')
from gi.repository import Gtk, Gdk, GLib, Pango

ASSETS = Path(__file__).resolve().parent


class Window(Gtk.ApplicationWindow):
    def __init__(self, app, path):
        super().__init__(application=app, title=NAME, default_width=850, default_height=620)
        self.set_size_request(720, 520)
        titlebar = Gtk.Box()
        titlebar.add_css_class('compact-titlebar')
        self.set_titlebar(titlebar)
        self.add_css_class('shuttli')
        self.path = path
        self.locale = Translator(Path(path).parent)
        self.pool = ThreadPoolExecutor(max_workers=1)
        self.page = 'status'
        self.generation = 0
        self.settings = None
        self.peers = []
        self.history_offset = 0
        self.previews = []
        self.setting_toggles = {}
        self.polling = False
        self.service_available = None
        self.last_sequence = None
        self.last_policy_revision = None
        self.last_device_counts = None
        self.closed = False
        self.connect('close-request', self.closing)
        css = Gtk.CssProvider()
        css.load_from_path(str(ASSETS / 'style.css'))
        Gtk.StyleContext.add_provider_for_display(self.get_display(), css, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        root = Gtk.Box()
        self.set_child(root)
        sidebar = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=4)
        sidebar.set_size_request(195, -1)
        sidebar.add_css_class('sidebar')
        root.append(sidebar)
        brand = Gtk.Box(spacing=10)
        brand.add_css_class('brand')
        logo = Gtk.Image.new_from_file(str(ASSETS / 'mark.svg' if (ASSETS / 'mark.svg').exists() else ASSETS.parent / 'assets/mark.svg'))
        logo.set_pixel_size(24)
        brand.append(logo)
        self.label(brand, NAME)
        sidebar.append(brand)
        self.nav_buttons = []
        for page, icon in [('status', 'object-select-symbolic'), ('devices', 'computer-symbolic'), ('history', 'document-open-recent-symbolic'), ('settings', 'emblem-system-symbolic')]:
            button = Gtk.Button()
            button.add_css_class('nav')
            box = Gtk.Box(spacing=12)
            box.append(Gtk.Image.new_from_icon_name(icon))
            label = self.label(box, self.t('nav.' + page))
            button.set_child(box)
            button.connect('clicked', lambda _, page=page: self.navigate(page))
            sidebar.append(button)
            self.nav_buttons.append((page, button, label))
        sidebar.append(Gtk.Box(vexpand=True))
        summary = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
        summary.add_css_class('sidebar-status')
        sidebar.append(summary)
        self.summary_directions = self.label(summary, self.t('common.unavailable'))
        self.summary_mode = self.label(summary, '', 'muted')
        self.summary_directions.set_max_width_chars(22)
        self.summary_mode.set_max_width_chars(22)
        main = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, hexpand=True)
        root.append(main)
        header = Gtk.Box(spacing=8)
        header.add_css_class('page-header')
        self.back = self.button(header, '‹', lambda: self.navigate('devices' if self.page == 'device' else 'history'))
        self.back.set_visible(False)
        self.page_title = self.label(header, '', 'page-title')
        self.page_title.set_hexpand(True)
        self.page_title.set_xalign(.5)
        close = Gtk.Button.new_from_icon_name('window-close-symbolic')
        close.add_css_class('flat')
        close.set_tooltip_text(self.t('status.close_hint'))
        close.connect('clicked', lambda *_: self.close())
        header.append(close)
        handle = Gtk.WindowHandle()
        handle.set_child(header)
        main.append(handle)
        self.message = Gtk.Label(wrap=True, xalign=0)
        self.message.add_css_class('banner')
        self.message.set_visible(False)
        main.append(self.message)
        self.scroll = Gtk.ScrolledWindow(vexpand=True, hexpand=True)
        self.scroll.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        main.append(self.scroll)
        self.content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12)
        self.content.add_css_class('page-content')
        self.scroll.set_child(self.content)
        theme = Gtk.Settings.get_default()
        theme.connect('notify::gtk-application-prefer-dark-theme', self.theme_changed)
        theme.connect('notify::gtk-theme-name', self.theme_changed)
        self.theme_changed(theme)
        self.navigate('status')
        self.poll_source = GLib.timeout_add(1500, self.poll)
        self.poll()

    def theme_changed(self, settings, *_):
        dark = settings.get_property('gtk-application-prefer-dark-theme') or 'dark' in settings.get_property('gtk-theme-name').lower()
        (self.add_css_class if dark else self.remove_css_class)('dark')

    def t(self, key, **values):
        return self.locale(key, **values)

    def closing(self, *_):
        self.closed = True
        GLib.source_remove(self.poll_source)
        self.close_previews()
        self.pool.shutdown(wait=False, cancel_futures=True)
        return False

    def label(self, box, text, style=None):
        label = Gtk.Label(label=str(text), wrap=True, xalign=0, selectable=False)
        label.set_wrap_mode(Pango.WrapMode.WORD_CHAR)
        label.set_max_width_chars(65)
        if style:
            label.add_css_class(style)
        box.append(label)
        return label

    def button(self, box, text, fn, style=None):
        button = Gtk.Button(label=text)
        button.set_valign(Gtk.Align.CENTER)
        button.get_child().set_wrap(True)
        if style:
            button.add_css_class(style)
        button.connect('clicked', lambda *_: fn())
        box.append(button)
        return button

    def group(self, title=None):
        if title:
            self.label(self.content, title, 'section-title')
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        box.add_css_class('group')
        self.content.append(box)
        return box

    def row(self, box, title, subtitle=None, control=None):
        row = Gtk.Box(spacing=14)
        row.add_css_class('row')
        labels = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=5, hexpand=True)
        self.label(labels, title)
        if subtitle:
            self.label(labels, subtitle, 'muted')
        row.append(labels)
        if control:
            control.set_valign(Gtk.Align.CENTER)
            row.append(control)
        box.append(row)
        return row

    def notice(self, text, error=False):
        self.message.set_text(text)
        self.message.set_visible(bool(text))
        (self.message.add_css_class if error else self.message.remove_css_class)('error')

    def request(self, action):
        if action['command'] == '_setting':
            current = self.request({'command': 'settings'})
            if current['type'] == 'error':
                return current
            expected = current['settings']
            settings = dict(expected)
            settings.update(action['values'])
            return self.request({'command': 'configure', 'expected': expected, 'settings': settings})
        if action['command'] == '_history':
            entries = []
            for offset in range(0, 10000, 100):
                answer = self.request({'command': 'history', 'offset': offset, 'limit': 100})
                if answer['type'] == 'error':
                    return answer
                entries.extend(answer['entries'])
                if len(answer['entries']) < 100:
                    break
            return {'type': 'history', 'entries': entries}
        return control_request(self.path, action)

    def call(self, action, callback=None, *, quiet=False, guarded=True):
        generation = self.generation
        def work():
            try:
                answer = self.request(action)
            except Exception as error:
                answer = {'type': 'error', 'message': str(error)}
            GLib.idle_add(done, answer)
        def done(answer):
            if self.closed or (guarded and generation != self.generation):
                return False
            if not quiet or answer['type'] == 'error':
                detail = self.locale.message(answer.get('message', ''))
                self.notice(self.t('common.error', detail=detail) if answer['type'] == 'error' else detail, answer['type'] == 'error')
            if callback:
                callback(answer)
            return False
        self.pool.submit(work)

    def update_summary(self):
        if self.settings:
            self.summary_directions.set_text(self.t('status.directions', send=self.t('common.on' if self.settings['send'] else 'common.off'), receive=self.t('common.on' if self.settings['receive'] else 'common.off')))
            self.summary_mode.set_text(self.t('ui.automatic' if self.settings['automatic'] else 'ui.manual'))

    def poll(self):
        if self.closed:
            return False
        if self.locale.reload():
            self.update_summary()
            for page, _, label in self.nav_buttons:
                label.set_text(self.t('nav.' + page))
            self.navigate(self.page if self.page in ('status', 'devices', 'history', 'settings') else 'history')
        if not self.polling:
            self.polling = True
            def updated(answer):
                self.polling = False
                if answer['type'] == 'stopped':
                    self.close()
                    return
                if answer['type'] != 'status':
                    self.service_available = False
                    self.summary_directions.set_text(self.t('common.unavailable'))
                    self.summary_mode.set_text('')
                    self.content.set_sensitive(False)
                    return
                recovered = self.service_available is False
                self.service_available = True
                self.content.set_sensitive(True)
                if recovered:
                    self.navigate(self.page)
                status = answer['status']
                self.settings = status['settings']
                self.update_summary()
                revision_changed = self.last_policy_revision is not None and self.last_policy_revision != status['policy_revision']
                sequence_changed = self.last_sequence is not None and self.last_sequence != status['sequence']
                counts_changed = self.last_device_counts != status.get('devices')
                self.last_device_counts = status.get('devices')
                self.last_sequence = status['sequence']
                self.last_policy_revision = status['policy_revision']
                for key, (widget, handler) in self.setting_toggles.items():
                    widget.handler_block(handler)
                    widget.set_active(self.settings[key])
                    widget.handler_unblock(handler)
                if (revision_changed and self.page in ('status', 'devices', 'device')) or (sequence_changed and self.page in ('history', 'detail', 'status')) or (counts_changed and self.page == 'status'):
                    self.navigate(self.page)
            self.call({'command': 'status'}, updated, quiet=True, guarded=False)
        return True

    def navigate(self, page):
        self.page = page
        self.generation += 1
        self.setting_toggles = {}
        for name, button, _ in self.nav_buttons:
            selected = name == page or (name == 'devices' and page == 'device') or (name == 'history' and page == 'detail')
            (button.add_css_class if selected else button.remove_css_class)('selected')
        self.page_title.set_text(self.t('ui.device_detail' if page == 'device' else 'ui.history_detail' if page == 'detail' else 'nav.' + page))
        self.back.set_visible(page in ('device', 'detail'))
        self.notice('')
        while self.content.get_first_child():
            self.content.remove(self.content.get_first_child())
        self.scroll.get_vadjustment().set_value(0)
        getattr(self, {'settings': 'preferences', 'device': 'device_detail', 'detail': 'history_detail'}.get(page, page))()

    def toggle(self, box, key, subtitle=None, *, policy=None, peer=None):
        value = (self.settings if policy is None else policy)[key]
        switch = Gtk.Switch(active=value)
        def changed(widget, _):
            desired = widget.get_active()
            if peer and key == 'send' and desired:
                widget.handler_block(handler)
                widget.set_active(False)
                widget.handler_unblock(handler)
                self.allow_device(peer, policy)
                return
            widget.set_sensitive(False)
            if policy is not None:
                new = dict(policy, **{key: desired})
                action = {'command': 'peer', 'id': peer['id'], 'expected': dict(policy), 'policy': new}
            elif key in ('send', 'receive'):
                action = {'command': 'set_directions', key: desired}
            else:
                action = {'command': '_setting', 'values': {key: desired}}
            def completed(answer):
                widget.set_sensitive(True)
                actual = desired if answer['type'] != 'error' else not desired
                widget.handler_block(handler)
                widget.set_active(actual)
                widget.handler_unblock(handler)
                if policy is not None:
                    policy[key] = actual
                elif answer['type'] == 'settings':
                    self.settings = answer['settings']
            self.call(action, completed)
        handler = switch.connect('notify::active', changed)
        if policy is None:
            self.setting_toggles[key] = (switch, handler)
        self.row(box, self.t('setting.' + key), subtitle, switch)
        return switch

    def status(self):
        def render(answer):
            if answer['type'] != 'status':
                self.label(self.content, self.t('common.unavailable'), 'empty')
                self.button(self.content, self.t('ui.retry'), lambda: self.navigate('status'))
                return
            status = answer['status']
            self.settings = status['settings']
            hero = Gtk.Box(spacing=18)
            self.content.append(hero)
            hero_icon = self.label(hero, '↕' if self.settings['send'] or self.settings['receive'] else 'Ⅱ', 'hero-icon')
            hero_icon.set_size_request(32, 32)
            hero_icon.set_xalign(.5)
            hero_icon.set_wrap(False)
            headings = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=5)
            hero.append(headings)
            state = {(True, True): 'tray.both', (True, False): 'tray.send_only', (False, True): 'tray.receive_only', (False, False): 'tray.paused'}[(self.settings['send'], self.settings['receive'])]
            self.label(headings, self.t(state), 'hero-title')
            self.label(headings, self.t('ui.automatic' if self.settings['automatic'] else 'ui.manual'), 'muted')
            counts = status.get('devices', {})
            group = self.group(self.t('nav.devices'))
            statistics = Gtk.Box(spacing=18, homogeneous=True)
            statistics.add_css_class('row')
            group.append(statistics)
            for key, label in [('discovered', 'status.discovered'), ('send', 'status.allowed_send'), ('receive', 'status.allowed_receive')]:
                metric = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=6)
                statistics.append(metric)
                self.label(metric, self.t(label), 'muted')
                self.label(metric, str(counts.get(key, 0)), 'hero-title')
            self.label(self.content, self.t('status.permissions_hint'), 'muted')
            group = self.group(self.t('ui.directions'))
            self.toggle(group, 'send', self.t('ui.send_hint'))
            self.toggle(group, 'receive', self.t('ui.receive_hint'))
            group = self.group(self.t('ui.sync_mode'))
            self.toggle(group, 'automatic', self.t('ui.automatic_hint'))
            button = Gtk.Button(label=self.t('action.send'))
            button.add_css_class('suggested-action')
            button.set_sensitive(status['clipboard_available'] and counts.get('send', 0) > 0)
            button.connect('clicked', lambda *_: self.send_now(button))
            self.row(group, self.t('tray.send'), self.t('ui.manual_hint'), button)
            activity = Gtk.Box()
            self.content.append(activity)
            self.label(activity, self.t('ui.recent_activity'), 'section-title').set_hexpand(True)
            self.button(activity, self.t('nav.history'), lambda: self.navigate('history'), 'link')
            recent = self.group()
            def history_ready(a):
                if a['type'] == 'history':
                    groups = group_history(a['entries'])[:3]
                    if not groups:
                        self.row(recent, self.t('history.empty'))
                    for entry in groups:
                        self.history_row(recent, entry)
            def peers_ready(a):
                if a['type'] == 'devices':
                    self.peers = a['devices']
                self.call({'command': '_history'}, history_ready, quiet=True)
            self.call({'command': 'devices'}, peers_ready, quiet=True)
            details = Gtk.Expander(label=self.t('status.details'))
            self.content.append(details)
            box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
            details.set_child(box)
            self.label(box, self.t('status.fingerprint') + '\n' + status['device']).set_selectable(True)
            self.label(box, status['clipboard'], 'muted')
            self.label(box, self.t('status.clipboard', available=self.t('common.yes' if status['clipboard_available'] else 'common.no')))
            if status['last_error']:
                self.label(box, self.locale.message(status['last_error']), 'error')
            self.label(self.content, self.t('status.close_hint'), 'muted')
        self.call({'command': 'status'}, render)

    def send_now(self, button):
        button.set_sensitive(False)
        self.call({'command': 'send'}, lambda _: button.set_sensitive(True))

    def devices(self):
        def render(answer):
            if answer['type'] != 'devices':
                return
            self.peers = answer['devices']
            self.settings = answer['settings']
            self.label(self.content, self.t('ui.devices_hint'), 'muted')
            self.button(self.content, self.t('action.refresh_discovery'), lambda: self.call({'command': 'refresh'}, lambda _: self.navigate('devices')), 'link')
            group = self.group(self.t('ui.tailnet'))
            if not self.peers:
                self.row(group, self.t('devices.empty'))
            for peer in self.peers:
                policy = self.settings['peers'].get(peer['id'], DEFAULT_POLICY)
                button = Gtk.Button()
                button.add_css_class('row-button')
                box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
                button.set_child(box)
                state = self.t('devices.online' if peer['online'] else 'devices.offline')
                title = peer['name'] + '   ·   ' + state
                directions = self.t('status.directions', send=self.t('common.on' if policy['send'] else 'common.off'), receive=self.t('common.on' if policy['receive'] else 'common.off'))
                subtitle = peer['id'][:12] + ' · ' + directions
                if not peer.get('capabilities', {}).get('accept_live_offer', True):
                    subtitle += '\n' + self.t('devices.history_pull')
                self.row(box, title, subtitle, Gtk.Image.new_from_icon_name('go-next-symbolic'))
                button.connect('clicked', lambda _, peer=peer: self.open_device(peer))
                group.append(button)
            self.label(self.content, self.t('devices.help'), 'muted')
        self.call({'command': 'devices'}, render)

    def open_device(self, peer):
        self.selected_peer = peer
        self.navigate('device')

    def device_detail(self):
        def render(answer):
            if answer['type'] != 'devices':
                return
            peer = next((d for d in answer['devices'] if d['id'] == self.selected_peer['id']), self.selected_peer)
            self.settings = answer['settings']
            policy = dict(DEFAULT_POLICY, **self.settings['peers'].get(peer['id'], {}))
            self.label(self.content, peer['name'], 'hero-title')
            self.label(self.content, peer['address'] + ' · ' + self.t('devices.online' if peer['online'] else 'devices.offline'), 'muted')
            self.row(self.group(), self.t('status.fingerprint'), peer['id'])
            group = self.group(self.t('ui.directions'))
            self.toggle(group, 'send', self.t('ui.send_hint'), policy=policy, peer=peer)
            self.toggle(group, 'receive', self.t('ui.receive_hint'), policy=policy, peer=peer)
            group = self.group(self.t('ui.content_types'))
            for key in ('text', 'png', 'quiet'):
                self.toggle(group, key, policy=policy, peer=peer)
            modes = ['inherit', 'off', 'status', 'content']
            chooser = Gtk.DropDown.new_from_strings([self.t('history.mode.' + mode) for mode in modes])
            chooser.set_selected(modes.index(policy.get('history') or 'inherit'))
            def changed(widget, *_):
                new = dict(policy, history=None if widget.get_selected() == 0 else modes[widget.get_selected()])
                self.call({'command': 'peer', 'id': peer['id'], 'expected': dict(policy), 'policy': new}, lambda _: self.navigate('device'))
            chooser.connect('notify::selected', changed)
            self.row(self.group(), self.t('devices.history'), control=chooser)
        self.call({'command': 'devices'}, render)

    def dialog(self, title, action_label, build, confirm):
        dialog = Gtk.Window(title=title, transient_for=self, modal=True, default_width=430)
        dialog.set_resizable(False)
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=16)
        for side in ('top', 'bottom', 'start', 'end'):
            getattr(box, 'set_margin_' + side)(24)
        dialog.set_child(box)
        self.label(box, title, 'hero-title')
        ready = build(box)
        actions = Gtk.Box(spacing=10, halign=Gtk.Align.END)
        box.append(actions)
        self.button(actions, self.t('ui.cancel'), dialog.close)
        button = self.button(actions, action_label, lambda: (dialog.close(), confirm()), 'suggested-action')
        if ready is not None:
            button.set_sensitive(False)
            ready.connect('toggled', lambda widget: button.set_sensitive(widget.get_active()))
        dialog.present()

    def allow_device(self, peer, policy):
        def build(box):
            self.label(box, self.t('ui.allow_hint', name=peer['name']))
            self.label(box, peer['id'], 'preview').set_selectable(True)
            check = Gtk.CheckButton()
            check.set_child(Gtk.Label(label=self.t('ui.fingerprint_checked'), wrap=True, xalign=0))
            box.append(check)
            return check
        self.dialog(self.t('ui.allow_title'), self.t('ui.allow'), build,
                    lambda: self.call({'command': 'peer', 'id': peer['id'], 'expected': dict(policy), 'policy': dict(policy, send=True)}, lambda _: self.navigate('device')))

    def history(self):
        def peers_ready(answer):
            if answer['type'] == 'devices':
                self.peers = answer['devices']
                self.settings = answer['settings']
            self.call({'command': '_history'}, render)
        def render(answer):
            if answer['type'] != 'history':
                return
            self.label(self.content, self.t('ui.session_history'), 'muted')
            actions = Gtk.Box(spacing=12)
            self.content.append(actions)
            self.button(actions, self.t('history.refresh'), lambda: self.navigate('history'), 'link')
            self.button(actions, self.t('ui.history_settings'), lambda: self.navigate('settings'), 'link')
            self.button(actions, self.t('ui.clear'), self.clear_history, 'link')
            entries = group_history(answer['entries'])
            self.history_offset = min(self.history_offset, max(0, ((len(entries) - 1) // 20) * 20))
            group = self.group()
            if not entries:
                self.row(group, self.t('history.empty'), self.t('ui.history_empty_hint'))
            for entry in entries[self.history_offset:self.history_offset + 20]:
                self.history_row(group, entry)
            nav = Gtk.Box(spacing=10)
            self.content.append(nav)
            self.label(nav, self.t('ui.entries', count=len(entries))).set_hexpand(True)
            def page(delta):
                self.history_offset = max(0, self.history_offset + delta)
                self.navigate('history')
            self.button(nav, self.t('history.previous'), lambda: page(-20)).set_sensitive(self.history_offset > 0)
            self.button(nav, self.t('history.next'), lambda: page(20)).set_sensitive(self.history_offset + 20 < len(entries))
        self.call({'command': 'devices'}, peers_ready, quiet=True)

    def history_row(self, box, entry):
        button = Gtk.Button()
        button.add_css_class('row-button')
        content = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=7)
        content.add_css_class('row')
        top = Gtk.Box(spacing=8)
        content.append(top)
        direction = '↓' if entry['direction'] == 'receive' else '↑' if entry['direction'] == 'send' else '•'
        self.label(top, direction + '  ' + self.t('format.' + entry['format'])).set_hexpand(True)
        self.label(top, datetime.fromtimestamp(entry['time']).strftime('%H:%M:%S'), 'muted')
        if entry['local']:
            self.label(content, self.t('ui.local_copy'), 'muted')
        if not entry['transfers']:
            self.label(content, self.t('ui.not_sent'), 'muted')
        for transfer in entry['transfers']:
            row = Gtk.Box(spacing=10)
            content.append(row)
            source = self.t('ui.from' if entry['direction'] == 'receive' else 'ui.to', name=peer_name(self.peers, transfer['peer']))
            self.label(row, source, 'muted').set_hexpand(True)
            badge = self.label(row, self.t('state.' + transfer['state']), 'badge')
            badge.add_css_class({'applied': 'success', 'failed': 'error', 'unknown': 'warning', 'sending': 'info', 'receiving': 'info'}.get(transfer['state'], 'muted'))
        self.label(content, self.t('ui.content_size', bytes=entry['bytes']), 'muted')
        button.set_child(content)
        button.connect('clicked', lambda *_: self.open_history(entry))
        box.append(button)

    def open_history(self, entry):
        self.selected_history = entry
        self.navigate('detail')

    def history_detail(self):
        def render(answer):
            if answer['type'] != 'history':
                return
            old = self.selected_history
            entry = next((e for e in group_history(answer['entries']) if e['event'] == old['event'] and e['direction'] == old['direction']), None)
            if entry is None:
                self.label(self.content, self.t('ui.expired'), 'empty')
                return
            self.selected_history = entry
            self.label(self.content, self.t('format.' + entry['format']), 'hero-title')
            self.label(self.content, datetime.fromtimestamp(entry['time']).strftime('%Y-%m-%d %H:%M:%S') + ' · ' + self.t('ui.content_size', bytes=entry['bytes']), 'muted')
            preview = self.group(self.t('history.preview'))
            ident = entry['content_id']
            if ident is None:
                self.row(preview, self.t('ui.content_unavailable'))
            else:
                def show(answer):
                    if answer['type'] != 'preview':
                        self.row(preview, self.t('ui.expired'))
                        return
                    data = base64.b64decode(answer['base64'], validate=True)
                    scroll = Gtk.ScrolledWindow(min_content_height=130, max_content_height=260, propagate_natural_height=True)
                    if answer['format'] == 'text':
                        label = Gtk.Label(label=data.decode('utf-8'), wrap=True, selectable=True, xalign=0, yalign=0)
                        label.set_wrap_mode(Pango.WrapMode.WORD_CHAR)
                        label.add_css_class('preview')
                        scroll.set_child(label)
                    else:
                        texture = Gdk.Texture.new_from_bytes(GLib.Bytes.new(data))
                        picture = Gtk.Picture(paintable=texture, can_shrink=True)
                        picture.set_size_request(-1, 180)
                        scroll.set_child(picture)
                    preview.append(scroll)
                self.call({'command': 'preview', 'id': ident}, show, quiet=True)
            actions = Gtk.FlowBox(selection_mode=Gtk.SelectionMode.NONE, column_spacing=8, row_spacing=8, homogeneous=False)
            actions.set_max_children_per_line(3)
            self.content.append(actions)
            for key, command in [('history.copy', 'copy'), ('history.copy_local', 'local'), ('history.resend', 'resend')]:
                button = Gtk.Button(label=self.t(key))
                button.set_sensitive(ident is not None)
                if command == 'copy':
                    button.add_css_class('suggested-action')
                def clicked(_, command=command):
                    action = {'command': 'copy' if command == 'local' else command, 'id': ident}
                    if command in ('copy', 'local'):
                        action['local_only'] = command == 'local'
                    self.call(action)
                button.connect('clicked', clicked)
                actions.insert(button, -1)
            self.label(self.content, self.t('ui.copy_hint'), 'muted')
            group = self.group(self.t('ui.transfers'))
            if entry['local']:
                self.row(group, self.t('ui.local_copy'), self.t('ui.not_sent') if not entry['transfers'] else None)
            for transfer in entry['transfers']:
                self.row(group, peer_name(self.peers, transfer['peer']) + ' · ' + self.t('state.' + transfer['state']), self.locale.message(transfer['detail']))
            self.label(self.content, self.t('status.receipt_hint'), 'muted')
        self.call({'command': '_history'}, render)

    def close_previews(self):
        for window in self.previews:
            window.close()
        self.previews.clear()

    def clear_history(self):
        def build(box):
            self.label(box, self.t('ui.clear_hint'))
        self.dialog(self.t('history.clear'), self.t('ui.clear'), build,
                    lambda: self.call({'command': 'clear_history'}, lambda _: self.navigate('history')))

    def preferences(self):
        def render(answer):
            if answer['type'] != 'settings':
                return
            self.settings = answer['settings']
            group = self.group(self.t('ui.sync'))
            for key, hint in [('send', 'ui.send_hint'), ('receive', 'ui.receive_hint'), ('automatic', 'ui.automatic_hint'), ('text', None), ('png', None)]:
                self.toggle(group, key, self.t(hint) if hint else None)
            self.toggle(self.group(self.t('setting.notifications')), 'notifications', self.t('ui.notification_hint'))
            group = self.group(self.t('history.retention'))
            modes = ['off', 'status', 'content']
            chooser = Gtk.DropDown.new_from_strings([self.t('history.mode.' + mode) for mode in modes])
            chooser.set_selected(modes.index(self.settings['history']))
            chooser.connect('notify::selected', lambda widget, *_: self.call({'command': '_setting', 'values': {'history': modes[widget.get_selected()]}}, lambda _: self.navigate('settings')))
            self.row(group, self.t('ui.keep'), control=chooser)
            self.row(group, self.t('history.storage_hint'))
            count = Gtk.SpinButton.new_with_range(0, 10000, 1)
            count.set_value(self.settings['history_limit'])
            self.row(group, self.t('history.limit'), self.t('ui.limit_hint'), count)
            apply = Gtk.Button(label=self.t('ui.apply'))
            apply.add_css_class('suggested-action')
            apply.connect('clicked', lambda *_: self.call({'command': '_setting', 'values': {'history_limit': count.get_value_as_int()}}, lambda _: self.navigate('settings')))
            self.row(group, self.t('ui.storage_limits', text=self.settings['history_memory_bytes'] // 1048576, images=self.settings['history_bytes'] // 1048576), control=apply)
            clear = Gtk.Button(label=self.t('ui.clear'))
            clear.connect('clicked', lambda *_: self.clear_history())
            self.row(group, self.t('history.clear'), self.t('ui.clear_hint'), clear)
            group = self.group(self.t('ui.this_computer'))
            start = Gtk.Switch()
            start.set_sensitive(False)
            self.row(group, self.t('ui.start_login'), self.t('status.close_hint'), start)
            def loaded(answer):
                if answer['type'] != 'autostart':
                    return
                start.set_active(answer['status']['state'] == 'enabled')
                start.set_sensitive(True)
                def change(widget, *_):
                    desired = widget.get_active()
                    widget.set_sensitive(False)
                    def saved(answer):
                        widget.handler_block(handler)
                        widget.set_active(answer['status']['state'] == 'enabled' if answer['type'] == 'autostart' else not desired)
                        widget.handler_unblock(handler)
                        widget.set_sensitive(True)
                    self.call({'command': 'autostart', 'enabled': desired}, saved)
                handler = start.connect('notify::active', change)
            self.call({'command': 'autostart', 'enabled': None}, loaded, quiet=True)
            language = Gtk.DropDown.new_from_strings([name or self.t('locale.system') for _, name in LANGUAGES])
            language.set_selected([code for code, _ in LANGUAGES].index(self.locale.preference))
            def changed(widget, *_):
                try:
                    self.locale.select(LANGUAGES[widget.get_selected()][0])
                    self.update_summary()
                    for page, _, label in self.nav_buttons:
                        label.set_text(self.t('nav.' + page))
                    self.navigate('settings')
                except (OSError, ValueError) as error:
                    self.notice(str(error), True)
            language.connect('notify::selected', changed)
            self.row(group, self.t('locale.label'), self.t('locale.help'), language)
            self.row(group, self.t('ui.app_version'), VERSION)
            self.label(self.content, NAME + ' · MIT License', 'muted')
        self.call({'command': 'settings'}, render)


def main():
    profile = hashlib.sha256(sys.argv[1].encode()).hexdigest()[:16]
    app = Gtk.Application(application_id='org.shuttli.Control.profile_' + profile)
    def activate(app):
        window = app.get_active_window() or Window(app, sys.argv[1])
        window.present()
    app.connect('activate', activate)
    app.run([])


if __name__ == '__main__':
    main()
