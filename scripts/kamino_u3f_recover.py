#!/usr/bin/env python3
"""Bounded exact T2-T4 binary-gap recovery after independently verified T1-A."""
import base64
import json
import os
from pathlib import Path

import kamino_u3f_fidelity as fidelity

f = fidelity.f
lut = f.lut
t = f.s.transport


def check(params, result):
    lut.require(result['context']['slot'] == params[1]['slot'] and result['value'] is not None, 'exact historical binary account required')
    account = result['value']
    lut.require(account['owner'] == 'BPFLoaderUpgradeab1e11111111111111111111111', 'historical loader differs')
    data = base64.b64decode(account['data'][0], validate=True)
    sliced = 'dataSlice' in params[1]
    lut.require(account['executable'] is (not sliced), 'historical executable flag differs')
    if sliced:
        offset, length = [params[1]['dataSlice'][k] for k in ('offset', 'length')]
        lut.require(len(data) == min(length, account['space'] - offset) > 0, 'partial binary chunk')
    else:
        lut.require(len(data) == account['space'] == 36 and data[:4] == b'\x02\0\0\0', 'invalid upgradeable program header')
    return {'owner': account['owner'], 'executable': account['executable'], 'lamports': account['lamports'], 'data_bytes': len(data), 'data_sha256': lut.sha(data)}


def run(root):
    result, semantics = fidelity.derive()
    lut.require(result['decision'] == 'T1-A', 'T1-A required before cohort recovery')
    previous = lut.read(f.ROOT / 'phase-u3d2-validation/final-report.json')['T2_T4_binary_result']
    qualified = lut.read(f.envelope.U3B / 'acquisition.json')['provider']
    rpc = t.CurlArchive(os.environ['SOLANA_ARCHIVE_RPC_URL'], os.environ.get('SOLANA_ARCHIVE_RPC_ORIGIN', ''))
    lut.require(rpc.provider == qualified['scheme_host'], 'previously qualified archive required')
    client = t.EvidenceClient(root, rpc, lut.read(t.POLICY_PATH), qualified)
    targets = []
    for name in ('T2', 'T3', 'T4'):
        request = previous[name]
        method, params = request['method'], request['params']
        _, facts, failure = client.call(method, params, lambda value, p=params: check(p, value))
        target = {'target': name, 'method': method, 'params': params, 'failure': failure, 'facts': facts, 'binary_set_complete': False, 'runtime_executed': False}
        targets.append(target)
        print(json.dumps({'target': name, 'failure': failure, 'bytes': facts['data_bytes'] if facts else None}), flush=True)
    result = {'kind': 'exact_T2_T4_missing_context_recovery', 'T1_gate': 'T1-A verified from raw evidence', 'targets': targets}
    client.finish(result)
    return result


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    run(args.output.resolve())
