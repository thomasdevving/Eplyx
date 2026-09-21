#!/usr/bin/env python3
"""Offline native-v0 materiality experiment from frozen historical evidence."""
import argparse
import base64
import json
import gzip
import os
from pathlib import Path
import subprocess
import tarfile
import time

import kamino_u3c_envelope as envelope
import kamino_u3e_runtime as runtime

s = runtime.s
lut = s.lut
ROOT = s.ROOT
VALIDATION = ROOT / 'phase-u3f-validation'


def read_capture(path):
    """Large immutable execution responses may be retained losslessly as gzip."""
    if path.exists():
        return lut.read(path)
    with gzip.open(str(path) + '.gz', 'rb') as stream:
        return json.load(stream)


def preserved():
    frozen = lut.read(VALIDATION / 'preimplementation.json')
    for name, digest in frozen['prior_U3E_hashes'].items():
        lut.require(lut.sha((lut.REPO / name).read_bytes()) == digest, f'prior U3E evidence changed: {name}')


def program_accounts():
    """Join retained slices at exact offsets; preserve all metadata and ELF padding."""
    root = ROOT / 'phase-u3c-dependencies'
    index = lut.read(root / 'capture-index.json')
    lut.require(lut.sha((root / 'capture.tar.gz').read_bytes()) == index['compressed_sha256'], 'binary capture archive hash differs')
    pieces = {}
    with tarfile.open(root / 'capture.tar.gz') as archive:
        receipt = json.load(archive.extractfile('receipt.json'))
        for request in receipt['requests']:
            if request['method'] != 'getAccountInfo' or request['status'] != 'success' or request['params'][1]['slot'] != s.inventory.SLOT - 1:
                continue
            address, config = request['params']
            raw = archive.extractfile(request['response_file']).read()
            lut.require(lut.sha(raw) == request['response_sha256'], 'historical program response hash differs')
            response = json.loads(raw)['result']
            lut.require(response['context']['slot'] == s.inventory.SLOT - 1, 'program account context differs')
            account = response['value']
            data = base64.b64decode(account['data'][0], validate=True)
            metadata = {k: v for k, v in account.items() if k != 'data'}
            entry = pieces.setdefault(address, {'metadata': metadata, 'chunks': {}, 'responses': []})
            lut.require(entry['metadata'] == metadata, 'ProgramData chunk metadata differs')
            offset = config.get('dataSlice', {}).get('offset', 0)
            lut.require(offset not in entry['chunks'], 'duplicate binary chunk context')
            entry['chunks'][offset] = data
            entry['responses'].append({'file': request['response_file'], 'sha256': request['response_sha256'], 'offset': offset})
    result = []
    for address, entry in sorted(pieces.items()):
        data = b''
        for offset, chunk in sorted(entry['chunks'].items()):
            lut.require(offset == len(data), 'missing ProgramData byte interval')
            data += chunk
        lut.require(len(data) == entry['metadata']['space'], 'incomplete historical executable account')
        result.append({'address': address, 'slot': s.inventory.SLOT - 1, 'kind': 'program',
                       'account': dict(entry['metadata'], data=[base64.b64encode(data).decode(), 'base64']),
                       'data_sha256': lut.sha(data), 'provenance': {'archive': 'phase-u3c-dependencies/capture.tar.gz', 'responses': entry['responses']}})
    return result


