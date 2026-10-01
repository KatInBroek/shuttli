"""Display identity from the single source, or the running host's extracted copy."""
from pathlib import Path
import tomllib


def load_name(path):
    name = Path(path).read_text(encoding='utf-8').rstrip('\r\n')
    if not name or len(name.encode('utf-8')) > 128 or name.strip() != name or any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in name):
        raise ValueError('invalid product display name')
    return name


base = Path(__file__).resolve().parent
NAME = load_name(base / 'product-name.txt' if (base / 'product-name.txt').is_file()
                 else base.parents[2] / 'branding/name.txt')
VERSION = ((base / 'product-version.txt').read_text(encoding='utf-8').strip()
           if (base / 'product-version.txt').is_file()
           else tomllib.loads((base.parents[2] / 'Cargo.toml').read_text(encoding='utf-8'))['workspace']['package']['version'])
