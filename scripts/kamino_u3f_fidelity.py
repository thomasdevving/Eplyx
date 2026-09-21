#!/usr/bin/env python3
"""Strict T1 outcome and raw account reconciliation; no tolerance or RPC."""
import base64
import json
from pathlib import Path
import subprocess

import kamino_u3f as f

lut = f.lut


def compare(payload, execution, references):
    lut.require(execution.get('original_instructions_executed') is True and not execution.get('diagnostic_control'), 'diagnostic/reduced envelope cannot claim historical fidelity')
    observed = execution['evidence']
    lut.require(observed['native_message'] == payload['proof']['native_v0_message'], 'original native message identity differs')
    lut.require(observed['account_vector'] == [a['address'] for a in payload['proof']['full_account_keys']], 'resolved account vector differs')
    original = payload['frozen']['result']['meta']
    required = set(payload['watch'])
    lut.require(set(observed['post_accounts']) == set(references) == required, 'complete watched post-state required')
    for index, address in enumerate(observed['account_vector']):
        if address in references:
            reference = references[address]
            lut.require((reference['lamports'] if reference else 0) == original['postBalances'][index], 'post reference contradicts validator balance')
    failures = []
    if observed['success'] != (original['err'] is None) or observed['error'] is not None:
        failures.append('outcome')
    if observed['fee'] != original['fee']:
        failures.append('fee')
    if observed['logs'] != original['logMessages']:
        failures.append('logs')
    expected_inner = [{'outer_index': group['index'], 'instructions': [
        {'program_id_index': i['programIdIndex'], 'accounts': i['accounts'], 'data_hex': lut.baseline.b58decode(i['data']).hex(), 'stack_height': i['stackHeight']} for i in group['instructions']]} for group in original['innerInstructions']]
    actual_inner = [g for g in observed['inner_instructions'] if g['instructions']]
    if actual_inner != expected_inner:
        failures.append('inner_instructions')
    expected_return = original.get('returnData')
    if expected_return is None:
        # Agave records no return-data object when its byte payload is empty.
        # LiteSVM retains the last program ID even with an empty payload. Keep
        # that raw evidence, but compare using the validator's representation.
        if observed['return_data']['data_base64'] != '':
            failures.append('return_data')
    elif observed['return_data'] != {'program': expected_return['programId'], 'data_base64': expected_return['data'][0]}:
        failures.append('return_data')
    accounts = []
    for address in payload['watch']:
        expected, actual = references[address], observed['post_accounts'][address]
        changed = []
        if expected is None or actual is None:
            if expected is not actual:
                changed.append('lifecycle')
        else:
            for field in ('owner', 'lamports', 'executable', 'rentEpoch', 'data'):
                if expected[field] != actual[field]:
                    changed.append(field)
        if changed:
            failures.append(f'account:{address}:{",".join(changed)}')
        accounts.append({'address': address, 'matched': not changed, 'different_fields': changed,
                         'expected_data_sha256': lut.sha(base64.b64decode(expected['data'][0])) if expected else None,
                         'actual_data_sha256': lut.sha(base64.b64decode(actual['data'][0])) if actual else None})
    return {'kind': 'strict_native_v0_historical_fidelity', 'decision': 'T1-A' if not failures else 'T1-B' if observed['success'] else 'T1-C',
            'fidelity': 'matched' if not failures else 'mismatched', 'failures': failures, 'watched_accounts': accounts,
            'outcome_matched': 'outcome' not in failures, 'fee_matched': 'fee' not in failures,
            'logs_byte_identical': 'logs' not in failures, 'inner_instructions_exact': 'inner_instructions' not in failures,
            'return_data_matched': 'return_data' not in failures,
            'return_data_representation': 'Agave omits empty payloads; LiteSVM last-program ID retained as diagnostic when data is empty',
            'compute_units': {'original': original['computeUnitsConsumed'], 'local': observed['compute_units'], 'diagnostic_match': original['computeUnitsConsumed'] == observed['compute_units']},
            'raw_post_state_matched': all(a['matched'] for a in accounts), 'tolerance_used': False,
            'runtime_managed_exclusions_added': [], 'Instructions_sysvar': 'runtime-generated context, no historical ordinary-account post reference; exact native envelope and Scope check retained'}


def references(payload):
    pre = {i['address']: i['account'] for i in payload['seeds'] if i['address'] in payload['watch']}
    pre.update({address: None for address in payload['absent']})
    post = dict(pre)
    root = f.ROOT / 'phase-u3e-state'
    for attempt in lut.read(root / 'receipt.json')['attempts']:
        if attempt['method'] == 'getAccountInfo' and attempt['requested_slot'] == f.s.inventory.SLOT and not attempt['failure_class']:
            post[attempt['account']] = lut.read(root / attempt['body_file'])['result']['value']
    return pre, post


def derive():
    payload, manifest = f.prepare()
    pre, post = references(payload)
    execution = lut.read(f.ROOT / 'phase-u3f-materiality-attempt-1/empty.json')
    fidelity = compare(payload, execution, post)
    lut.require(fidelity['fidelity'] == 'matched', 'strict historical fidelity failed; no semantics')
    # Existing U2 labels and evaluator, full original transaction retained.
    row, _, _ = f.s.inventory.frozen_inputs()
    semantic_input = {'transaction': row['transaction'], 'watch': payload['watch'], 'absent': payload['absent'],
                      'pre': pre, 'post': execution['evidence']['post_accounts'], 'expected_post': post, 'outcome': execution['evidence']}
    proc = subprocess.run([str(lut.REPO / 'target/debug/examples/evaluate_envelope_semantics')], input=lut.canonical(semantic_input), capture_output=True)
    lut.require(proc.returncode == 0, proc.stderr.decode(errors='replace'))
    semantics = json.loads(proc.stdout)
    return fidelity, semantics


if __name__ == '__main__':
    fidelity, semantics = derive()
    (f.VALIDATION / 'T1-fidelity.json').write_bytes(lut.canonical(fidelity))
    (f.VALIDATION / 'T1-semantics.json').write_bytes(lut.canonical(semantics))
    print(lut.canonical({'decision': fidelity['decision'], 'watch_count': len(fidelity['watched_accounts']), 'semantics': semantics}).decode())
