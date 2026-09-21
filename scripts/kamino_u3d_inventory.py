#!/usr/bin/env python3
"""Offline T1 input inventory, not state acquisition or execution admission.

Rebuild U3C first with rebuild-kamino-u3c-envelopes.sh --verify. This reader
pins those revalidated artifacts to the Stage 0 freeze and never makes RPCs.
Unknown historical owners remain null; interface expectations are separate.
"""
import argparse
import json
from pathlib import Path
import re
import tarfile

import kamino_u3b_lut as lut

REPO = lut.REPO
VALIDATION = REPO / 'docs/examples/phase-u3d-validation'
OUTPUT = REPO / 'docs/examples/phase-u3d-inventory'
SIG = '5eLacZQNT4qYoCd6w9KyULUmycVFeEMjcuoS9sEZXsUifVKXi6AkGUx93zgmzEGZ2KUDzfB8Z7m8ZoAo3fvSWg97'
SLOT = 448195166
SCOPE = 'HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ'
KLEND = 'KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD'
SYSTEM = '11111111111111111111111111111111'
INSTRUCTIONS = 'Sysvar1nstructions1111111111111111111111111'


def frozen_inputs():
    freeze = lut.read(VALIDATION / 'preimplementation.json')
    control = lut.read(VALIDATION / 'stage-0.json')
    lut.require(control['status'] == 'passed', 'Stage 0 must pass')
    hashes = dict(freeze['frozen_artifact_hashes'], **control['source_hashes'])
    for name, digest in hashes.items():
        lut.require(lut.sha((REPO / name).read_bytes()) == digest, f'frozen input changed: {name}')
    row = lut.read(REPO / f'docs/examples/phase-u3c-envelope/transactions/{SIG}.json')
    dependencies = lut.read(REPO / f'docs/examples/phase-u3c-dependencies/resolved/{SIG}.json')
    proof = lut.read(REPO / f'docs/examples/phase-u3b2-lut/proofs/{SIG}.json')['proof']
    return row, dependencies, proof


def captured_headers():
    """Use retained raw account headers, including the first ProgramData chunk."""
    root = REPO / 'docs/examples/phase-u3c-dependencies'
    index = lut.read(root / 'capture-index.json')
    lut.require(lut.sha((root / 'capture.tar.gz').read_bytes()) == index['compressed_sha256'], 'capture hash mismatch')
    headers = {}
    with tarfile.open(root / 'capture.tar.gz') as archive:
        receipt = json.load(archive.extractfile('receipt.json'))
        for request in receipt['requests']:
            if request['method'] != 'getAccountInfo' or request['status'] != 'success':
                continue
            address, config = request['params']
            if config['slot'] != SLOT - 1 or config.get('dataSlice', {}).get('offset', 0) != 0:
                continue
            body = archive.extractfile(request['response_file']).read()
            lut.require(lut.sha(body) == request['response_sha256'], 'raw response hash mismatch')
            response = json.loads(body)['result']
            lut.require(response['context']['slot'] == SLOT - 1, 'historical header context mismatch')
            account = response['value']
            headers[address] = {
                'owner': account['owner'], 'lamports': account['lamports'],
                'executable': account['executable'], 'context_slot': SLOT - 1,
                'response_file_in_u3c_archive': request['response_file'],
                'response_sha256': request['response_sha256'],
                'data_is_slice': 'dataSlice' in config,
            }
    return headers


def role_names():
    # Reuse the frozen U2 instruction interface; never derive roles from writes.
    source = (REPO / 'engine/src/protocol/kamino/mod.rs').read_text()
    def names(constant):
        match = re.search(r'const ' + constant + r':.*?= \[(.*?)\];', source, re.S)
        lut.require(match is not None, 'missing frozen account roles')
        return re.findall(r'"([^"\n]+)"', match.group(1))
    return names('DEPOSIT_ROLES') + names('FARMS_ROLES')


