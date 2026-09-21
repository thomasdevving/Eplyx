#!/usr/bin/env python3
"""Exact T1 boundary acquisition. Raw acquisition never seals runtime inputs."""
import base64
import hashlib
import os
from pathlib import Path

import historical_transport as transport
import kamino_u3d_inventory as inventory
import kamino_u3d_state as state

lut = transport.lut
ROOT = lut.REPO / 'docs/examples'
NATIVE = 'NativeLoader1111111111111111111111111111111'
LOADERS = {'BPFLoader2111111111111111111111111111111111', 'BPFLoaderUpgradeab1e11111111111111111111111'}


def inputs():
    row, binaries, proof = inventory.frozen_inputs()
    derived = inventory.derive(row, binaries, proof, inventory.captured_headers())
    plan = state.state_plan(row, binaries)
    tx = row['transaction']
    for account in derived['account_rows']:
        classes, roles = account['classes'], {r['role'] for r in account.get('roles', [])}
        categories = []
        mapping = {'programdata': 'ProgramData', 'sysvar': 'Sysvar', 'runtime_provided': 'RuntimeProvided',
                   'protocol_state': 'ProtocolState', 'oracle_state': 'OracleState', 'token_account': 'TokenAccount',
                   'mint': 'Mint', 'authority_pda_presence_unknown': 'Authority'}
        categories.extend(value for key, value in mapping.items() if key in classes)
        if 'program' in classes:
            categories.append('ProgramBinary' if account['observed_invocation'] and account['address'] not in (inventory.SYSTEM, 'ComputeBudget111111111111111111111111111111') else 'StandardProgram')
        if account.get('message_index') == 0:
            categories.extend(['Payer', 'Authority'])
        account['categories'] = categories or ['Other']
        account['PreStateBoundary'] = {'slot': inventory.SLOT - 1, 'kind': 'historical_account'} if not account['runtime_provided'] else {'kind': 'original_execution_instruction_context'}
        account['PostStateReference'] = {'slot': inventory.SLOT, 'kind': 'historical_account_and_validator_metadata'} if any(p['address'] == account['address'] for p in plan) else None
        account['execution_seed_admitted'] = False
    derived.update(kind='u3e_recomputed_T1_inventory', planned_boundary_requests=plan,
                   runtime_requirements=[{'name': name, 'status': 'not_yet_proven', 'category': 'RuntimeProvided'}
                                         for name in ('Clock', 'Rent', 'SlotHashes_if_required', 'epoch', 'feature_profile', 'recent_blockhash', 'native_programs', 'loaders', 'compute_budget')])
    return row, binaries, derived


def known_hashes():
    result = {}
    old = lut.read(ROOT / 'phase-u3d2-state/result.json')
    for target in old['targets']:
        if target['signature'] == inventory.SIG:
            for account in target['acquired']:
                result[(account['address'], account['slot'])] = account['facts']['data_sha256']
    probe = lut.read(ROOT / 'phase-u3e-mapping-probe/result.json')
    lut.require(probe['probe_passed'], 'exact mappings probe must pass before full capture')
    result[(probe['request']['address'], probe['request']['slot'])] = probe['facts']['data_sha256']
    return result


def permits_absence(row, item):
    tx = row['transaction']
    address = tx['instructions'][5]['accounts'][3]['address']
    # Verify the interface role, rather than assuming an arbitrary zero balance is absent.
    labels = inventory.role_names()
    lut.require(labels[3] == 'lending-market-authority', 'authority interface changed')
    index = next(i for i, a in enumerate(tx['account_keys']) if a['address'] == item['address'])
    return item['address'] == address and tx['pre_balances'][index] == tx['post_balances'][index] == 0


