#!/usr/bin/env python3
"""Historical bank-input acquisition after complete ordinary T1 boundaries."""
import base64
import os
from pathlib import Path
import struct

import kamino_u3e_proof as proof

s = proof.s
lut = s.lut
SYSVAR_OWNER = 'Sysvar1111111111111111111111111111111111111'
REQUIREMENTS = [
    ('Clock', 'SysvarC1ock11111111111111111111111111111111'),
    ('Rent', 'SysvarRent111111111111111111111111111111111'),
    ('EpochSchedule', 'SysvarEpochSchedu1e111111111111111111111111'),
    ('SlotHashes', 'SysvarS1otHashes111111111111111111111111111'),
]


def plan():
    return [{'name': name, 'address': address, 'method': 'getAccountInfo',
             'params': [address, {'encoding': 'base64', 'commitment': 'finalized', 'slot': s.inventory.SLOT}]} for name, address in REQUIREMENTS]


def validate(item, response):
    lut.require(response['context']['slot'] == s.inventory.SLOT, 'execution bank context differs')
    account = response['value']
    lut.require(account is not None and account['owner'] == SYSVAR_OWNER and account['executable'] is False, 'missing or invalid historical sysvar owner')
    lut.require(account['data'][1] == 'base64', 'sysvar encoding')
    data = base64.b64decode(account['data'][0], validate=True)
    lut.require('space' not in account or account['space'] == len(data), 'incomplete sysvar bytes')
    facts = {'data_bytes': len(data), 'data_sha256': lut.sha(data), 'owner': account['owner'],
             'lamports': account['lamports'], 'executable': account['executable'], 'rent_epoch': account['rentEpoch'],
             'raw_account_sha256': lut.sha(lut.canonical(account))}
    name = item['name']
    if name == 'Clock':
        lut.require(len(data) == 40, 'Clock layout differs')
        facts['fields'] = dict(zip(['slot', 'epoch_start_timestamp', 'epoch', 'leader_schedule_epoch', 'unix_timestamp'], struct.unpack('<QqQQq', data)))
        lut.require(facts['fields']['slot'] == s.inventory.SLOT, 'Clock is not the execution slot')
        row, _, _ = s.inventory.frozen_inputs()
        lut.require(facts['fields']['unix_timestamp'] == row['transaction']['block_time'], 'Clock and original validator block time disagree')
    elif name == 'Rent':
        lut.require(len(data) == 17, 'Rent layout differs')
        facts['fields'] = dict(zip(['lamports_per_byte_year', 'exemption_threshold', 'burn_percent'], struct.unpack('<QdB', data)))
    elif name == 'EpochSchedule':
        lut.require(len(data) == 33, 'EpochSchedule layout differs')
        facts['fields'] = dict(zip(['slots_per_epoch', 'leader_schedule_slot_offset', 'warmup', 'first_normal_epoch', 'first_normal_slot'], struct.unpack('<QQBQQ', data)))
    elif name == 'SlotHashes':
        count = int.from_bytes(data[:8], 'little')
        lut.require(0 < count <= 512 and len(data) == 8 + 40 * count, 'SlotHashes layout differs')
        slots = [int.from_bytes(data[8 + 40 * i:16 + 40 * i], 'little') for i in range(count)]
        lut.require(all(a > b for a, b in zip(slots, slots[1:])) and max(slots) < s.inventory.SLOT, 'SlotHashes context differs')
        facts['fields'] = {'count': count, 'slots': slots}
    return facts


def run(root, verify=False):
    state = proof.screen(s.ROOT / 'phase-u3e-boundary-proof', verify=True)
    lut.require(state['failure'] is None and state['raw_boundary_capture_complete'], 'state boundary proof prerequisite')
    qualified = lut.read(s.state.envelope.U3B / 'acquisition.json')['provider']
    planned = plan()
    if verify:
        receipt = lut.read(root / 'receipt.json')
        lut.require(receipt['provider'] == qualified['scheme_host'] and receipt['qualification'] == qualified, 'archive qualification differs')
        groups = s.transport.verify(root, [(i['method'], i['params']) for i in planned],
                                    {s.transport.request_id(i['method'], i['params']): lambda v, item=i: validate(item, v) for i in planned})
        used = set()
        def call(item):
            key = s.transport.request_id(item['method'], item['params'])
            lut.require(key in groups, 'required runtime context request missing')
            used.add(key)
            last = groups[key][-1]
            return (None, last['failure_class']) if last['failure_class'] else (validate(item, lut.read(root / last['body_file'])['result']), None)
    else:
        rpc = s.transport.CurlArchive(os.environ['SOLANA_ARCHIVE_RPC_URL'], os.environ.get('SOLANA_ARCHIVE_RPC_ORIGIN', ''))
        lut.require(rpc.provider == qualified['scheme_host'], 'previously qualified provider required')
        client = s.transport.EvidenceClient(root, rpc, lut.read(s.transport.POLICY_PATH), qualified)
        def call(item):
            _, facts, failure = client.call(item['method'], item['params'], lambda value: validate(item, value))
            return facts, failure
    result = {'kind': 'u3e_historical_runtime_input_capture', 'planned': planned, 'acquired': [], 'failure': None,
              'historical_runtime_inputs_captured': False, 'runtime_context_proven': False, 'runtime_executed': False}
    for item in planned:
        facts, failure = call(item)
        if failure:
            result['failure'] = {'request': item, 'reason': failure}
            break
        result['acquired'].append(dict(item, facts=facts))
    result['historical_runtime_inputs_captured'] = result['failure'] is None
    if verify:
        lut.require(used == set(groups), 'requests after runtime stopping boundary')
        lut.require((root / 'result.json').read_bytes() == lut.canonical(result), 'runtime derivation differs')
    else:
        client.finish(result)
    return result


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--verify', action='store_true')
    args = parser.parse_args()
    print(lut.canonical(run(args.output.resolve(), args.verify)).decode())
