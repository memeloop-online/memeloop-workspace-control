import datetime
import json
import pathlib
import re
import statistics
import sys


MESSAGES = {
    'cr_created': 'created LogicalVolume CR',
    'entered': 'benchmark: entered LV create',
    'rpc_started': 'benchmark: starting CreateLV RPC',
    'rpc_finished': 'benchmark: completed CreateLV RPC',
    'lv_ready': 'created new LV',
}
ROOT = pathlib.Path(sys.argv[1])


def logs(path, name):
    periods = {}
    for line in path.read_text().splitlines():
        timestamp, _, data = line.partition(' ')
        if not data.startswith('{'):
            continue
        log = json.loads(data)
        if log.get('name') != name:
            continue
        for key, message in MESSAGES.items():
            if log.get('msg') == message:
                assert key not in periods, (name, message)
                periods[key] = datetime.datetime.fromisoformat(timestamp).timestamp()
    return periods


def prom(path, name):
    matches = [line.rsplit(' ', 1)[-1] for line in path.read_text().splitlines()
               if re.match(name + r'(?:\{.*?\})? ', line)]
    assert len(matches) == 1, (name, path, matches)
    return float(matches[0])


def group(variant, trial, count):
    rows = []
    for i in range(1, count + 1):
        fixture = ROOT / f'{variant}-{trial}-{i}'
        for line in (fixture / 'volume-ids.tsv').read_text().splitlines():
            role, pv, _ = line.split('\t')
            stamps = logs(fixture / 'create.log', pv) | logs(fixture / 'lv.log', pv)
            assert set(stamps) == set(MESSAGES), (pv, stamps)
            assert stamps['entered'] <= stamps['rpc_started'] <= stamps['rpc_finished'] <= stamps['lv_ready']
            rows.append({'role': role, 'fixture': i, 'stamps': stamps,
                         'cr_to_admission_ms': round(1000 * (stamps['entered'] - stamps['cr_created']), 1),
                         'lookup_before_rpc_ms': round(1000 * (stamps['rpc_started'] - stamps['entered']), 1),
                         'native_rpc_ms': round(1000 * (stamps['rpc_finished'] - stamps['rpc_started']), 1),
                         'status_update_ms': round(1000 * (stamps['lv_ready'] - stamps['rpc_finished']), 1),
                         'cr_to_lv_ready_ms': round(1000 * (stamps['lv_ready'] - stamps['cr_created']), 1)})
    for role in ('home', 'scratch'):
        spans = sorted((r['stamps']['rpc_started'], r['stamps']['rpc_finished']) for r in rows if r['role'] == role)
        assert all(x[1] <= y[0] for x, y in zip(spans, spans[1:])), (variant, trial, spans)
    overlap = any(
        max(a['stamps']['rpc_started'], b['stamps']['rpc_started']) <
        min(a['stamps']['rpc_finished'], b['stamps']['rpc_finished'])
        for a in rows for b in rows if a['role'] != b['role']
    )
    return {'count': count, 'different_pool_rpcs_overlap': overlap, 'volumes': rows}


def medians(groups):
    rows = [r for g in groups for r in g['volumes']]
    return {k: round(statistics.median(r[k] for r in rows), 1) for k in rows[0] if k.endswith('_ms')}


results = {}
for variant in ('stock', 'patched'):
    serial = [group(variant, f'serial{i}', 1) for i in (1, 2, 3)]
    parallel = [group(variant, f'parallel{i}', 4) for i in (1, 2)]
    assert variant != 'stock' or not any(g['different_pool_rpcs_overlap'] for g in serial + parallel)
    before, after = (ROOT / f'{variant}-{phase}.prom' for phase in ('before', 'after'))
    assert prom(before, 'process_start_time_seconds') == prom(after, 'process_start_time_seconds')
    metrics = {}
    for phase in ('queue', 'work'):
        key = f'workqueue_{phase}_duration_seconds'
        count, duration = (prom(after, key + '_' + unit) - prom(before, key + '_' + unit) for unit in ('count', 'sum'))
        assert count > 0 and duration > 0
        metrics[phase] = {'operations': int(count), 'mean_ms': round(1000 * duration / count, 1)}
    results[variant] = {'serial': serial, 'parallel': parallel, 'serial_medians_ms': medians(serial),
                        'parallel_medians_ms': medians(parallel), 'all_reconciles_including_cleanup': metrics}

verification = json.loads((ROOT / 'upgrade-and-rollback.json').read_text())
assert all(verification.values())
report = {'verification': verification, 'measurements': results, 'across_separate_pools_overlap_observed':
          any(g['different_pool_rpcs_overlap'] for g in results['patched']['serial'] + results['patched']['parallel'])}
(ROOT / 'queue-comparison.json').write_text(json.dumps(report, indent=2) + '\n')
print(json.dumps({arm: r['parallel_medians_ms'] for arm, r in results.items()}))
