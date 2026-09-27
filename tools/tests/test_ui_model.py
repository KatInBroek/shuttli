"""History presentation must preserve provenance and each destination outcome."""
from pathlib import Path
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'crates/native-ui/linux'))
from ui_model import group_history, peer_name


def row(ident, seq, peer, state='applied', available=True, direction='send'):
    return dict(id=ident, event={'origin': [1] * 32, 'epoch': [2] * 16, 'seq': seq},
                peer=peer, direction=direction, state=state, format='text', bytes=4,
                time=seq, available=available, detail='fixture')


class HistoryPresentationTests(unittest.TestCase):
    def test_destinations_group_without_hiding_failure_or_losing_copyable_id(self):
        a = row(1, 1, 'a', available=False)
        b = row(2, 1, 'b', state='unknown')
        result = group_history([a, b])
        self.assertEqual(len(result), 1)
        self.assertEqual(result[0]['content_id'], 2)
        self.assertEqual([(x['peer'], x['state']) for x in result[0]['transfers']],
                         [('a', 'applied'), ('b', 'unknown')])
        self.assertNotIn('content_id', a)

    def test_resends_equal_bytes_and_received_events_do_not_merge(self):
        result = group_history([row(1, 1, 'a'), row(2, 2, 'a'),
                                row(3, 2, 'b', direction='receive')])
        self.assertEqual([x['id'] for x in result], [3, 2, 1])
        self.assertEqual(result[0]['peer'], 'b')

    def test_status_only_and_missing_peers_have_honest_fallbacks(self):
        self.assertIsNone(group_history([row(1, 1, 'a', available=False)])[0]['content_id'])
        self.assertEqual(peer_name([], 'f'*64), 'f'*12)
        self.assertEqual(peer_name([{'id': 'f'*64, 'name': 'Renamed device'}], 'f'*64), 'Renamed device')
        self.assertEqual(group_history([]), [])

    def test_local_copy_groups_with_automatic_destinations_and_keeps_local_preview(self):
        local = row(1, 1, 'self', direction='local')
        outgoing = row(2, 1, 'peer', state='unknown')
        for order in ([local, outgoing], [outgoing, local]):
            groups = group_history(order)
            self.assertEqual(len(groups), 1)
            self.assertTrue(groups[0]['local'])
            self.assertEqual(groups[0]['direction'], 'local')
            self.assertEqual(groups[0]['content_id'], 1)
            self.assertEqual(groups[0]['transfers'], [outgoing])
        only_local = group_history([local])[0]
        self.assertEqual(only_local['transfers'], [])
        self.assertEqual(only_local['content_id'], 1)
