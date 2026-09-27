"""Presentation values; never decides synchronization permissions."""
import json

DEFAULT_POLICY = dict(send=False, receive=True, text=True, png=True,
                      max_bytes=8388608, quiet=False, history=None)


def group_history(entries):
    """Group one event's destinations, retaining every result and copyable ID.

    The API returns local and transfer rows. Fetch all pages before grouping.
    Different events with identical bytes must remain distinct.
    """
    groups = {}
    for entry in entries:
        key = (json.dumps(entry['event'], sort_keys=True), 'send' if entry['direction'] == 'local' else entry['direction'])
        if key not in groups:
            groups[key] = dict(entry, transfers=[], content_id=None, local=False)
        group = groups[key]
        if entry['direction'] == 'local':
            group['local'] = True
            group['direction'] = 'local'
        else:
            group['transfers'].append(entry)
        group['time'] = max(group['time'], entry['time'])
        if entry['available'] and (group['content_id'] is None or entry['direction'] == 'local' or not group['local']):
            group['content_id'] = entry['id']
    return sorted(groups.values(), key=lambda g: (g['time'], g['id']), reverse=True)


def peer_name(peers, fingerprint):
    return next((p['name'] for p in peers if p['id'] == fingerprint), fingerprint[:12])


def allowed_count(settings, peers):
    return sum(bool(settings['peers'].get(p['id'], DEFAULT_POLICY).get('send'))
               for p in peers)