def validate(row, derived, item, response):
    lut.require(item in derived['planned_boundary_requests'], 'unknown boundary or current/post-state seed substitution')
    facts = state.account_check(row, item, response)
    account = response['value']
    if account is None:
        lut.require(permits_absence(row, item), 'required historical state absent')
        return facts
    tx = row['transaction']
    entry = next(a for a in derived['account_rows'] if a['address'] == item['address'])
    categories = entry['categories']
    roles = {r['role'] for r in entry['roles']}
    data = base64.b64decode(account['data'][0], validate=True)
    expected = entry.get('expected_owner')
    discriminator = None
    if 'OracleState' in categories:
        discriminator = next(name for role, name in [('oracle-prices', 'OraclePrices'), ('oracle-mappings', 'OracleMappings'), ('oracle-twaps', 'OracleTwaps')] if role in roles)
    if 'ProtocolState' in categories:
        discriminator = 'Reserve' if 'reserve' in roles else 'Obligation' if 'obligation' in roles else 'LendingMarket'
        if discriminator in ('Reserve', 'Obligation'):
            lut.require(len(data) == {'Reserve': 8624, 'Obligation': 3344}[discriminator], 'protocol account length differs')
    if discriminator:
        lut.require(data[:8] == hashlib.sha256(f'account:{discriminator}'.encode()).digest()[:8], 'wrong protocol discriminator')
    if 'Mint' in categories:
        target = tx['instructions'][5]['accounts']
        expected = target[12]['address'] if item['address'] == target[5]['address'] else target[11]['address']
        lut.require(len(data) >= 82 and data[45] == 1, 'uninitialized or malformed mint')
    if 'Payer' in categories or 'Authority' in categories:
        expected = inventory.SYSTEM
        lut.require(not data, 'unexpected authority/payer data')
    if item['address'] == inventory.SYSTEM:
        expected = NATIVE
        lut.require(account['executable'], 'native program is not executable')
    elif 'farms-program' in roles:
        lut.require(account['owner'] in LOADERS and account['executable'], 'invalid passed Farms program header')
    else:
        lut.require(not account['executable'], 'state must not be executable')
    if expected:
        lut.require(account['owner'] == expected, 'wrong historical owner')
    prior = known_hashes().get((item['address'], item['slot']))
    lut.require(prior is None or facts['data_sha256'] == prior, 'contradicts prior exact S-1/S data hash')
    facts.update(owner_check='role_and_historical_boundary', anchor_type=discriminator,
                 typed_protocol_and_extension_validation='pending', execution_seed_admitted=False)
    return facts


def evaluate(row, derived, call):
    _, binaries, _ = inventory.frozen_inputs()
    lut.require(derived['planned_boundary_requests'] == state.state_plan(row, binaries), 'complete ordered boundary inventory required')
    result = {'kind': 'u3e_T1_boundary_capture', 'signature': inventory.SIG,
              'planned': derived['planned_boundary_requests'], 'acquired': [], 'failure': None,
              'raw_boundary_capture_complete': False, 'historical_state_complete': False,
              'runtime_executed': False, 'production_replay_eligible': False}
    def genesis_check(value):
        lut.require(value == lut.GENESIS, 'archive genesis differs')
        return {'genesis': value}
    _, _, error = call('getGenesisHash', [], genesis_check, False)
    if error:
        result['failure'] = {'reason': error, 'request': {'method': 'getGenesisHash', 'params': []}}
        return result
    for item in result['planned']:
        _, facts, error = call(item['method'], item['params'], lambda value, i=item: validate(row, derived, i, value), permits_absence(row, item))
        if error:
            result['failure'] = {'reason': error, 'request': item}
            return result
        result['acquired'].append(dict(item, facts=facts))
    # Complete raw bytes alone never assert complete state, runtime closure or replay.
    result['raw_boundary_capture_complete'] = True
    return result


def run(root, verify=False):
    row, _, derived = inputs()
    frozen = ROOT / 'phase-u3e-validation/state-inventory.json'
    lut.require(frozen.read_bytes() == lut.canonical(derived), 'inventory changed after pre-capture freeze')
    qualified = lut.read(state.envelope.U3B / 'acquisition.json')['provider']
    if verify:
        receipt = lut.read(root / 'receipt.json')
        lut.require(receipt['provider'] == qualified['scheme_host'] and receipt['qualification'] == qualified, 'archive qualification differs')
        contexts = [('getGenesisHash', [])] + [(i['method'], i['params']) for i in derived['planned_boundary_requests']]
        validators = {transport.request_id(i['method'], i['params']): lambda value, item=i: validate(row, derived, item, value) for i in derived['planned_boundary_requests']}
        validators[transport.request_id('getGenesisHash', [])] = lambda value: lut.require(value == lut.GENESIS, 'archive genesis differs')
        absent = {transport.request_id(i['method'], i['params']) for i in derived['planned_boundary_requests'] if permits_absence(row, i)}
        groups = transport.verify(root, contexts, validators, absent)
        used = []
        def call(method, params, check, allow):
            key = transport.request_id(method, params)
            lut.require(key in groups, 'required retained request missing')
            used.append(key)
            last = groups[key][-1]
            if last['failure_class']:
                return None, None, last['failure_class']
            value = lut.read(root / last['body_file'])['result']
            return value, check(value), None
        result = evaluate(row, derived, call)
        lut.require(set(used) == set(groups), 'unexpected requests after stopping boundary')
        lut.require((root / 'result.json').read_bytes() == lut.canonical(result), 'state result differs')
    else:
        rpc = transport.CurlArchive(os.environ['SOLANA_ARCHIVE_RPC_URL'], os.environ.get('SOLANA_ARCHIVE_RPC_ORIGIN', ''))
        lut.require(rpc.provider == qualified['scheme_host'], 'previously qualified provider required')
        client = transport.EvidenceClient(root, rpc, lut.read(transport.POLICY_PATH), qualified)
        result = evaluate(row, derived, client.call)
        client.finish(result)
    return result


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--verify', action='store_true')
    args = parser.parse_args()
    print(lut.canonical(run(args.output.resolve(), args.verify)).decode())