def prepare():
    preserved()
    # Independent offline boundary revalidation; the retained SlotHashes null is
    # preserved and is not relabeled as historical sysvar evidence.
    bank = runtime.run(ROOT / 'phase-u3e-runtime', verify=True)
    lut.require(bank['failure']['request']['name'] == 'SlotHashes', 'unexpected historical bank-input gap')
    row, binaries, proof = s.inventory.frozen_inputs()
    _, inputs, _ = envelope.frozen_inputs()
    frozen = next(i for i in inputs if i['result']['transaction']['signatures'][0] == s.inventory.SIG)
    seeds = program_accounts()
    absent = []
    root = ROOT / 'phase-u3e-state'
    receipt = lut.read(root / 'receipt.json')
    for a in receipt['attempts']:
        if a['method'] != 'getAccountInfo' or a['requested_slot'] != s.inventory.SLOT - 1 or a['failure_class']:
            continue
        response = lut.read(root / a['body_file'])['result']
        if response['value'] is None:
            item = next(i for i in s.inputs()[2]['planned_boundary_requests'] if i['address'] == a['account'] and i['boundary'] == 'pre')
            lut.require(s.permits_absence(row, item), 'required seed is absent')
            absent.append(a['account'])
            continue
        account = response['value']
        data = base64.b64decode(account['data'][0], validate=True)
        seeds.append({'address': a['account'], 'slot': a['requested_slot'], 'kind': 'ordinary', 'account': account,
                      'data_sha256': lut.sha(data), 'provenance': {'file': str((root / a['body_file']).relative_to(lut.REPO)), 'sha256': a['body_sha256']}})
    for item in frozen['evidence']:
        response = json.loads(base64.b64decode(item['raw_response_base64']))['result']
        account = response['value']
        seeds.append({'address': item['pubkey'], 'slot': response['context']['slot'], 'kind': 'lut', 'account': account,
                      'data_sha256': lut.sha(base64.b64decode(account['data'][0])), 'provenance': {'proof_id': proof['proof_id']}})
    root = ROOT / 'phase-u3e-runtime'
    receipt = lut.read(root / 'receipt.json')
    for a in receipt['attempts']:
        if a['failure_class']:
            continue
        response = lut.read(root / a['body_file'])['result']
        account = response['value']
        seeds.append({'address': a['account'], 'slot': a['requested_slot'], 'kind': 'runtime', 'account': account,
                      'data_sha256': lut.sha(base64.b64decode(account['data'][0])),
                      'provenance': {'file': str((root / a['body_file']).relative_to(lut.REPO)), 'sha256': a['body_sha256']}})
    addresses = [i['address'] for i in seeds]
    lut.require(len(addresses) == len(set(addresses)), 'seed identity overlap')
    watch = [i['address'] for i in row['transaction']['account_keys'] if i['address'] != s.inventory.INSTRUCTIONS]
    manifest = {'kind': 'experimental_native_v0_seed_manifest', 'signature': s.inventory.SIG, 'slot': s.inventory.SLOT,
                'proof_id': proof['proof_id'], 'original_outer_instruction_count': len(row['transaction']['instructions']),
                'seeds': [{k: v for k, v in seed.items() if k != 'account'} | {'owner': seed['account']['owner'], 'lamports': seed['account']['lamports'], 'executable': seed['account']['executable'], 'data_bytes': len(base64.b64decode(seed['account']['data'][0]))} for seed in seeds],
                'absent': absent, 'watch': watch, 'SlotHashes': 'controlled default/empty/different variants; none claimed historical'}
    return {'frozen': frozen, 'proof': proof, 'programs': binaries['programs'], 'seeds': seeds, 'absent': absent, 'watch': watch}, manifest


def run(output):
    lut.require(not output.exists(), 'new immutable experiment directory required')
    payload, manifest = prepare()
    output.mkdir(parents=True)
    (output / 'input-manifest.json').write_bytes(lut.canonical(manifest))
    policy = {'variants': ['default', 'empty', 'different'], 'original_message_and_signatures_preserved': True,
              'watch_set': manifest['watch'], 'post_state_seed_allowed': False, 'historical_SlotHashes_claim': False,
              'success_required_for_materiality_proof': True, 'reconstruction_if_material': True,
              'production_admission': False}
    (output / 'experiment-policy.json').write_bytes(lut.canonical(policy))
    env = {k: v for k, v in os.environ.items() if not any(x in k.upper() for x in ('RPC', 'ARCHIVE', 'API_KEY', 'ALCHEMY', 'HELIUS'))}
    results = []
    for variant in policy['variants']:
        payload['variant'] = variant
        tick = time.perf_counter()
        run = subprocess.run([str(lut.REPO / 'target/debug/examples/execute_envelope_v0')], input=lut.canonical(payload), env=env, capture_output=True, timeout=180)
        (output / f'{variant}.stderr').write_bytes(run.stderr)
        if run.returncode:
            result = {'variant': variant, 'construction_failure': run.stderr.decode(errors='replace'), 'process_exit': run.returncode, 'wall_seconds': time.perf_counter() - tick, 'submitted_to_vm': False}
        else:
            result = json.loads(run.stdout)
            result['wall_seconds'] = time.perf_counter() - tick
            result['submitted_to_vm'] = True
        (output / f'{variant}.json').write_bytes(lut.canonical(result))
        results.append(result)
        print(json.dumps({'variant': variant, 'submitted': result['submitted_to_vm'], 'success': result.get('evidence', {}).get('success'), 'error': result.get('evidence', {}).get('error') or result.get('construction_failure')}), flush=True)
    comparable = all(r['submitted_to_vm'] for r in results)
    identical = comparable and all(r['evidence'] == results[0]['evidence'] for r in results)
    success = comparable and all(r['evidence']['success'] for r in results)
    summary = {'kind': 'SlotHashes_materiality_experiment', 'variants': len(results), 'all_submitted_to_vm': comparable,
               'execution_evidence_byte_identical': identical, 'all_succeeded': success,
               'full_T1_materiality_proven': identical and success,
               'evidence_hashes': {r['variant']: lut.sha(lut.canonical(r['evidence'])) if r['submitted_to_vm'] else None for r in results},
               'production_replay_eligible': False, 'baseline_fidelity_matched': False}
    (output / 'summary.json').write_bytes(lut.canonical(summary))
    return summary


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    print(lut.canonical(run(args.output.resolve())).decode())
