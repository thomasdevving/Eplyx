#!/usr/bin/env python3
"""Rebuild the U3D.2 binary and partial-state evidence without transport."""
import base64
import json
from pathlib import Path
import tempfile

import kamino_u3b_lut as lut
import kamino_u3c_dependencies as dependencies
import kamino_u3d_state as state


def verify_binaries(evidence):
    with tempfile.TemporaryDirectory(prefix='eplyx-u3d2-offline-') as tmp:
        tmp = Path(tmp)
        if (evidence / 'capture.tar.gz').exists():
            root = tmp / 'capture'
            dependencies.unpack(evidence, root)
            for path in evidence.rglob('*.json'):
                if path.name == 'capture-index.json':
                    continue
                relative = path.relative_to(evidence)
                lut.require((root / relative).read_bytes() == path.read_bytes(), 'retained binary metadata differs from archive')
        else:
            root = evidence
        receipt, primary, cache = dependencies.validate_capture(root)
        used = set()
        for target in receipt['target_results']:
            sig = target['signature']
            result = dependencies.replay_resolver(primary[sig], cache, tmp / 'derived' / sig, used)
            lut.require(result == lut.read(root / target['result_file']), 'binary resolver result differs')
            for program in result['programs']:
                if program['source'] == 'historical_mainnet':
                    file = program['binary_file']
                    original = (root / sig / file).read_bytes()
                    rebuilt = (tmp / 'derived' / sig / file).read_bytes()
                    lut.require(original == rebuilt and lut.sha(rebuilt) == program['provenance']['sha256'],
                                'historical binary image differs')
        lut.require(used == set(cache), 'unused binary request membership')
        return len(cache)


def price_diff(root):
    receipt = lut.read(root / 'receipt.json')
    success = [r for r in receipt['requests'] if r['method'] == 'getAccountInfo' and not r['transport_error']]
    lut.require(len(success) == 2 and success[0]['params'][0] == success[1]['params'][0], 'partial price evidence differs')
    data = [base64.b64decode(lut.read(root / r['response_file'])['result']['value']['data'][0], validate=True) for r in success]
    lut.require(len(data[0]) == len(data[1]), 'price account length changed')
    return {'kind': 'archive_boundary_raw_diff_not_local_execution', 'address': success[0]['params'][0],
            'pre_slot': success[0]['params'][1]['slot'], 'post_slot': success[1]['params'][1]['slot'],
            'data_bytes': len(data[0]), 'pre_data_sha256': lut.sha(data[0]), 'post_data_sha256': lut.sha(data[1]),
            'changed_bytes': [{'offset': i, 'pre': a, 'post': b} for i, (a, b) in enumerate(zip(*data)) if a != b],
            'local_scope_execution_proven': False, 'scope_to_klend_causal_effect_proven': False}


