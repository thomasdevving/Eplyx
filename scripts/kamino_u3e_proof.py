#!/usr/bin/env python3
"""Recompute captured T1 boundary facts and rerun the existing slot screen."""
import base64
import json
import os
from pathlib import Path
import subprocess

import kamino_u3e_state as s

lut = s.lut
VERIFIER = lut.REPO / 'target/debug/examples/verify_envelope_state'
BLOCK_PARAMS = [s.inventory.SLOT, {'encoding': 'json', 'transactionDetails': 'accounts', 'rewards': False,
                                  'commitment': 'finalized', 'maxSupportedTransactionVersion': 1}]


def run_verifier(value):
    run = subprocess.run([str(VERIFIER)], input=lut.canonical(value), capture_output=True)
    lut.require(run.returncode == 0, run.stderr.decode(errors='replace'))
    return json.loads(run.stdout)


def boundary_facts():
    root = s.ROOT / 'phase-u3e-state'
    result = s.run(root, verify=True)
    lut.require(result['raw_boundary_capture_complete'], 'complete raw boundaries required')
    row, binaries, inventory = s.inputs()
    receipt = lut.read(root / 'receipt.json')
    accounts = {(a['account'], a['requested_slot']): lut.read(root / a['body_file'])['result']['value']
                for a in receipt['attempts'] if a['account'] and a['failure_class'] is None}
    typed, comparisons = [], []
    for entry in inventory['account_rows']:
        address = entry['address']
        if (address, s.inventory.SLOT - 1) not in accounts:
            continue
        pre, post = [accounts[(address, slot)] for slot in (s.inventory.SLOT - 1, s.inventory.SLOT)]
        lut.require((pre is None) == (post is None), 'unexpected lifecycle change')
        if pre is None:
            lut.require('Authority' in entry['categories'], 'missing required account')
            comparisons.append({'address': address, 'present_at_both_boundaries': False, 'authority_absence_proven': True})
            continue
        lut.require(pre['owner'] == post['owner'] and pre['executable'] == post['executable'], 'unexpected owner/executable change')
        if not entry['is_writable']:
            lut.require(pre == post, 'readonly boundary account changed')
        predata, postdata = [base64.b64decode(a['data'][0]) for a in (pre, post)]
        comparisons.append({'address': address, 'is_writable': entry['is_writable'],
                            'pre_data_sha256': lut.sha(predata), 'post_data_sha256': lut.sha(postdata),
                            'pre_lamports': pre['lamports'], 'post_lamports': post['lamports'],
                            'changed_byte_offsets': [i for i in range(max(len(predata), len(postdata))) if predata[i:i+1] != postdata[i:i+1]],
                            'post_reference_basis': 'exact end-of-S plus validator balances; conditional on final clean slot screen',
                            'local_post_state_compared': False})
        facts = next(a['facts'] for a in result['acquired'] if a['address'] == address and a['boundary'] == 'pre')
        kind = facts.get('anchor_type') if 'ProtocolState' in entry['categories'] else next((c for c in entry['categories'] if c in ('Mint', 'TokenAccount')), None)
        if kind:
            typed.extend({'address': address, 'boundary': boundary, 'type': kind, 'account': account}
                         for boundary, account in [('pre', pre), ('post', post)])
    decoded = run_verifier({'mode': 'decode', 'accounts': typed})
    scope = row['transaction']['instructions'][0]['accounts']
    price, mapping, twap = [base64.b64decode(accounts[(a['address'], s.inventory.SLOT - 1)]['data'][0]) for a in scope[:3]]
    lut.require(len(price) == 40 + 512 * 56 and len(mapping) == 8 + 512 * 58 and len(twap) == 72 + 512 * 672, 'Scope source layout size differs')
    lut.require(price[8:40] == lut.baseline.b58decode(scope[1]['address']) and twap[8:40] == lut.baseline.b58decode(scope[0]['address'])
                and twap[40:72] == lut.baseline.b58decode(scope[1]['address']), 'Scope has_one relationships differ')
    scope_entries = []
    for token, remaining in zip(inventory['scope_tokens'], scope[4:]):
        lut.require(mapping[8 + 32 * token:40 + 32 * token] == lut.baseline.b58decode(remaining['address']), 'Scope mapping/remaining identity differs')
        scope_entries.append({'token': token, 'raw_oracle_type': mapping[8 + 512 * 32 + token],
                              'twap_enabled_bitmask': mapping[8 + 512 * 35 + token],
                              'generic_hex': mapping[8 + 512 * 38 + 20 * token:8 + 512 * 38 + 20 * (token + 1)].hex(),
                              'remaining_account': remaining['address']})
    return row, inventory, {'kind': 'u3e_boundary_relationship_checks', 'typed_checks': decoded,
                           'boundary_comparisons': comparisons, 'scope_entries': scope_entries,
                           'scope_relationships_match': True, 'raw_boundary_capture_complete': True,
                           'runtime_context_complete': False, 'runtime_executed': False}


def screen(root, verify=False):
    row, inventory, facts = boundary_facts()
    required = sorted(a['address'] for a in inventory['account_rows'] if not a['runtime_provided'])
    lut.require(len(required) == len(set(required)) == 26, 'final message/programdata closure differs')
    def check(block):
        return run_verifier({'mode': 'screen', 'slot': row['transaction']['slot'], 'signature': s.inventory.SIG,
                             'required_accounts': required, 'block': {'params': BLOCK_PARAMS, 'result': block}})
    qualified = lut.read(s.state.envelope.U3B / 'acquisition.json')['provider']
    if verify:
        receipt = lut.read(root / 'receipt.json')
        lut.require(receipt['qualification'] == qualified and receipt['provider'] == qualified['scheme_host'], 'archive qualification differs')
        groups = s.transport.verify(root, [('getBlock', BLOCK_PARAMS)], {s.transport.request_id('getBlock', BLOCK_PARAMS): check})
        attempts = groups[s.transport.request_id('getBlock', BLOCK_PARAMS)]
        failure = attempts[-1]['failure_class']
        result = None if failure else check(lut.read(root / attempts[-1]['body_file'])['result'])
    else:
        rpc = s.transport.CurlArchive(os.environ['SOLANA_ARCHIVE_RPC_URL'], os.environ.get('SOLANA_ARCHIVE_RPC_ORIGIN', ''))
        lut.require(rpc.provider == qualified['scheme_host'], 'previously qualified provider required')
        client = s.transport.EvidenceClient(root, rpc, lut.read(s.transport.POLICY_PATH), qualified)
        _, result, failure = client.call('getBlock', BLOCK_PARAMS, check)
    facts.update(final_required_accounts=required, final_same_slot_screen=result, failure=failure)
    if verify:
        lut.require((root / 'result.json').read_bytes() == lut.canonical(facts), 'derived state proof differs')
    else:
        client.finish(facts)
    return facts


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--verify', action='store_true')
    args = parser.parse_args()
    print(lut.canonical(screen(args.output.resolve(), args.verify)).decode())
