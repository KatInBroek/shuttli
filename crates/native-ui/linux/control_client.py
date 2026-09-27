"""Bounded local control transport shared by the window and tray."""
import json
import socket
from pathlib import Path

API_VERSION = 2
MAX_REQUEST = 256 * 1024


def request(path, action, *, timeout=12, max_response=12 * 1024 * 1024):
    data = json.dumps({'version': API_VERSION, 'action': action}).encode()
    if len(data) > MAX_REQUEST:
        raise ValueError('Control request exceeds limit')
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
        stream.settimeout(timeout)
        try:
            stream.connect(path)
        except (FileNotFoundError, ConnectionRefusedError):
            if Path(path).with_name('stopped').is_file():
                return {'type': 'stopped'}
            raise
        stream.sendall(data)
        stream.shutdown(socket.SHUT_WR)
        with stream.makefile('rb') as reader:
            data = reader.read(max_response + 1)
        if len(data) > max_response:
            raise ValueError('Control response exceeds limit')
        answer = json.loads(data)
        if not isinstance(answer, dict) or not isinstance(answer.get('type'), str):
            raise ValueError('Invalid control response')
        return answer
