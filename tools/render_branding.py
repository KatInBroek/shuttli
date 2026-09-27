#!/usr/bin/env python3
"""Refresh generated product-name spans in README; historical records stay unchanged."""
import argparse
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'crates/native-ui/linux'))
from brand import NAME


def render(text, name):
    # Escape Markdown punctuation; newlines/control characters are rejected by the loader.
    escaped = re.sub(r'([\\`*_{}\[\]()<>#+.!|&~-])', r'\\\1', name)
    return re.sub(r'(?<=<!-- product-name:start -->).*?(?=<!-- product-name:end -->)',
                  lambda _: escaped, text)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    path = ROOT / 'README.md'
    original = path.read_text(encoding='utf-8')
    generated = render(original, NAME)
    if args.check and generated != original:
        raise SystemExit('Product name changed: run python tools/render_branding.py')
    if not args.check:
        path.write_text(generated, encoding='utf-8')


if __name__ == '__main__':
    main()
