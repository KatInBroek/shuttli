"""Explicit two-device real-clipboard stability checks; reports contain metadata only."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import queue
import random
import struct
import subprocess
import threading
import time
import zlib


class Bridge:
    def __init__(self, command):
        self.process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True, encoding='utf-8')
        self.lines = queue.Queue()
        def reader():
            for line in self.process.stdout:
                self.lines.put(line)
            self.lines.put(None)
        threading.Thread(target=reader, daemon=True).start()

    def call(self, value):
        self.process.stdin.write(json.dumps(value) + '\n')
        self.process.stdin.flush()
        line = self.lines.get(timeout=12)
        if line is None:
            raise RuntimeError('clipboard bridge exited: ' + self.process.stderr.read()[:2000])
        result = json.loads(line)
        if 'error' in result:
            raise RuntimeError(result['error'])
        return result

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.terminate()


def image(index, large=False):
    width, height = ((480, 270), (1920, 1080), (1600, 1600))[index % 3] if not large else (1024, 768)
    def chunk(kind, body):
        return struct.pack('!I', len(body)) + kind + body + struct.pack('!I', zlib.crc32(kind + body))
    if large:
        pixels = random.Random(index).randbytes(width * height * 4)
        rows = b''.join(b'\0' + pixels[y*width*4:(y+1)*width*4] for y in range(height))
    else:
        rows = b''.join(b'\0' + bytes(((index*29+y) % 256, (index*31+y*3) % 256, (index*17) % 256, 255)) * width for y in range(height))
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('!IIBBBBB', width, height, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b'')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--config', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--count', type=int, default=30)
    parser.add_argument('--remote-empty', action='store_true')
    parser.add_argument('--large-images', action='store_true', help='use incompressible multi-megabyte PNG fixtures')
    parser.add_argument('--restart-receiver', action='store_true', help='restart the receiver around each explicit resend')
    parser.add_argument('--resend-id', type=int, help='reuse retained content; do not access the Windows OS clipboard')
    parser.add_argument('--allow-clipboard-overwrite', action='store_true', required=True)
    args = parser.parse_args()
    config = json.loads(args.config.read_text(encoding='utf-8-sig'))
    if args.restart_receiver and not args.resend_id:
        parser.error('--restart-receiver requires --resend-id')
    prefix = config['ssh_prefix']
    windows_binary = config['windows_binary']
    env = dict(os.environ, SHUTTLI_DATA_DIR=config['windows_profile'])
    results = []
    def api(side, *command):
        invocation = [windows_binary, *command, '--json'] if side == 'windows' else [*prefix, config['linux_binary'], *command, '--json']
        for _ in range(25):
            result = subprocess.run(invocation, env=env, capture_output=True, text=True, encoding='utf-8', timeout=18)
            answer = json.loads(result.stdout)
            if answer['type'] != 'error':
                return answer
            if not answer['message'].startswith('operation in progress'):
                raise RuntimeError(answer['message'])
            time.sleep(.1)
        raise RuntimeError('agent remained busy')
    local = None if args.resend_id else Bridge([config['windows_helper'], 'clipboard'])
    remote = Bridge([*prefix, 'python3', config['linux_bridge'], '--session-pid', str(config['linux_session_pid']), '--allow-real-clipboard'])
    saved_local = saved_remote = None
    preview = None
    original = {}
    try:
        saved_local = local.call({'op': 'read'}) if local else None
        saved_remote = remote.call({'op': 'read'})
        for side in ('windows', 'linux'):
            original[side] = api(side, 'settings')['settings']
        target = api('linux', 'status')['status']['device']
        if not original['windows']['peers'].get(target, {}).get('send'):
            raise RuntimeError('test target does not already have sending permission')
        api('linux', 'set', 'send', 'off')  # prevent fixture forwarding to unrelated allowed devices
        api('windows', 'set', 'automatic', 'on')
        api('windows', 'set', 'notifications', 'off')
        api('linux', 'set', 'notifications', 'off')
        resend_id = args.resend_id
        for index in range(args.count):
            if resend_id:
                preview = api('windows', 'history', 'preview', str(resend_id))
                kind, data = preview['format'], base64.b64decode(preview['base64'])
            else:
                kind = 'text' if index % 2 == 0 else 'png'
                data = ('Synthetic stability %d — 中文 😀\r\nline\nCR\rend' % index).encode() if kind == 'text' else image(index, args.large_images)
            before = max((row['id'] for row in api('windows', 'history')['entries']), default=0)
            if args.remote_empty:
                remote.call({'op': 'write', 'format': 'text', 'data': ''})
                time.sleep(.8)
            started = time.monotonic()
            restart = None
            if args.restart_receiver:
                restart = subprocess.Popen([*prefix, 'python3', config['linux_lifecycle'],
                    '--binary', config['linux_binary'], '--pid', str(config['linux_session_pid']),
                    '--state', config['linux_restart_state'], '--allow-restart'],
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding='utf-8')
                if json.loads(restart.stdout.readline()).get('stopped') is not True:
                    raise RuntimeError('receiver did not stop for recovery fixture')
            if resend_id:
                api('windows', 'history', 'resend', str(resend_id))
                expected = preview
            else:
                observed = local.call({'op': 'inspect'})
                local.call({'op': 'write', 'generation': observed['generation'], 'format': kind, 'data': base64.b64encode(data).decode()})
                expected = local.call({'op': 'read'})
            if restart:
                line = restart.stdout.readline()
                restart.wait(timeout=12)
                if restart.returncode or not line:
                    raise RuntimeError('receiver restart failed: ' + restart.stderr.read()[:1000])
                config['linux_session_pid'] = json.loads(line)['pid']
                args.config.write_text(json.dumps(config, indent=2), encoding='utf-8')
            deadline, outcome = time.monotonic() + 22, None
            while time.monotonic() < deadline:
                candidates = [row for row in api('windows', 'history')['entries'] if row['id'] > before and row['direction'] == 'send' and row['peer'] == target and row['format'] == kind]
                if candidates and candidates[0]['state'] in ('applied', 'failed', 'unknown', 'cancelled', 'superseded'):
                    outcome = candidates[0]
                    break
                time.sleep(.15)
            if outcome is None:
                outcome = {'state': 'timeout', 'detail': 'no terminal receipt'}
            interrupted_state = None
            if restart and outcome['state'] != 'applied':
                interrupted_state = outcome['state']
                # A fresh explicit send tests recovery. Never claim the interrupted
                # transfer was retried automatically or acknowledged successfully.
                time.sleep(5)
                before = max((row['id'] for row in api('windows', 'history')['entries']), default=0)
                api('windows', 'history', 'resend', str(resend_id))
                outcome = None
                deadline = time.monotonic() + 22
                while time.monotonic() < deadline:
                    candidates = [row for row in api('windows', 'history')['entries'] if row['id'] > before and row['direction'] == 'send' and row['peer'] == target]
                    if candidates and candidates[0]['state'] in ('applied', 'failed', 'unknown', 'cancelled', 'superseded'):
                        outcome = candidates[0]
                        break
                    time.sleep(.2)
                if outcome is None:
                    outcome = {'state': 'timeout', 'detail': 'no terminal receipt after recovery'}
            actual = remote.call({'op': 'read', 'summary': True})
            expected_body = base64.b64decode(expected.get('data') or expected.get('base64') or '')
            actual_body = base64.b64decode(actual.get('data') or '')
            if kind == 'text':
                expected_body, actual_body = expected_body.replace(b'\r\n', b'\n'), actual_body.replace(b'\r\n', b'\n')
            actual_digest = actual.get('sha256') or hashlib.sha256(actual_body).hexdigest()
            verified = outcome['state'] == 'applied' and actual.get('format') == kind and hashlib.sha256(expected_body).hexdigest() == actual_digest
            result = dict(index=index, kind=kind, bytes=len(data), state=outcome['state'], detail=outcome.get('detail'),
                          source_id=outcome.get('id'), elapsed_ms=round((time.monotonic()-started)*1000), independent_readback=verified)
            results.append(result)
            if interrupted_state:
                result['interrupted_state'] = interrupted_state
                result['explicit_send_after_restart'] = True
            if not verified:
                status = api('linux', 'status')['status']
                result['receiver_available'] = status['clipboard_available']
                result['receiver_error'] = status['last_error']
            print(json.dumps(result), flush=True)
            if not verified:
                break
            if resend_id:
                resend_id = outcome['id']
            time.sleep(.25)
    finally:
        for side in original:
            try:
                api(side, 'set', 'send', 'off')
            except Exception:
                pass
        try:
            if saved_local and saved_local.get('data'):
                observed = local.call({'op': 'inspect'})
                local.call(dict(op='write', generation=observed['generation'], format=saved_local['format'], data=saved_local['data']))
            if saved_remote and saved_remote.get('format'):
                remote.call(dict(op='write', format=saved_remote['format'], data=saved_remote['data']))
                if args.resend_id and preview and saved_remote['format'] == preview['format'] and saved_remote['data'] == preview['base64']:
                    api('windows', 'set', 'send', 'on')
                    time.sleep(.7)
                    api('windows', 'history', 'resend', str(resend_id))
                    time.sleep(1.5)
                else:
                    try:
                        remote.call({'op': 'store'})
                    except (RuntimeError, queue.Empty):
                        pass
            elif saved_remote:
                remote.call(dict(op='write', format='text', data=''))
        finally:
            for side, settings in original.items():
                for key in ('automatic', 'notifications', 'send'):
                    api(side, 'set', key, 'on' if settings[key] else 'off')
            if local:
                local.close()
            remote.close()
            args.report.parent.mkdir(parents=True, exist_ok=True)
            args.report.write_text(json.dumps(dict(requested=args.count, remote_empty=args.remote_empty,
                restart_receiver=args.restart_receiver, large_images=args.large_images, results=results), indent=2), encoding='utf-8')
    if len(results) != args.count or not all(row['independent_readback'] for row in results):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
