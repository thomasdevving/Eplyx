#!/usr/bin/env python3
"""Rebuild and execute the experimental record offline; reject changed evidence."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time
from unittest.mock import patch
import kamino_u3f_fidelity as fidelity

f = fidelity.f
lut = f.lut


def verify_pre(payload, execution):
    actual = execution['seeded_pre_accounts']
    lut.require(set(actual) == {s['address'] for s in payload['seeds']}, 'actual seed membership differs')
    for seed in payload['seeds']:
        observed = actual[seed['address']]
        lut.require(observed is not None, 'actual seed absent')
        for key in ('owner', 'lamports', 'executable', 'rentEpoch', 'data'):
            lut.require(observed[key] == seed['account'][key], 'actual VM pre-state differs from historical seed')


def run(output, verify_record=True):
    tick = time.perf_counter()
    env = {k: v for k, v in os.environ.items() if not any(x in k.upper() for x in ('RPC', 'ARCHIVE', 'API_KEY', 'ALCHEMY', 'HELIUS'))}
    if verify_record:
        record = lut.read(f.ROOT / 'phase-u3f-record/record.json')
        for name, digest in record['evidence'].items():
            lut.require(lut.sha((lut.REPO / name).read_bytes()) == digest, f'record evidence changed: {name}')
    subprocess.run(['cargo', 'build', '--offline', '-q', '-p', 'eplyx-engine', '--example', 'execute_envelope_v0', '--example', 'evaluate_envelope_semantics', '--example', 'verify_envelope_state', '--example', 'acquire_envelope_dependencies'], cwd=lut.REPO, env=env, check=True)
    with patch.object(f.s.transport.CurlArchive, 'once', side_effect=AssertionError('offline transport forbidden')):
        payload, manifest = f.prepare()
        pre, post = fidelity.references(payload)
        baseline = lut.read(f.ROOT / 'phase-u3f-materiality-attempt-1/empty.json')
        results = []
        # Reversed order independently exercises deterministic materiality.
        for variant in ('different', 'empty', 'default'):
            payload['variant'] = variant
            process = subprocess.run([str(lut.REPO / 'target/debug/examples/execute_envelope_v0')], cwd=lut.REPO, env=env, input=lut.canonical(payload), capture_output=True, timeout=180)
            lut.require(process.returncode == 0, process.stderr.decode(errors='replace'))
            execution = json.loads(process.stdout)
            verify_pre(payload, execution)
            lut.require(execution['evidence'] == baseline['evidence'], 'offline execution evidence changed')
            result = fidelity.compare(payload, execution, post)
            lut.require(result['fidelity'] == 'matched', 'offline raw fidelity failed')
            results.append({'variant': variant, 'evidence_sha256': lut.sha(lut.canonical(execution['evidence'])), 'fidelity': result['fidelity'], 'pre_state_exact': True, 'preparation_seconds': execution['preparation_seconds'], 'execution_seconds': execution['execution_seconds']})
        result, semantics = fidelity.derive()
        lut.require(semantics == lut.read(f.VALIDATION / 'T1-semantics.json'), 'existing semantics changed')
        lut.require(result == lut.read(f.VALIDATION / 'T1-fidelity.json'), 'reconciliation changed')
    report = {'offline_reexecution_passed': True, 'record_hashes_verified': verify_record, 'provider_environment_removed': True, 'live_transport_forbidden': True, 'variants': results, 'elapsed_seconds': time.perf_counter() - tick}
    if output:
        lut.require(not output.exists(), 'refuses overwrite of offline report')
        output.write_bytes(lut.canonical(report))
    print(lut.canonical(report).decode())
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    run(args.output)
