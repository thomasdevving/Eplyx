#!/usr/bin/env python3
"""Bounded primary-cohort state capture and transport-free revalidation.

This establishes raw account acquisition and checks validator lamports/owners.
It is not full protocol-state proof, runtime-context proof or replay admission.
"""
import argparse
import base64
import importlib.util
import json
import os
from pathlib import Path
import time

import kamino_u3b_lut as lut
import kamino_u3c_envelope as envelope
import kamino_u3d_inventory as inventory

spec = importlib.util.spec_from_file_location('u3d_raw_archive', Path(__file__).with_name('capture-kamino-u3b-luts.py'))
transport = importlib.util.module_from_spec(spec)
spec.loader.exec_module(transport)
INSTRUCTIONS = inventory.INSTRUCTIONS


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(lut.canonical(value))


def targets(binary_root):
    inventory.frozen_inputs()
    summary = lut.read(envelope.OUTPUT / 'summary.json')
    rows = []
    for sig in summary['primary_signatures']:
        row = lut.read(envelope.OUTPUT / f'transactions/{sig}.json')
        fresh = binary_root / sig / 'result.json'
        old = lut.REPO / f'docs/examples/phase-u3c-dependencies/resolved/{sig}.json'
        result = lut.read(fresh) if fresh.exists() else {}
        source = fresh
        # A failed new request does not erase an already revalidated complete
        # capture at this exact predecessor slot. Never substitute another slot.
        if not result.get('C4_binaries_identified') and lut.read(old).get('C4_binaries_identified'):
            result, source = lut.read(old), old
        rows.append((row, result, source))
    return rows


def state_plan(row, binaries):
    tx = row['transaction']
    screen = binaries.get('slot_screening')
    lut.require(binaries.get('C4_binaries_identified') is True, 'historical_binary_set_incomplete')
    lut.require(binaries['signature'] == tx['signature'] and binaries['pre_slot'] == tx['slot'] - 1,
                'historical_binary_context_mismatch')
    required = {a['address'] for a in tx['account_keys'] if a['address'] != INSTRUCTIONS}
    required.update(p['provenance']['programdata_address'] for p in binaries['programs']
                    if p.get('provenance', {}).get('programdata_address'))
    lut.require(screen and screen['slot'] == tx['slot'] and screen['target_signature'] == tx['signature']
                and not screen['conflicts'] and required <= set(screen['required_accounts']),
                'same_slot_screen_incomplete_or_ambiguous')
    invoked = {p['program_id'] for p in binaries['programs']}
    addresses = [a['address'] for a in tx['account_keys'] if a['address'] not in invoked and a['address'] != INSTRUCTIONS]
    scope = [a['address'] for a in tx['instructions'][0]['accounts'][:3]]
    ordered = scope + [a for a in addresses if a not in scope]
    lut.require(set(ordered) == set(addresses) and len(ordered) == len(addresses), 'state_inventory_incomplete')
    return [{'address': address, 'boundary': boundary, 'slot': slot,
             'method': 'getAccountInfo',
             'params': [address, {'encoding': 'base64', 'commitment': 'finalized', 'slot': slot}]}
            for address in ordered for boundary, slot in [('pre', tx['slot'] - 1), ('post', tx['slot'])]]


