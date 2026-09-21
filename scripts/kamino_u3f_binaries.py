#!/usr/bin/env python3
"""Continue exact historical resolver contexts; retained successes only, no slot substitution."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tarfile
import kamino_u3f_fidelity as fidelity

f = fidelity.f
lut = f.lut
t = f.s.transport


def retained():
    cache = {}
    def add(method, params, body, digest, source):
        lut.require(lut.sha(body) == digest, 'retained response hash differs')
        value = json.loads(body)['result']
        key = t.request_id(method, params)
        if key in cache:
            a, b = cache[key]['result'], value
            # Wrapper API-version fields are not account evidence.
            lut.require((a.get('value'), a.get('context', {}).get('slot')) == (b.get('value'), b.get('context', {}).get('slot')) if isinstance(a, dict) and method == 'getAccountInfo' else a == b, 'contradictory historical response')
        else:
            cache[key] = {'result': value, 'sources': []}
        cache[key]['sources'].append(source | {'sha256': digest})
    for name in ('phase-u3c-dependencies', 'phase-u3d2-binaries'):
        root = f.ROOT / name
        index = lut.read(root / 'capture-index.json')
        path = root / 'capture.tar.gz'
        lut.require(lut.sha(path.read_bytes()) == index['compressed_sha256'], 'archive hash differs')
        with tarfile.open(path) as archive:
            receipt = json.load(archive.extractfile('receipt.json'))
            for r in receipt['requests']:
                if r['status'] != 'success':
                    continue
                add(r['method'], r['params'], archive.extractfile(r['response_file']).read(), r['response_sha256'], {'archive': str(path.relative_to(lut.REPO)), 'member': r['response_file']})
    root = f.ROOT / 'phase-u3f-binary-recovery'
    for r in lut.read(root / 'receipt.json')['attempts']:
        if r['failure_class'] is None:
            add(r['method'], r['params'], (root / r['body_file']).read_bytes(), r['body_sha256'], {'file': str((root / r['body_file']).relative_to(lut.REPO))})
    return cache


def run(root):
    lut.require(fidelity.derive()[0]['decision'] == 'T1-A', 'T1 fidelity gate')
    cache = retained()
    qualified = lut.read(f.envelope.U3B / 'acquisition.json')['provider']
    rpc = t.CurlArchive(os.environ['SOLANA_ARCHIVE_RPC_URL'], os.environ.get('SOLANA_ARCHIVE_RPC_ORIGIN', ''))
    lut.require(rpc.provider == qualified['scheme_host'], 'qualified archive required')
    client = t.EvidenceClient(root, rpc, lut.read(t.POLICY_PATH), qualified)
    rows = [lut.read(p) for p in (f.ROOT / 'phase-u3c-envelope/transactions').glob('*.json')]
    rows = sorted([r for r in rows if r['envelope']['envelope_admissible']], key=lambda r: -r['transaction']['slot'])
    lut.require(len(rows) == 4 and rows[0]['transaction']['signature'] == f.s.inventory.SIG, 'frozen cohort differs')
    usage, results = [], []
    for number, row in enumerate(rows[1:], 2):
        name = f'T{number}'
        folder = root / name
        folder.mkdir()
        with subprocess.Popen([str(lut.REPO / 'target/debug/examples/acquire_envelope_dependencies')], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as process:
            process.stdin.write(json.dumps({'transaction': row['transaction'], 'output': str(folder.resolve())}) + '\n')
            process.stdin.flush()
            result = None
            for line in process.stdout:
                message = json.loads(line)
                if message['kind'] == 'capture_result':
                    result = message['result']
                    continue
                lut.require(message['kind'] == 'rpc_request', 'unexpected resolver output')
                method, params = message['method'], message['params']
                key = t.request_id(method, params)
                # Always screen the full block freshly for this target.
                if key in cache and method != 'getBlock':
                    reply = {'result': cache[key]['result']}
                    sources = cache[key]['sources']
                else:
                    value, _, failure = client.call(method, params)
                    reply = {'error': failure} if failure else {'result': value}
                    sources = [{'receipt': str((root / 'receipt.json').relative_to(lut.REPO)), 'request_id': key}]
                    cache[key] = reply | {'sources': sources}
                    print(json.dumps({'target': name, 'method': method, 'failure': failure, 'requests': len(client.receipt['attempts'])}), flush=True)
                usage.append({'target': name, 'method': method, 'params': params, 'sources': sources})
                (root / 'resolver-requests.json').write_bytes(lut.canonical(usage))
                process.stdin.write(json.dumps(reply) + '\n')
                process.stdin.flush()
            stderr = process.stderr.read()
            code = process.wait()
            (folder / 'resolver.stderr').write_text(stderr)
            lut.require(code == 0 and result is not None, 'resolver failed; raw evidence retained')
        (folder / 'result.json').write_bytes(lut.canonical(result))
        results.append({'target': name, 'result': result})
        print(json.dumps({'target': name, 'binaries_complete': result['C4_binaries_identified'], 'failure': result['failure']}), flush=True)
    client.finish({'targets': results, 'historical_state_acquisition': 'not_attempted'})
    files = {str(p.relative_to(root)): lut.sha(p.read_bytes()) for p in root.rglob('*') if p.is_file()}
    (root / 'checksums.json').write_bytes(lut.canonical(files))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    run(parser.parse_args().output.resolve())
