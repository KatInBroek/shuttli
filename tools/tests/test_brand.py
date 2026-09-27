"""Rename contracts across presentation boundaries, without touching user profiles."""
import importlib.util
import plistlib
import runpy
import shutil
import contextlib
import io
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
UI = ROOT / 'crates/native-ui/linux'
sys.path.insert(0, str(UI))
import brand
import i18n
import tray_model


class BrandTests(unittest.TestCase):
    def test_catalogs_and_tray_preserve_literal_name_in_all_languages(self):
        name = 'Élan "A" \\ {count} & <B>'
        with patch.object(i18n, 'NAME', name), patch.object(tray_model, 'NAME', name):
            for language in ('en', 'nl', 'de', 'fr'):
                translator = i18n.Translator(environ={'LANG': language})
                for key in ('tray.open', 'devices.empty', 'status.close_hint'):
                    self.assertIn(name, translator(key))
                self.assertTrue(tray_model.TrayState(locale=translator).title.startswith(name + ' · '))
                self.assertEqual(translator.message('Unknown {raw}'), 'Unknown {raw}')

    def test_extracted_resources_load_without_source_tree(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp)
            (path / 'brand.py').write_bytes((UI / 'brand.py').read_bytes())
            name = 'Élan "A" \\ {name}'
            (path / 'product-name.txt').write_text(name, encoding='utf-8')
            result = subprocess.check_output([sys.executable, '-c', 'import brand; print(brand.NAME)'], cwd=path, env={**os.environ, 'PYTHONIOENCODING': 'utf-8'})
            self.assertEqual(result.decode().strip(), name)

    def test_invalid_names_fail_instead_of_silently_using_old_brand(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'name.txt'
            for name in ('', ' leading', 'trailing ', 'bad\nName', 'bad\x00Name', 'a'*129):
                path.write_text(name, encoding='utf-8')
                with self.assertRaises(ValueError):
                    brand.load_name(path)
            path.write_text('Renamed\n', encoding='utf-8')
            self.assertEqual(brand.load_name(path), 'Renamed')

    def test_generated_readme_is_current_and_render_is_idempotent(self):
        spec = importlib.util.spec_from_file_location('render_branding', ROOT / 'tools/render_branding.py')
        renderer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(renderer)
        readme = (ROOT / 'README.md').read_text(encoding='utf-8')
        self.assertEqual(renderer.render(readme, brand.NAME), readme)
        other = renderer.render(readme, 'Élan [new] & tools')
        self.assertIn('Élan \\[new\\] \\& tools', other)
        self.assertEqual(renderer.render(other, 'Élan [new] & tools'), other)

    def test_installer_uses_artifact_name_and_preserves_install_ids(self):
        name = 'Élan "new" \\ & <tools>'
        for platform in ('linux', 'darwin'):
            with self.subTest(platform=platform), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                script = root / 'tools/install.py'
                script.parent.mkdir()
                shutil.copyfile(ROOT / 'tools/install.py', script)
                artwork = root / 'crates/native-ui/assets/app-icon.svg'
                artwork.parent.mkdir(parents=True)
                shutil.copyfile(ROOT / 'crates/native-ui/assets/app-icon.svg', artwork)
                binary = root / 'target/release/shuttli'
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b'test executable')
                prefix, apps = root / 'prefix', root / 'apps'
                with patch.object(sys, 'argv', [str(script), '--prefix', str(prefix), '--app-dir', str(apps)]), \
                     patch.object(sys, 'platform', platform), \
                     patch('subprocess.check_output', return_value=name + '\n') as metadata, \
                     patch('subprocess.run'), contextlib.redirect_stdout(io.StringIO()):
                    runpy.run_path(str(script), run_name='__main__')
                metadata.assert_called_once_with([str(binary.resolve()), '--print-product-name'], encoding='utf-8')
                self.assertEqual((prefix / 'bin/shuttli').read_bytes(), b'test executable')
                if platform == 'linux':
                    entry = (prefix / 'share/applications/org.shuttli.Control.desktop').read_text(encoding='utf-8')
                    self.assertTrue((prefix / 'share/icons/hicolor/scalable/apps/org.shuttli.Control.svg').is_file())
                    self.assertIn('Icon=org.shuttli.Control', entry)
                    self.assertIn('Name=' + name.replace('\\', '\\\\') + '\n', entry)
                else:
                    info = plistlib.loads((apps / 'Shuttli.app/Contents/Info.plist').read_bytes())
                    self.assertEqual(info['CFBundleName'], name)
                    self.assertEqual(info['CFBundleDisplayName'], name)
                    self.assertEqual(info['CFBundleIdentifier'], 'org.shuttli.app')

    def test_display_literals_are_not_reintroduced(self):
        # Technical identifiers are distinct from localized display text.
        allowed = {
            'crates/adapters/src/files.rs': {'Library/Application Support/Shuttli'},
            'crates/native-ui/build.rs': {'macos/ShuttliUI.swift'},
        }
        for path in (ROOT / 'crates').rglob('*'):
            if path.suffix not in ('.rs', '.py', '.swift', '.json'):
                continue
            for line in path.read_text(encoding='utf-8').splitlines():
                for literal in allowed.get(path.relative_to(ROOT).as_posix(), set()):
                    line = line.replace(literal, '')
                self.assertNotIn('Shuttli', line, str(path))


if __name__ == '__main__':
    unittest.main()
