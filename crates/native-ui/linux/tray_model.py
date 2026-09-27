"""Presentation-only tray state and approved artwork; no synchronization I/O.

The design team can replace these labels/pixels without changing API semantics.
"""
from brand import NAME
from tray_icons import pixels
from dataclasses import dataclass, field
from i18n import Translator

ENGLISH = Translator(environ={"LANG": "en"})


@dataclass(frozen=True)
class TrayState:
    send: bool = False
    receive: bool = False
    available: bool = False
    locale: object = field(default=None, compare=False, repr=False)

    @classmethod
    def from_answer(cls, answer, locale=None):
        if answer.get('type') != 'status':
            return cls(locale=locale)
        settings = answer['status']['settings']
        return cls(settings['send'], settings['receive'], True, locale)

    def text(self, key):
        return (self.locale or ENGLISH)(key)

    @property
    def title(self):
        key = 'tray.connecting' if not self.available else {
            (True, True): 'tray.both', (True, False): 'tray.send_only',
            (False, True): 'tray.receive_only', (False, False): 'tray.paused',
        }[(self.send, self.receive)]
        return NAME + ' · ' + self.text(key)

    def menu(self):
        return [
            (1, self.title, False, None),
            (10, self.text('tray.open'), True, None),
            (11, None, False, None),
            (20, self.text('tray.enable_send'), self.available, self.send),
            (21, self.text('tray.enable_receive'), self.available, self.receive),
            (30, self.text('tray.send'), self.available and self.send, None),
            (39, None, False, None),
            (40, self.text('tray.quit'), self.available, None),
        ]

    def action(self, ident):
        if not self.available:
            return None
        if ident == 40:
            return {'command': 'quit'}
        if ident == 20:
            return {'command': 'set_directions', 'send': not self.send}
        if ident == 21:
            return {'command': 'set_directions', 'receive': not self.receive}
        if ident == 30 and self.send:
            return {'command': 'send'}
        return None


def icon_pixels(state, size=24):
    """Linked sheets: arrows, pause bars, or dashed unavailable outline."""
    if size not in (16, 24, 32):
        raise ValueError('unsupported tray icon size')
    ident = 5 if not state.available else {
        (True, True): 1, (True, False): 2,
        (False, True): 3, (False, False): 4,
    }[(state.send, state.receive)]
    return pixels(ident, size)