def account_check(row, item, response):
    """Return facts only; reject contradictions before spending more requests."""
    tx = row['transaction']
    lut.require(response.get('context', {}).get('slot') == item['slot'], 'wrong_historical_context')
    index = next(i for i, key in enumerate(tx['account_keys']) if key['address'] == item['address'])
    expected_lamports = tx[f"{item['boundary']}_balances"][index]
    account = response.get('value')
    if account is None:
        lut.require(expected_lamports == 0, 'historical_account_missing_with_nonzero_validator_balance')
        return {'present': False, 'validator_lamports': 0, 'absence_requires_pair_validation': True}
    lut.require(isinstance(account, dict), 'malformed_account')
    lut.require(type(account.get('lamports')) is int and account['lamports'] == expected_lamports,
                'historical_lamports_contradict_validator_boundary')
    lut.require(type(account.get('executable')) is bool and type(account.get('rentEpoch')) is int,
                'malformed_account_flags')
    lut.require(len(lut.baseline.b58decode(account.get('owner', ''))) == 32, 'malformed_account_owner')
    encoded = account.get('data')
    lut.require(isinstance(encoded, list) and len(encoded) == 2 and encoded[1] == 'base64', 'wrong_account_encoding')
    data = base64.b64decode(encoded[0], validate=True)
    if 'space' in account:
        lut.require(account['space'] == len(data), 'partial_account_data')
    scope_accounts = [a['address'] for a in tx['instructions'][0]['accounts'][:3]]
    if item['address'] in scope_accounts:
        lut.require(account['owner'] == envelope.SCOPE and not account['executable'], 'wrong_scope_state_owner')
    token = next((t for t in tx[f"{item['boundary']}_token_balances"] if t['account_index'] == index), None)
    if token:
        lut.require(account['owner'] == token['program_id'] and not account['executable'], 'wrong_token_state_owner')
        # SPL/Token-2022 base account layout; extensions require later validation.
        lut.require(len(data) >= 165 and data[:32] == lut.baseline.b58decode(token['mint'])
                    and int.from_bytes(data[64:72], 'little') == token['amount'],
                    'token_bytes_contradict_validator_boundary')
    ata = tx['instructions'][1]['accounts']
    if item['address'] == ata[1]['address']:
        lut.require(len(data) >= 165 and account['owner'] == ata[5]['address']
                    and data[:32] == lut.baseline.b58decode(ata[3]['address'])
                    and data[32:64] == lut.baseline.b58decode(ata[2]['address'])
                    and data[108] == 1, 'ata_historical_bytes_contradict_existing_account_model')
    return {'present': True, 'owner': account['owner'], 'lamports': account['lamports'],
            'executable': account['executable'], 'rent_epoch': account['rentEpoch'],
            'data_bytes': len(data), 'data_sha256': lut.sha(data),
            'raw_account_json_sha256': lut.sha(lut.canonical(account)),
            'context_slot': response['context']['slot'], 'validator_lamports_match': True}


def evaluate(rows, call):
    results = []
    genesis, error = call('getGenesisHash', [])
    genesis_error = error or (None if genesis == lut.GENESIS else 'archive_genesis_mismatch')
    for row, binaries, source in rows:
        tx = row['transaction']
        result = {'signature': tx['signature'], 'slot': tx['slot'], 'binary_evidence_source': str(source.relative_to(lut.REPO)),
                  'binary_evidence_sha256': lut.sha(source.read_bytes()) if source.exists() else None,
                  'planned': [], 'acquired': [], 'failure': None, 'raw_state_capture_complete': False,
                  'complete_historical_state_proven': False, 'runtime_executed': False}
        try:
            result['planned'] = state_plan(row, binaries)
            lut.require(genesis_error is None, genesis_error or 'genesis_failure')
            for item in result['planned']:
                response, error = call(item['method'], item['params'])
                if error:
                    result['failure'] = {'reason': error, 'request': item}
                    break
                try:
                    facts = account_check(row, item, response)
                except (ValueError, TypeError, KeyError) as exc:
                    result['failure'] = {'reason': str(exc), 'request': item}
                    break
                result['acquired'].append(dict(item, facts=facts))
            result['raw_state_capture_complete'] = result['failure'] is None
        except ValueError as exc:
            result['failure'] = {'reason': str(exc), 'request': None}
        results.append(result)
    return {'kind': 'experimental_primary_raw_state_capture', 'sample_fingerprint': lut.FINGERPRINT,
            'targets': results, 'runtime_executed': False, 'production_replay_eligible': False}


