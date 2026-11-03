#!/usr/bin/env python3
"""Explicit real-clipboard fixture bridge. JSON stdio; never prints clipboard data to logs."""
import argparse
import base64
import json
import hashlib
import os
from pathlib import Path
import sys
import threading

p = argparse.ArgumentParser()
p.add_argument('--session-pid', type=int, required=True)
p.add_argument('--allow-real-clipboard', action='store_true', required=True)
args = p.parse_args()
session = Path('/proc') / str(args.session_pid)
if session.stat().st_uid != os.getuid():
    raise SystemExit('desktop process must belong to this user')
for field in (session/'environ').read_bytes().split(b'\0'):
    key, separator, value = field.partition(b'=')
    if separator and key in (b'DISPLAY', b'XAUTHORITY', b'XDG_RUNTIME_DIR'):
        os.environ[key.decode()] = value.decode()
os.environ['GDK_BACKEND'] = 'x11'

import gi
gi.require_version('Gdk', '4.0')
gi.require_version('Gtk', '4.0')
from gi.repository import Gdk, GLib, GObject, Gtk

Gtk.init()
display = Gdk.Display.get_default()
if not display:
    raise SystemExit('desktop display unavailable')
clipboard = display.get_clipboard()
loop = GLib.MainLoop()
providers = []


def respond(value):
    print(json.dumps(value), flush=True)


def execute(request):
    def captured(format, data):
        if request.get('summary'):
            normalized = data.replace(b'\r\n', b'\n') if format == 'text' else data
            respond({'format': format, 'bytes': len(data), 'sha256': hashlib.sha256(normalized).hexdigest()})
        else:
            respond({'format': format, 'data': base64.b64encode(data).decode()})
    try:
        if request['op'] == 'write':
            data = base64.b64decode(request['data'], validate=True)
            if len(data) > 8*1024*1024:
                raise ValueError('fixture too large')
            if request['format'] == 'text':
                text = data.decode('utf-8')
                if '\0' in text:
                    raise ValueError('NUL in fixture')
                value = GObject.Value(str, text)
                provider = Gdk.ContentProvider.new_for_value(value)
            else:
                provider = Gdk.ContentProvider.new_for_bytes('image/png', GLib.Bytes.new(data))
            if not clipboard.set_content(provider):
                raise RuntimeError('fixture ownership failed')
            providers[:] = [provider]
            display.flush()
            respond({'ok': True})
        elif request['op'] == 'read':
            mimes = clipboard.get_formats().get_mime_types()
            if 'image/png' in mimes:
                def ready(source, result):
                    try:
                        stream, _ = source.read_finish(result)
                        blocks = bytearray()
                        def more(stream, result):
                            try:
                                block = stream.read_bytes_finish(result).get_data()
                                if block:
                                    blocks.extend(block)
                                    if len(blocks) > 8*1024*1024:
                                        raise ValueError('clipboard too large')
                                    # Finish the GDK callback before scheduling another
                                    # read: immediate re-entry can discard its GTask
                                    # when an INCR chunk is already buffered.
                                    def next_chunk():
                                        stream.read_bytes_async(65536, 0, None, more)
                                        return False
                                    GLib.idle_add(next_chunk)
                                else:
                                    captured('png', blocks)
                            except Exception as error:
                                respond({'error': str(error)})
                        stream.read_bytes_async(65536, 0, None, more)
                    except Exception as error:
                        respond({'error': str(error)})
                clipboard.read_async(['image/png'], 0, None, ready)
            else:
                def text_ready(source, result):
                    try:
                        value = source.read_text_finish(result)
                        if value is not None:
                            captured('text', value.encode())
                        else:
                            respond({'format': None, 'data': None})
                    except Exception as error:
                        respond({'error': str(error)})
                clipboard.read_text_async(None, text_ready)
        elif request['op'] == 'store':
            def stored(source, result):
                try:
                    source.store_finish(result)
                    respond({'ok': True})
                except Exception as error:
                    respond({'error': str(error)})
            clipboard.store_async(0, None, stored)
        elif request['op'] == 'quit':
            loop.quit()
        else:
            raise ValueError('unknown operation')
    except Exception as error:
        respond({'error': str(error)})
    return False


def input_worker():
    for line in sys.stdin:
        try:
            GLib.idle_add(execute, json.loads(line))
        except Exception as error:
            respond({'error': str(error)})
    GLib.idle_add(loop.quit)


threading.Thread(target=input_worker, daemon=True).start()
loop.run()
