"""Catalog contracts, fallbacks, live preference changes and UI-only scope."""
import ast
import json
from pathlib import Path
import string
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'crates/native-ui/linux'))
from i18n import Translator, LANGUAGES, system_language
from tray_model import TrayState


class I18nTests(unittest.TestCase):
    def test_catalogs_have_identical_keys_named_placeholders_and_no_blank_translations(self):
        t = Translator(environ={'LANG':'en'})
        english = t.catalogs['en']
        def placeholders(value):
            return {name for _,name,_,_ in string.Formatter().parse(value) if name is not None}
        for code, catalog in t.catalogs.items():
            self.assertEqual(set(catalog),set(english),code)
            for key, value in catalog.items():
                self.assertTrue(value.strip(), (code,key))
                self.assertEqual(placeholders(value),placeholders(english[key]),(code,key))
                self.assertTrue(all(name.isidentifier() for name in placeholders(value)))
        # All statically referenced UI keys must exist. Dynamic enums checked below.
        for name in ('shuttli.py','tray_model.py','tray.py'):
            tree=ast.parse((ROOT/'crates/native-ui/linux'/name).read_text())
            for node in ast.walk(tree):
                if isinstance(node,ast.Constant) and isinstance(node.value,str) and '.' in node.value:
                    value=node.value
                    if value.startswith(('ui.','nav.','tray.','history.','locale.','status.','common.','devices.','autostart.','setting.','action.')) and not value.endswith('.'):
                        self.assertIn(value,english,(name,value))
        for value in ['sending','receiving','applied','failed','cancelled','superseded','unknown']:
            self.assertIn('state.'+value,english)

    def test_system_locales_and_language_preferences(self):
        for env,expected in [({},'en'),({'LANG':'nl_NL.UTF-8'},'nl'),({'LANG':'de-DE'},'de'),
                             ({'LANG':'en_US','LC_MESSAGES':'fr_FR'},'fr'),
                             ({'LANG':'nl_NL','LANGUAGE':'xx:de:fr'},'de'),
                             ({'LANG':'fr_FR','LC_ALL':'C','LANGUAGE':'nl'},'en'),
                             ({'LANG':'zh_CN.UTF-8'},'en')]:
            self.assertEqual(system_language(env),expected)

    def test_device_preferences_reload_fallback_and_do_not_change_sync_settings(self):
        with tempfile.TemporaryDirectory() as directory:
            profile=Path(directory)
            settings=profile/'settings.json';settings.write_text('{"send":false,"receive":true}')
            before=settings.read_bytes()
            first=Translator(profile,environ={'LANG':'nl_NL'})
            second=Translator(profile,environ={'LANG':'nl_NL'})
            self.assertEqual(first.language,'nl')
            for code in ['de','fr','en','nl']:
                first.select(code)
                second.reload()
                self.assertEqual(second.language,code)
                self.assertEqual(second.preference,code)
                self.assertEqual(TrayState(True,True,True,second).menu()[1][1],second('tray.open'))
            self.assertFalse(second.reload())
            self.assertEqual(settings.read_bytes(),before)
            first.select('system');second.reload();self.assertEqual(second.language,'nl')
            with self.assertRaises(ValueError):first.select('../../etc')
            for malformed in ['{', '[]', '{"language":[]}', '{"language":"zz"}', 'x'*5000]:
                first.path.write_text(malformed);second.reload();self.assertEqual(second.language,'nl')
            first.select('de');second.reload();first.path.unlink();second.reload();self.assertEqual(second.language,'nl')

    def test_named_arguments_fallback_and_api_diagnostics(self):
        t=Translator(environ={'LANG':'de'})
        self.assertEqual(t('status.directions',send='A',receive='B'),'Senden: A · Empfangen: B')
        del t.catalogs['de']['tray.open']
        self.assertEqual(t('tray.open'),'Open window')
        self.assertEqual(t('future.key'),'future.key')
        self.assertEqual(t.message('TLS {raw}'),'TLS {raw}')
        self.assertEqual(t.message('Queued for 2 allowed device(s)'),t.plural('result.queued',2))
        self.assertEqual(t.message('Clipboard sent'),'Zwischenablage gesendet')
        t.language='fr'
        self.assertEqual(t.plural('notice.devices',0),'0 appareil')
        self.assertEqual(t.plural('notice.devices',2),'2 appareils')
