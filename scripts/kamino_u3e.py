#!/usr/bin/env python3
"""T1-only diagnosed archive probe. Later stages require this gate to pass."""
import argparse
import os
from pathlib import Path

import historical_transport as transport
import kamino_u3d_inventory as inventory
import kamino_u3d_state as state

lut = transport.lut
MAPPING = '4zh6bmb77qX2CL7t5AJYCqa6YqFafbz3QJNeFvZjLowg'


def probe_inputs():
    row, binaries, proof = inventory.frozen_inputs()
    plan = state.state_plan(row, binaries)
    item = next(i for i in plan if i['address'] == MAPPING and i['boundary'] == 'pre')
    lut.require(item['slot'] == 448195165, 'exact failed mappings context required')
    return row, item


def probe(root, verify=False):
    row, item = probe_inputs()
    method, params = item['method'], item['params']
    validator = lambda value: state.account_check(row, item, value)
    if verify:
        qualified = lut.read(state.envelope.U3B / 'acquisition.json')['provider']
        receipt = lut.read(root / 'receipt.json')
        lut.require(receipt['provider'] == qualified['scheme_host'] and receipt['qualification'] == qualified,
                    'archive qualification differs')
        groups = transport.verify(root, [(method, params)], {transport.request_id(method, params): validator})
        attempts = groups[transport.request_id(method, params)]
        last = attempts[-1]
        failure = last['failure_class']
        facts = None
        if failure is None:
            facts = validator(lut.read(root / last['body_file'])['result'])
    else:
        policy = lut.read(transport.POLICY_PATH)
        rpc = transport.CurlArchive(os.environ['SOLANA_ARCHIVE_RPC_URL'], os.environ.get('SOLANA_ARCHIVE_RPC_ORIGIN', ''))
        qualified = lut.read(state.envelope.U3B / 'acquisition.json')['provider']
        lut.require(rpc.provider == qualified['scheme_host'], 'previously qualified provider required')
        client = transport.EvidenceClient(root, rpc, policy, qualified)
        _, facts, failure = client.call(method, params, validator)
        attempts = client.receipt['attempts']
    result = {'kind': 'exact_T1_mapping_reproduction', 'signature': inventory.SIG,
              'sample_fingerprint': lut.FINGERPRINT, 'request': item,
              'probe_passed': failure is None, 'failure_class': failure, 'facts': facts,
              'behavior': ('prior_empty_response_not_reproduced' if len(attempts) == 1 else 'transient_failure_then_success')
                          if failure is None else 'unresolved_in_bounded_attempt',
              'attempts': len(attempts), 'historical_state_complete': False, 'runtime_executed': False}
    if verify:
        lut.require((root / 'result.json').read_bytes() == lut.canonical(result), 'probe derivation differs')
    else:
        client.finish(result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--verify', action='store_true')
    args = parser.parse_args()
    print(lut.canonical(probe(args.output.resolve(), args.verify)).decode())


if __name__ == '__main__':
    main()
