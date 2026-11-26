import datetime
import json
import pathlib
import statistics
import sys


def watch_events(path):
    decoder = json.JSONDecoder()
    data = path.read_text()
    index = 0
    while (index := next((i for i in range(index, len(data)) if not data[i].isspace()), len(data))) < len(data):
        event, index = decoder.raw_decode(data, index)
        yield event


def time_of(path, message, name):
    for line in path.read_text().splitlines():
        timestamp, _, data = line.partition(' ')
        if not data.startswith('{'):
            continue
        log = json.loads(data)
        if log.get('msg') == message and log.get('name') == name:
            return datetime.datetime.fromisoformat(timestamp)
    raise RuntimeError(f'Missing {message} for {name}')


def prepared(obj):
    meta = obj['metadata']
    return (meta.get('labels', {}).get('app.kubernetes.io/created-by') == 'topolvm-controller'
            and 'topolvm.io/logicalvolume' in meta.get('finalizers', []))


root = pathlib.Path(sys.argv[1])
events = {}
for entry in watch_events(root / 'logicalvolume-watch.json'):
    if entry['type'] in ('ADDED', 'MODIFIED', 'DELETED'):
        obj = entry['object']
        events.setdefault(obj['metadata']['name'], []).append((entry['type'], obj))

results = {}
for variant in ('stock', 'patched'):
    rows = []
    for trial in (1, 2, 3):
        folder = root / f'{variant}-{trial}'
        for line in (folder / 'volume-ids.tsv').read_text().splitlines():
            role, name, handle = line.split('\t')
            history = events[name]
            assert history[0][0] == 'ADDED' and history[-1][0] == 'DELETED'
            first_ready = next(i for i, (_, obj) in enumerate(history) if obj.get('status', {}).get('volumeID') == handle)
            assert prepared(history[first_ready][1])
            assert prepared(history[0][1]) == (variant == 'patched')
            assert first_ready == 1 if variant == 'patched' else first_ready >= 3
            added = time_of(folder / 'create.log', 'created LogicalVolume CR', name)
            created = time_of(folder / 'lv.log', 'created new LV', name)
            rows.append({'trial': trial, 'role': role, 'metadata_already_prepared': prepared(history[0][1]),
                         'updates_before_ready': first_ready - 1, 'create_to_native_ms': (created - added).total_seconds() * 1000})
    results[variant] = {'volumes': rows, 'median_create_to_native_ms': round(statistics.median(r['create_to_native_ms'] for r in rows), 1)}

assert json.loads((root / 'upgrade-and-rollback.json').read_text())['same_peer_pod_and_container_after_upgrade_and_rollback']
(root / 'prepared-provisioning.json').write_text(json.dumps(results, indent=2) + '\n')
print(json.dumps(results))
