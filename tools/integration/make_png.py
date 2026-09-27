#!/usr/bin/env python3
"""Generate a deterministic RGBA fixture; does not use private image data."""
import hashlib
from pathlib import Path
import random
import struct
import sys
import zlib

width = height = 512
pixels = random.Random(20260926).randbytes(width * height * 4)

def chunk(kind, data):
    return struct.pack('>I',len(data)) + kind + data + struct.pack('>I',zlib.crc32(kind+data)&0xffffffff)

raw = b''.join(b'\0'+pixels[y*width*4:(y+1)*width*4] for y in range(height))
data = (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR',struct.pack('>IIBBBBB',width,height,8,6,0,0,0))
        + chunk(b'IDAT',zlib.compress(raw)) + chunk(b'IEND',b''))
if len(sys.argv) != 2:
    raise SystemExit('usage: make_png.py <output.png>')
Path(sys.argv[1]).write_bytes(data)
print('bytes:',len(data))
print('canonical_digest:',hashlib.sha256(b'png.rgba8.v1\0'+struct.pack('>II',width,height)+pixels).hexdigest())
