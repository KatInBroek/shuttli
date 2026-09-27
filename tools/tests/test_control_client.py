"""Exercise the shared window/tray transport against a real local socket."""
import json
import os
from pathlib import Path
import socket
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'crates/native-ui/linux'))
import control_client


@unittest.skipUnless(os.name == 'posix', 'Unix-domain desktop control')
class ControlClientTests(unittest.TestCase):
    def exchange(self, response, **options):
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / 'control.sock')
            received = []
            with socket.socket(socket.AF_UNIX) as server:
                server.bind(path)
                server.listen(1)
                def serve():
                    with server.accept()[0] as stream:
                        with stream.makefile('rb') as reader:
                            received.append(json.loads(reader.read()))
                        stream.sendall(response)
                worker = threading.Thread(target=serve)
                worker.start()
                try:
                    result = control_client.request(path, {'command': 'status'}, **options)
                finally:
                    worker.join(timeout=2)
                    self.assertFalse(worker.is_alive())
            self.assertEqual(received, [{'version': 2, 'action': {'command': 'status'}}])
            return result

    def test_versioned_request_and_structured_response(self):
        self.assertEqual(self.exchange(b'{"type":"settings","settings":{}}')['type'], 'settings')

    def test_oversized_and_malformed_responses_are_rejected(self):
        with self.assertRaisesRegex(ValueError, 'exceeds limit'):
            self.exchange(b'x' * 65, max_response=64)
        for body in (b'null', b'[]', b'{"message":"no type"}', b'invalid'):
            with self.subTest(body=body), self.assertRaises(ValueError):
                self.exchange(body)

    def test_explicit_quit_is_distinct_from_an_unexpected_outage(self):
        with tempfile.TemporaryDirectory() as directory:
            path = str(Path(directory) / 'control.sock')
            with self.assertRaises(FileNotFoundError):
                control_client.request(path, {'command': 'status'})
            Path(directory, 'stopped').write_text('explicit quit')
            self.assertEqual(control_client.request(path, {'command': 'status'}), {'type': 'stopped'})

    def test_oversized_request_is_rejected_before_connecting(self):
        with self.assertRaisesRegex(ValueError, 'request exceeds limit'):
            control_client.request('/unused', {'command': 'x' * control_client.MAX_REQUEST})