def cohort_artifacts(binary_root, state_root):
    binary_receipt = lut.read(binary_root / 'receipt.json')
    state_receipt = lut.read(state_root / 'receipt.json')
    captured = lut.read(state_root / 'result.json')
    rows, inventories = [], []
    for index, target in enumerate(captured['targets'], 1):
        sig = target['signature']
        binary = lut.read(binary_root / sig / 'result.json')
        row = lut.read(state.envelope.OUTPUT / f'transactions/{sig}.json')
        tx = row['transaction']
        failure = next((r for r in binary_receipt['requests'] if r['status'] == 'failure'
                        and r['method'] == 'getAccountInfo' and r['params'][1]['slot'] == tx['slot'] - 1), None)
        rows.append({'target': f'T{index}', 'signature': sig, 'slot': tx['slot'],
            'C1': 'passed', 'C2': 'passed', 'C3': 'passed',
            'C4': 'passed' if binary['C4_binaries_identified'] else 'failed',
            'C5': 'partial_acquisition_blocked' if target['acquired'] else 'not_attempted',
            **{f'C{k}': 'not_attempted' for k in range(6, 11)},
            'binary_failure_request': failure,
            'state_failure': target['failure'] if binary['C4_binaries_identified'] else None,
            'account_boundaries_acquired': len(target['acquired']),
            'complete_historical_state_proven': False, 'runtime_executed': False,
            'production_replay_eligible': False})
        facts = {(a['address'], a['boundary']): a['facts'] for a in target['acquired']}
        accounts = []
        for number, key in enumerate(tx['account_keys']):
            refs = [{'outer_index': ix, 'account_position': pos,
                     'instruction_identity': row['envelope']['instructions'][ix]['identity'],
                     'instruction_role': row['envelope']['instructions'][ix]['role']}
                    for ix, instruction in enumerate(tx['instructions'])
                    for pos, account in enumerate(instruction['accounts']) if account['address'] == key['address']]
            accounts.append(dict(key, message_index=number,
                message_origin='static' if number < len(tx['account_keys']) - tx['loaded_address_count'] else 'lookup_loaded',
                instruction_references=refs, pre_state=facts.get((key['address'], 'pre')),
                post_reference=facts.get((key['address'], 'post'))))
        inventories.append({'target': f'T{index}', 'signature': sig, 'slot': tx['slot'],
            'frozen_transaction_sha256': row['frozen_transaction_sha256'], 'lut_proof_id': row['frozen_lut_proof_id'],
            'inventory_scope': 'Every original message account and ordered instruction reference; state proof and hidden runtime dependency closure not complete',
            'accounts': accounts})
    stages = {'rows': rows, 'primary_metrics': {'classified': 4, 'envelope_admitted': 4,
        'complete_observed_binary_sets': sum(r['C4'] == 'passed' for r in rows),
        'targets_with_partial_state_capture': sum(bool(r['account_boundaries_acquired']) for r in rows),
        'complete_historical_state_sets': 0, 'execution_attempted': 0, 'outcome_matched': 0,
        'post_state_matched': 0, 'production_assured_subjects': 0},
        'first_blocker_distribution': {'Binary': 3, 'HistoricalState': 1}, 'T1_decision': 'T1-D',
        'scope_change': 'All four independently attempted at user request; no dependent runtime stage bypassed.'}
    summary = {'binary_requests': len(binary_receipt['requests']),
        'binary_successes': sum(r['status'] == 'success' for r in binary_receipt['requests']),
        'binary_failures': sum(r['status'] == 'failure' for r in binary_receipt['requests']),
        'state_transport_requests': len(state_receipt['requests']),
        'state_transport_successes': sum(r['transport_error'] is None for r in state_receipt['requests']),
        'state_transport_failures': sum(r['transport_error'] is not None for r in state_receipt['requests']),
        'actual_account_state_rpc_attempts': sum(r['method'] == 'getAccountInfo' for r in state_receipt['requests']),
        'distinct_state_account_boundaries_acquired': sum(len(t['acquired']) for t in captured['targets']),
        'distinct_state_accounts_acquired': len({a['address'] for t in captured['targets'] for a in t['acquired']}),
        'total_network_requests': len(binary_receipt['requests']) + len(state_receipt['requests']),
        'binary_timing': lut.read(binary_root / 'timing.json'), 'state_timing': lut.read(state_root / 'timing.json'),
        'capture_archive': lut.read(binary_root / 'capture-index.json')['compressed_bytes'],
        'expanded_binary_capture_bytes': lut.read(binary_root / 'capture-index.json')['expanded_bytes'],
        'original_U3C_attempt_preserved': True, 'earlier_U3D_checkpoint_preserved': True,
        'transport_failure_interpretation': 'Measured zero-byte response failures; not nonexistent programs or permanently missing history.'}
    return {'stage-table.json': stages, 'account-inventories.json': inventories, 'attempt-summary.json': summary}


def main():
    evidence = lut.REPO / 'docs/examples/phase-u3d2-binaries'
    prior = lut.read(lut.REPO / 'docs/examples/phase-u3d2-validation/preimplementation.json')
    for path, digest in prior['prior_checkpoint_hashes'].items():
        lut.require(lut.sha((lut.REPO / path).read_bytes()) == digest, 'prior U3D checkpoint was overwritten')
    count = verify_binaries(evidence)
    root = lut.REPO / 'docs/examples/phase-u3d2-state'
    result = state.verify(root, evidence)
    diff = price_diff(root)
    expected = lut.REPO / 'docs/examples/phase-u3d2-validation/scope-price-boundary-diff.json'
    lut.require(expected.read_bytes() == lut.canonical(diff), 'raw price diff artifact differs')
    for name, value in cohort_artifacts(evidence, root).items():
        lut.require((expected.parent / name).read_bytes() == lut.canonical(value), f'cohort artifact differs: {name}')
    print(json.dumps({'offline': True, 'binary_requests_replayed': count,
                      'state_transport_attempts_verified': len(lut.read(root / 'receipt.json')['requests']),
                      'acquired_boundaries': [len(r['acquired']) for r in result['targets']],
                      'runtime_executed': False}))


if __name__ == '__main__':
    main()
