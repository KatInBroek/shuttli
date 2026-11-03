"""Write explicit, synthetic-only Windows presentation fixtures outside the repo."""
import argparse
import base64
import json
from pathlib import Path
import struct
import zlib


def png():
    def chunk(kind, body):
        return struct.pack('!I', len(body)) + kind + body + struct.pack('!I', zlib.crc32(kind + body))
    width, height = 320, 180
    pixels = b''.join(b'\0' + b''.join(bytes((43, 98, 217) if 45 < x < 275 and 35 < y < 145 else (230, 237, 249)) for x in range(width)) for y in range(height))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('!IIBBBBB', width, height, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(pixels)) + chunk(b'IEND', b'')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--language', choices=('system', 'en', 'nl', 'de', 'fr'), default='en')
    parser.add_argument('--theme', choices=('system', 'light', 'dark'), default='light')
    parser.add_argument('--empty', action='store_true')
    parser.add_argument('--unavailable', action='store_true')
    parser.add_argument('--text-scale', type=float, choices=(1, 2), default=1)
    args = parser.parse_args()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    policy = dict(send=False, receive=True, text=True, png=True, max_bytes=8388608, quiet=False, history=None)
    peers = [dict(id=c * 64, name=name, address=address, online=online) for c, name, address, online in (
        ('1', 'Studio workstation', '100.64.0.10:45987', True),
        ('2', 'Personal laptop', '100.64.0.11:45987', True),
        ('3', 'Build workstation with a deliberately long device name', '100.64.0.12:45987', False),
    )]
    settings = dict(version=1, send=True, receive=True, automatic=True, text=True, png=True, notifications=True,
                    history='content', history_limit=20, history_days=7, history_bytes=134217728,
                    history_memory_bytes=16777216, peers={p['id']: dict(policy, send=i != 1) for i, p in enumerate(peers)})
    image = png()
    text = 'Synthetic clipboard preview\nMeeting at 3 pm — café\n日本語 / 中文 / 😀\nThis is fixture content, not the current clipboard.'
    previews = {str(ident): dict(type='preview', format=kind, base64=base64.b64encode(data).decode()) for ident, kind, data in (
        (101, 'text', text.encode()), (102, 'png', image), (103, 'text', b'Synthetic multi-device delivery'))}
    rows = []
    for ident, seq, direction, peer, state, kind, available, stamp in (
        (101, 1, 'local', 'a'*64, 'applied', 'text', True, 1793340240),
        (104, 1, 'send', '1'*64, 'applied', 'text', True, 1793340240),
        (105, 1, 'send', '3'*64, 'unknown', 'text', True, 1793340240),
        (102, 2, 'receive', '2'*64, 'applied', 'png', True, 1793340180),
        (103, 3, 'send', '1'*64, 'failed', 'text', True, 1793340120),
        (106, 3, 'send', '3'*64, 'applied', 'text', True, 1793340120),
        (107, 4, 'receive', '1'*64, 'applied', 'text', False, 1793340060),
    ):
        rows.append(dict(id=ident, event=dict(origin=[1]*32, epoch=[2]*16, seq=seq), peer=peer, direction=direction,
                         state=state, format=kind, available=available, time=stamp, bytes=len(image) if kind == 'png' else 117,
                         detail='Synthetic acceptance result'))
    fixture = dict(status=dict(device='a'*64, clipboard='Windows native clipboard', clipboard_available=True,
                               last_error=None, settings=settings, policy_revision=1, sequence=4,
                               devices=dict(discovered=3, send=2, receive=3)),
                   devices=peers, history=rows, previews=previews, unavailable=args.unavailable, text_scale=args.text_scale)
    if args.empty:
        fixture['devices'], fixture['history'], settings['peers'] = [], [], {}
        fixture['status']['devices'] = dict(discovered=0, send=0, receive=0)
    args.output.write_text(json.dumps(fixture, ensure_ascii=False), encoding='utf-8')
    (args.output.parent / 'ui-preferences.json').write_text(json.dumps(dict(language=args.language, theme=args.theme)), encoding='utf-8')


if __name__ == '__main__':
    main()
