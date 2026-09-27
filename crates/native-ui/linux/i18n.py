"""Local presentation preferences and shared UTF-8 catalogs; no sync API access."""
from brand import NAME
import json
import os
from pathlib import Path
import re
import tempfile

LANGUAGES = [('system', None), ('en', 'English'), ('nl', 'Nederlands'), ('de', 'Deutsch'), ('fr', 'Français')]
CODES = {code for code, _ in LANGUAGES}


def language_code(value):
    code = value.split('.')[0].split('@')[0].replace('-', '_').split('_')[0].lower()
    return code if code in CODES - {'system'} else None


def system_language(environ=None):
    env = os.environ if environ is None else environ
    # LANGUAGE is a preference list; LC_ALL overrides the POSIX locale categories.
    locale = env.get('LC_ALL') or env.get('LC_MESSAGES') or env.get('LANG') or 'en'
    if locale in ('C', 'POSIX', 'C.UTF-8', 'C.utf8'):
        return 'en'
    candidates = (env.get('LANGUAGE') or locale).split(':')
    return next((c for value in candidates if (c := language_code(value))), 'en')


class Translator:
    def __init__(self, profile=None, environ=None, catalog_dir=None):
        self.path = Path(profile) / 'ui-preferences.json' if profile else None
        self.environ = environ
        base = Path(__file__).resolve().parent
        directory = Path(catalog_dir) if catalog_dir else (base / 'locales' if (base / 'locales').is_dir() else base.parent / 'locales')
        self.catalogs = {code: json.loads((directory / (code + '.json')).read_text(encoding='utf-8')) for code, _ in LANGUAGES if code != 'system'}
        self.preference = 'system'
        self.language = system_language(environ)
        self.signature = None
        self.reload()

    def reload(self):
        """Stat only when unchanged; corrupted preferences safely use the system locale."""
        try:
            stat = self.path.stat() if self.path else None
            signature = (stat.st_mtime_ns, stat.st_size, stat.st_ino) if stat else None
        except OSError:
            signature = None
        if signature == self.signature:
            return False
        preference = 'system'
        try:
            if self.path and signature and signature[1] <= 4096:
                value = json.loads(self.path.read_text(encoding='utf-8')).get('language')
                if value in CODES:
                    preference = value
        except (OSError, ValueError, AttributeError, TypeError):
            pass
        language = preference if preference != 'system' else system_language(self.environ)
        changed = (language, preference) != (self.language, self.preference)
        self.signature, self.preference, self.language = signature, preference, language
        return changed

    def select(self, preference):
        if preference not in CODES:
            raise ValueError('unsupported UI language')
        if not self.path:
            raise ValueError('a local profile is required')
        self.path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        name = None
        try:
            with tempfile.NamedTemporaryFile(mode='w', encoding='utf-8', dir=self.path.parent, prefix='.ui-preferences-', delete=False) as f:
                name = f.name
                json.dump({'version': 1, 'language': preference}, f)
                f.flush()
                os.fsync(f.fileno())
            os.replace(name, self.path)
        finally:
            if name and os.path.exists(name):
                os.unlink(name)
        return self.reload()

    def __call__(self, key, **values):
        template = self.catalogs[self.language].get(key, self.catalogs['en'].get(key, key))
        return template.format(app_name=NAME, **values) if values or "{app_name}" in template else template

    def plural(self, key, count):
        # Integer CLDR cardinal rules for these four languages (French includes 0).
        one = count in (0, 1) if self.language == 'fr' else count == 1
        return self(key + ('.one' if one else '.other'), count=count)

    def message(self, text):
        """Compatibility boundary for existing API prose; unknown diagnostics stay intact."""
        for key, value in self.catalogs['en'].items():
            if key.startswith(('notice.', 'result.')) and text == value:
                return self(key)
        for pattern, key in [(r'(\d+) device\(s\)', 'notice.devices'),
                             (r'Queued for (\d+) allowed device\(s\)', 'result.queued'),
                             (r'History queued as a new event for (\d+) device\(s\)', 'result.resent')]:
            match = re.fullmatch(pattern, text)
            if match:
                return self.plural(key, int(match[1]))
        return text
