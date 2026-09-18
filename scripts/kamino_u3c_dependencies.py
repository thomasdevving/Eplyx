#!/usr/bin/env python3
"""Offline revalidation of exact historical dependency responses; no transport.

The compressed archive preserves the original attempt, including its original
checkpoint labels. Derivation distinguishes a clean screen from state acquisition.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import time
import kamino_u3b_lut as lut
import kamino_u3c_envelope as envelope

EVIDENCE = lut.REPO / 'docs/examples/phase-u3c-dependencies'


def validate_capture(root):
    receipt = lut.read(root / 'receipt.json')
    qualified = lut.read(envelope.U3B / 'acquisition.json')['provider']
    lut.require(receipt['complete'] is True, 'incomplete dependency receipt')
    lut.require(receipt['sample_fingerprint'] == lut.FINGERPRINT and receipt['genesis'] == lut.GENESIS, 'frozen identity differs')
    lut.require(receipt['provider'] == qualified['scheme_host'], 'qualified provider differs')
    lut.require(receipt['max_attempts_per_request_context'] == 1 and receipt['fallback'] is False, 'bounded acquisition policy differs')
    if 'archive_validation_sha256' in receipt:
        lut.require(receipt['archive_validation_sha256'] == qualified['validation_artifact_sha256'], 'archive qualification differs')
    actual = {str(p.relative_to(root)) for p in root.rglob('*') if p.is_file()} - {'receipt.json', 'timing.json'}
    lut.require(actual == set(receipt['raw_artifact_hashes']), 'raw artifact membership differs')
    for ref, digest in receipt['raw_artifact_hashes'].items():
        lut.require(lut.sha(lut.safe_file(root, ref).read_bytes()) == digest, 'retained raw artifact hash differs')
    summaries = [json.loads(b) for n, b in envelope.derive().items() if n.startswith('transactions/')]
    primary = {r['envelope']['signature']: r for r in summaries if envelope.SCOPE in r['before_rejection']}
    lut.require({r['signature'] for r in receipt['target_results']} == set(primary) and len(receipt['target_results']) == 4, 'primary target membership differs')
    pre_slots = {r['envelope']['execution_slot'] - 1 for r in primary.values()}
    slots = {s + 1 for s in pre_slots}
    cache = {}
    for request in receipt['requests']:
        method, params = request['method'], request['params']
        key = lut.sha(lut.canonical([method, params]))
        lut.require(key == request['request_id'] and key not in cache, 'duplicate or changed request context')
        lut.require(request['attempted'] is True, 'unresolved request membership')
        if method == 'getAccountInfo':
            lut.require(params[1].get('slot') in pre_slots and params[1].get('commitment') == 'finalized' and params[1].get('encoding') == 'base64', 'current or intermediate account query rejected')
        elif method == 'getBlock':
            lut.require(params[0] in slots and params[1]['commitment'] == 'finalized', 'screen execution slot differs')
        else:
            lut.require(method == 'getGenesisHash' and params == [], 'unexpected acquisition method')
        ref = request['response_file']
        body = lut.safe_file(root, ref).read_bytes() if ref else b''
        if body:
            try:
                safe_value = json.loads(body)
            except (ValueError, UnicodeError):
                safe_value = body.decode('utf8', errors='replace')
            lut.baseline.hygiene(safe_value)
        if ref:
            lut.require(lut.sha(body) == request['response_sha256'], 'transport response hash differs')
        else:
            lut.require(request['status'] == 'failure', 'successful response cannot be absent')
        if request['status'] == 'success':
            value = json.loads(body)
            lut.require(value.get('error') is None and 'result' in value and not request['response_withheld'], 'invalid successful response')
            if method == 'getGenesisHash':
                lut.require(value['result'] == lut.GENESIS, 'archive genesis differs')
            if method == 'getAccountInfo':
                lut.require(value['result']['context']['slot'] == params[1]['slot'], 'archive returned current/wrong context')
            cache[key] = {'result': value['result']}
        else:
            lut.require(request['status'] == 'failure' and request['failure_reason'], 'explicit failure reason required')
            cache[key] = {'error': request['failure_reason']}
    return receipt, primary, cache


def unpack(evidence, root):
    index = lut.read(evidence / 'capture-index.json')
    archive = evidence / 'capture.tar.gz'
    lut.require(lut.sha(archive.read_bytes()) == index['compressed_sha256'], 'capture archive hash differs')
    seen = set()
    size = 0
    with tarfile.open(archive, 'r:gz') as tar:
        for member in tar:
            lut.require(member.isfile() and member.name not in seen, 'unsafe or duplicate archive member')
            path = lut.safe_file(root, member.name)
            seen.add(member.name)
            size += member.size
            lut.require(size <= index['expanded_bytes'], 'capture archive exceeds pinned size')
            path.parent.mkdir(parents=True, exist_ok=True)
            with tar.extractfile(member) as source, path.open('wb') as dest:
                while chunk := source.read(1024 * 1024):
                    dest.write(chunk)
    lut.require(size == index['expanded_bytes'] and len(seen) == index['expanded_files'], 'capture archive population differs')
    return index


def replay_resolver(row, cache, output, used):
    env = {k: v for k, v in os.environ.items() if not any(s in k for s in ('RPC', 'API_KEY', 'ARCHIVE', 'ALCHEMY', 'HELIUS'))}
    process = subprocess.Popen([str(lut.REPO / 'target/debug/examples/acquire_envelope_dependencies')], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
    process.stdin.write(json.dumps({'transaction': row['transaction'], 'output': str(output)}) + '\n')
    process.stdin.flush()
    result = None
    try:
        for line in process.stdout:
            message = json.loads(line)
            if message['kind'] == 'capture_result':
                lut.require(result is None, 'duplicate resolver result')
                result = message['result']
                continue
            lut.require(message['kind'] == 'rpc_request', 'unexpected offline resolver output')
            key = lut.sha(lut.canonical([message['method'], message['params']]))
            lut.require(key in cache, 'offline resolver requested uncaptured context; no fallback')
            used.add(key)
            process.stdin.write(json.dumps(cache[key]) + '\n')
            process.stdin.flush()
        lut.require(process.wait() == 0 and result is not None, 'offline dependency resolver failed')
        return result
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        for stream in (process.stdin, process.stdout, process.stderr):
            stream.close()


def derive(evidence=EVIDENCE, reverse=False):
    started = time.perf_counter()
    envelope.frozen_inputs()
    with tempfile.TemporaryDirectory(prefix='eplyx-u3c-offline-') as tmp:
        root = Path(tmp)
        index = unpack(evidence, root / 'capture')
        receipt, primary, cache = validate_capture(root / 'capture')
        qualified = lut.read(envelope.U3B / 'acquisition.json')['provider']
        lut.require(index['archive_validation_sha256'] == qualified['validation_artifact_sha256'], 'capture qualification binding differs')
        targets = receipt['target_results'][::-1] if reverse else receipt['target_results']
        outputs, stages, used = {}, [], set()
        for target in targets:
            sig = target['signature']
            old = lut.read(root / 'capture' / target['result_file'])
            result = replay_resolver(primary[sig], cache, root / 'derived' / sig, used)
            for field in ('programs', 'C4_binaries_identified', 'slot_screening', 'signature', 'execution_slot', 'pre_slot'):
                lut.require(result[field] == old[field], 'historical binary/screen derivation differs')
            for program in result['programs']:
                if program['source'] == 'historical_mainnet':
                    file = program['binary_file']
                    bytes_ = (root / 'derived' / sig / file).read_bytes()
                    lut.require(bytes_ == (root / 'capture' / sig / file).read_bytes() and lut.sha(bytes_) == program['provenance']['sha256'], 'reconstructed historical ELF differs')
            if result['C4_binaries_identified']:
                # The original collector used a pending-state checkpoint label.
                # No state RPC was made; it is not a measured C5 failure.
                lut.require(result['slot_screening'] is not None and not result['slot_screening']['conflicts'] and result['failure'] is None, 'complete binary target screen must be clean')
            else:
                lut.require(result['failure'] == old['failure'], 'observed binary failure differs')
            result['frozen_message_proof_id'] = primary[sig]['frozen_lut_proof_id']
            result['archive_validation_sha256'] = qualified['validation_artifact_sha256']
            outputs[f'resolved/{sig}.json'] = lut.canonical(result)
            stages.append({'signature': sig, 'execution_slot': result['execution_slot'], 'C1': 'passed', 'C2': 'passed', 'C3': 'passed', 'C4': 'passed' if result['C4_binaries_identified'] else 'failed', **{f'C{i}': 'not_attempted' for i in range(5, 11)}, 'next_observed_blocker': result['failure'], 'cohort_stop': 'bounded historical binary acquisition incomplete for three other primary targets' if result['C4_binaries_identified'] else None})
        lut.require(used == set(cache), 'unused request membership; complete capture not reproduced')
        stages.sort(key=lambda s: (-s['execution_slot'], s['signature']))
        outputs['stage-table.json'] = lut.canonical({'sample_fingerprint': lut.FINGERPRINT, 'rows': stages, 'primary_metrics': {'N_classified': 4, 'M_admitted': 4, 'C4_binary_capture_complete': sum(s['C4'] == 'passed' for s in stages), 'K_all_historical_dependencies_acquired': 0, 'J_baseline_attempted': 0, 'P_baseline_fidelity': 0}, 'runtime_executed': False, 'state_requests': 0, 'decision': 'B', 'scope': 'exact primary structural envelopes; production replay still blocked'})
        outputs['checksums.sha256'] = ''.join(f'{lut.sha(b)}  {n}\n' for n, b in sorted(outputs.items())).encode()
    return outputs, time.perf_counter() - started


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--evidence', type=Path, default=EVIDENCE)
    parser.add_argument('--output', type=Path, default=EVIDENCE)
    parser.add_argument('--verify', action='store_true')
    parser.add_argument('--reverse', action='store_true')
    args = parser.parse_args()
    lut.require(not args.output.resolve().is_relative_to(lut.SAMPLE.resolve()) and not args.output.resolve().is_relative_to(envelope.U3B.resolve()), 'cannot write frozen sample')
    outputs, elapsed = derive(args.evidence, args.reverse)
    for name, body in outputs.items():
        path = lut.safe_file(args.output, name)
        if args.verify:
            lut.require(path.read_bytes() == body, 'canonical dependency output differs')
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(body)
    print(json.dumps({'verified': args.verify, 'offline_seconds': elapsed, 'stage_table': json.loads(outputs['stage-table.json'])}, indent=2))


if __name__ == '__main__':
    main()