def derive(row, dependencies, proof, headers):
    tx = row['transaction']
    lut.require(tx['signature'] == SIG and tx['slot'] == SLOT and tx['success'], 'exact successful T1 required')
    lut.require(tx['account_keys'] == proof['full_account_keys'], 'native message key order changed')
    lut.require(len(tx['instructions']) == 8 and row['envelope']['envelope_admissible'], 'full admitted envelope required')
    lut.require(tx['instructions'] == [r['instruction'] for r in row['envelope']['instructions']], 'instruction order or bytes changed')
    screen = dependencies['slot_screening']
    required = {a['address'] for a in tx['account_keys'] if a['address'] != INSTRUCTIONS}
    programdata = {p['provenance']['programdata_address']: p['program_id'] for p in dependencies['programs']
                   if p.get('provenance', {}).get('programdata_address')}
    required.update(programdata)
    lut.require(screen['slot'] == SLOT and screen['target_signature'] == SIG and not screen['conflicts']
                and required <= set(screen['required_accounts']), 'complete unchanged same-slot screen required')
    roles = {a['address']: [] for a in tx['account_keys']}
    def add(ix, labels):
        accounts = tx['instructions'][ix]['accounts']
        lut.require(len(accounts) == len(labels), 'instruction role arity differs')
        for position, (account, label) in enumerate(zip(accounts, labels)):
            roles[account['address']].append({'outer_index': ix, 'account_position': position, 'role': label})
    add(0, ['oracle-prices', 'oracle-mappings', 'oracle-twaps', 'instructions-sysvar'] + ['scope-remaining-account'] * 4)
    add(1, ['ata-payer', 'associated-token-account', 'ata-wallet', 'ata-mint', 'system-program', 'ata-token-program'])
    for ix in (2, 3):
        add(ix, ['reserve', 'lending-market', 'optional-pyth-oracle', 'optional-switchboard-price', 'optional-switchboard-twap', 'scope-price-oracle'])
    add(4, ['lending-market', 'obligation', 'obligation-reserve', 'obligation-reserve'])
    add(5, role_names())
    invoked = {p['program_id']: p for p in dependencies['programs']}
    static_count = len(tx['account_keys']) - tx['loaded_address_count']
    tokens = {t['account_index']: t for t in tx['pre_token_balances']}
    mint_addresses = {t['mint'] for t in tx['pre_token_balances']}
    rows = []
    for index, key in enumerate(tx['account_keys']):
        address = key['address']
        labels = {r['role'] for r in roles[address]}
        classes = ['static' if index < static_count else 'lookup_loaded', 'writable' if key['is_writable'] else 'readonly']
        if key['is_signer']:
            classes.append('signer')
        if address in invoked or 'farms-program' in labels or address == SYSTEM:
            classes.append('program')
        if address == INSTRUCTIONS:
            classes.extend(['sysvar', 'runtime_provided'])
        if index in tokens:
            classes.append('token_account')
        if address in mint_addresses:
            classes.append('mint')
        if labels & {'oracle-prices', 'oracle-mappings', 'oracle-twaps'}:
            classes.append('oracle_state')
        if labels & {'reserve', 'obligation', 'lending-market'}:
            classes.append('protocol_state')
        if labels == {'lending-market-authority'}:
            classes.append('authority_pda_presence_unknown')
        header = headers.get(address)
        expected = SCOPE if 'oracle_state' in classes else KLEND if 'protocol_state' in classes else None
        if index in tokens:
            expected = tokens[index]['program_id']
        rows.append({
            'address': address, 'message_index': index, 'classes': classes, 'roles': roles[address],
            'is_signer': key['is_signer'], 'is_writable': key['is_writable'],
            'owner': header['owner'] if header else None, 'expected_owner': expected,
            'owner_evidence': 'historical_raw_response' if header else 'not_acquired',
            'historical_capture': header, 'historically_acquired': header is not None,
            'runtime_provided': address == INSTRUCTIONS,
            'required_boundary': 'execution_bank_instruction_context' if address == INSTRUCTIONS else 'end_of_S_minus_1_under_unchanged_screen',
            'requested_pre_slot': None if address == INSTRUCTIONS else SLOT - 1,
            'post_reference_slot': SLOT if key['is_writable'] else None,
            'observed_invocation': address in invoked,
            'binary_provenance': invoked.get(address),
            'token_metadata': tokens.get(index),
            'historical_bytes_still_required': header is None and address != INSTRUCTIONS,
        })
    for address, program in sorted(programdata.items()):
        lut.require(address in headers, 'ProgramData historical header missing')
        rows.append({'address': address, 'message_index': None, 'classes': ['programdata'],
                     'program': program, 'owner': headers[address]['owner'], 'historical_capture': headers[address],
                     'historically_acquired': True, 'runtime_provided': False,
                     'required_boundary': 'end_of_S_minus_1_under_unchanged_screen',
                     'requested_pre_slot': SLOT - 1, 'historical_bytes_still_required': False})
    lut.require(len(roles) == 23 and len(rows) == 27, 'frozen T1 account population differs')
    return {
        'kind': 'experimental_t1_input_inventory_not_execution_admission',
        'signature': SIG, 'execution_slot': SLOT, 'sample_fingerprint': lut.FINGERPRINT,
        'lut_proof_id': proof['proof_id'], 'account_rows': rows,
        'same_slot_screen': screen,
        'scope_ordered_remaining_accounts': [a['address'] for a in tx['instructions'][0]['accounts'][4:]],
        'scope_tokens': [344, 279, 13, 456],
        'lut_input_reference': f'docs/examples/phase-u3b2-lut/proofs/{SIG}.json',
        'message_resolution_inputs': [{
            'address': table['table_pubkey'], 'class': 'address_lookup_table',
            'owner': 'AddressLookupTab1e1111111111111111111111111',
            'owner_evidence': 'validated_by_frozen_U3B_proof',
            'required_boundary': 'execution_slot_active_address_resolution',
            'requested_slot': table['requested_slot'],
            'returned_context_slot': table['returned_context_slot'],
            'raw_account_sha256': table['raw_account_sha256'],
            'raw_response_sha256': table['raw_response_sha256'],
            'historically_acquired': True, 'runtime_provided': False,
            'not_a_protocol_seed_account': True,
        } for table in proof['tables']],
        'lut_boundary': 'existing execution-slot proof; message resolution input, not protocol pre-state',
        'inventory_complete_for_message_and_observed_binaries': True,
        'runtime_dependency_closure_proven': False,
        'protocol_state_accounts_acquired': 0, 'all_historical_inputs_complete': False,
        'runtime_executed': False, 'production_replay_eligible': False,
        'missing_historical_message_accounts': [r['address'] for r in rows if r['historical_bytes_still_required']],
        'boundary_limits': [
            'Clean screen proves no other transaction declares these 26 accounts writable in S; it does not acquire state.',
            'S-1 is sufficient for ordinary account writes only under the qualified archive contract and unchanged screen; validate bytes and validator balances before admission.',
            'Runtime bank updates and sysvars require execution-bank treatment, not blind S-1 seeding.',
            'System and Farms are passed accounts without observed invocation; prove their account headers before deciding runtime treatment.',
            'Optional KLend accounts alias KLend itself; do not invent separate oracle/farm state.',
            'ATA metadata and PDA admission do not prove historical token owner, mint, state or extensions.',
            'Scope mapping/TWAP/configuration closure and every hidden sysvar requirement remain unproven without historical state and runtime analysis.',
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=OUTPUT)
    parser.add_argument('--verify', action='store_true')
    args = parser.parse_args()
    # Never overwrite the evidence this tool consumes, even via --output.
    for name in ('phase-u3-baseline', 'phase-u3b2-lut', 'phase-u3c-envelope', 'phase-u3c-dependencies', 'phase-u3c-interface', 'phase-u3d-validation'):
        lut.require(not args.output.resolve().is_relative_to((REPO / 'docs/examples' / name).resolve()), 'cannot write frozen inputs or controls')
    row, dependencies, proof = frozen_inputs()
    body = lut.canonical(derive(row, dependencies, proof, captured_headers()))
    outputs = {'inventory.json': body, 'checksums.sha256': f'{lut.sha(body)}  inventory.json\n'.encode()}
    for name, content in outputs.items():
        path = args.output / name
        if args.verify:
            lut.require(path.read_bytes() == content, 'inventory output differs')
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
    print(json.dumps({'verified': args.verify, 'message_accounts': 23, 'programdata_accounts': 4,
                      'missing_historical_message_accounts': len(json.loads(body)['missing_historical_message_accounts']),
                      'inventory_sha256': lut.sha(body), 'runtime_executed': False}))


if __name__ == '__main__':
    main()