def capture(root, binary_root):
    lut.require(not root.exists(), 'one immutable directory per capture attempt')
    rows = targets(binary_root)
    rpc = transport.RawArchive(os.environ['SOLANA_ARCHIVE_RPC_URL'], os.environ.get('SOLANA_ARCHIVE_RPC_ORIGIN', ''))
    qualified = lut.read(envelope.U3B / 'acquisition.json')['provider']
    lut.require(rpc.identity == qualified['scheme_host'], 'select previously qualified archive')
    root.mkdir(parents=True)
    receipt = {'kind': 'u3d2_bounded_state_transport', 'complete': False, 'provider': rpc.identity,
               'archive_validation_sha256': qualified['validation_artifact_sha256'],
               'sample_fingerprint': lut.FINGERPRINT, 'max_attempts_per_context': 2,
               'retry_only': ['transport_exit_*', 'transport_timeout', 'invalid_json_response'],
               'fallback': False, 'requests': [], 'raw_artifact_hashes': {}}
    write(root / 'receipt.json', receipt)
    started = time.perf_counter()
    cache = {}
    def call(method, params):
        key = lut.sha(lut.canonical([method, params]))
        if key in cache:
            return cache[key]
        for attempt in (1, 2):
            start = time.perf_counter()
            body, value, error = rpc.call(method, params)
            elapsed = time.perf_counter() - start
            ref = f'rpc/{key}-{attempt}.body'
            withheld = False
            try:
                transport.safe_body(body)
            except ValueError:
                withheld, error, ref = True, 'sensitive_response_withheld', None
            if ref:
                path = root / ref
                path.parent.mkdir(exist_ok=True)
                path.write_bytes(body)  # Includes explicit zero-byte artifacts.
                receipt['raw_artifact_hashes'][ref] = lut.sha(body)
            receipt['requests'].append({'request_id': key, 'method': method, 'params': params,
                'attempt': attempt, 'transport_error': error, 'response_file': ref,
                'response_sha256': lut.sha(body), 'response_bytes': len(body),
                'withheld': withheld, 'seconds': elapsed})
            write(root / 'receipt.json', receipt)
            retryable = error and (error.startswith('transport_exit_') or error in ('transport_timeout', 'invalid_json_response'))
            if not retryable or attempt == 2:
                cache[key] = (None if error else value['result'], error)
                return cache[key]
    result = evaluate(rows, call)
    write(root / 'result.json', result)
    receipt['complete'] = True
    write(root / 'receipt.json', receipt)
    write(root / 'timing.json', {'wall_seconds': time.perf_counter() - started,
                               'transport_seconds': sum(r['seconds'] for r in receipt['requests']),
                               'requests': len(receipt['requests'])})
    return result


def verify(root, binary_root):
    receipt = lut.read(root / 'receipt.json')
    qualified = lut.read(envelope.U3B / 'acquisition.json')['provider']
    lut.require(receipt['complete'] and receipt['provider'] == qualified['scheme_host']
                and receipt['archive_validation_sha256'] == qualified['validation_artifact_sha256']
                and receipt['sample_fingerprint'] == lut.FINGERPRINT
                and receipt['max_attempts_per_context'] == 2 and receipt['fallback'] is False,
                'capture qualification or bounded policy differs')
    lut.require({str(p.relative_to(root)) for p in (root / 'rpc').glob('*')} == set(receipt['raw_artifact_hashes']), 'raw membership differs')
    cache, attempts = {}, {}
    for request in receipt['requests']:
        key = lut.sha(lut.canonical([request['method'], request['params']]))
        attempt = attempts.get(key, 0) + 1
        lut.require(key == request['request_id'] and request['attempt'] == attempt <= 2, 'retry bound or request identity differs')
        if attempt > 1:
            old = cache[key][1]
            lut.require(old and (old.startswith('transport_exit_') or old in ('transport_timeout', 'invalid_json_response')), 'unpermitted retry')
        attempts[key] = attempt
        ref = request['response_file']
        body = lut.safe_file(root, ref).read_bytes() if ref else b''
        if ref:
            lut.require(lut.sha(body) == request['response_sha256'] == receipt['raw_artifact_hashes'][ref]
                        and len(body) == request['response_bytes'], 'raw response hash or length mismatch')
        else:
            lut.require(request['withheld'] and request['transport_error'] == 'sensitive_response_withheld', 'missing raw body')
        error = request['transport_error']
        if not error:
            value = json.loads(body)
            lut.require(value.get('error') is None and 'result' in value, 'invalid successful RPC envelope')
            cache[key] = value['result'], None
        else:
            cache[key] = None, error
    used = set()
    def call(method, params):
        key = lut.sha(lut.canonical([method, params]))
        if key not in cache:
            # Missing retained evidence is a verifier failure, not a measured
            # provider error to be caught by evaluate's per-target stop logic.
            raise RuntimeError('uncaptured request; no current-state fallback')
        used.add(key)
        return cache[key]
    result = evaluate(targets(binary_root), call)
    lut.require(used == set(cache), 'unused request membership')
    lut.require(lut.canonical(result) == (root / 'result.json').read_bytes(), 'derived state result differs')
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--binaries', type=Path, required=True)
    parser.add_argument('--verify', action='store_true')
    args = parser.parse_args()
    result = (verify if args.verify else capture)(args.output.resolve(), args.binaries.resolve())
    print(json.dumps({'verified': args.verify, 'targets': [{'signature': r['signature'],
        'acquired_boundaries': len(r['acquired']), 'raw_state_capture_complete': r['raw_state_capture_complete'],
        'failure': r['failure']} for r in result['targets']]}, indent=2))


if __name__ == '__main__':
    main()
