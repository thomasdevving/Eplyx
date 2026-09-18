#!/usr/bin/env python3
"""Deterministic offline U3B membership and LUT-proof derivation."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import kamino_u3_baseline as baseline

REPO = baseline.REPO
SAMPLE = REPO / 'docs/examples/phase-u3-baseline'
EVIDENCE = REPO / 'docs/examples/phase-u3b-lut'
FINGERPRINT = 'b97116541aeefc6723ef91a1d38c9be52792d47316b573ac08d49484fd97d0af'
POLICY = '3a95c98305aa69e2b7f0502c6e9472f47aea863c3fdf32c12e3644a06a0180c3'
FROZEN_CHECKSUMS = '55eef5e2ba0e4cadbdaef6322b21b2daa6b5fdd0f404f58882a0c6cbb03c6ad9'
GENESIS = '5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d'
SLOT_HASHES = 'SysvarS1otHashes111111111111111111111111111'
STAGES = ['historical_state_acquired', 'execution_slot_visibility_valid', 'addresses_resolved', 'rpc_loaded_addresses_match', 'full_key_space_reconstructed', 'compiled_instruction_identity_match', 'previous_lut_gate_passed', 'next_old_engine_rejection']
read, canonical, require, sha = baseline.read, baseline.canonical, baseline.require, baseline.sha


def frozen_targets(sample=SAMPLE):
    # Check every frozen raw/derived byte before using any input; no new RPC.
    manifest = read(sample / 'manifest.json')
    require(sha((sample / 'checksums.sha256').read_bytes()) == FROZEN_CHECKSUMS, 'frozen U3A checksum inventory differs; stop')
    require(manifest['sample_fingerprint'] == FINGERPRINT, 'frozen U3A fingerprint differs; stop')
    require(sha((sample / 'sampling-policy.json').read_bytes()) == POLICY, 'frozen policy differs; stop')
    for line in (sample / 'checksums.sha256').read_text().splitlines():
        expected, name = line.split('  ', 1)
        require(sha(safe_file(sample, name).read_bytes()) == expected, f'frozen U3A input differs: {name}; stop')
    policy, receipt, source, membership = baseline.load_raw(sample)
    raw_hashes = dict(receipt['raw_artifact_hashes'], **{'capture-receipt.json': sha((sample / 'capture-receipt.json').read_bytes())})
    require(baseline.fingerprint(source, membership, raw_hashes, POLICY) == FINGERPRINT, 'recomputed U3A identity differs; stop')
    require(receipt['genesis_hash'] == GENESIS, 'frozen network differs')
    members = {m['signature']: m for m in membership}
    targets = []
    for row in read(sample / 'before-rejections.json'):
        if row['loaded_address_count'] == 0:
            continue
        require(row['old_engine']['first_blocker']['detail'].endswith('lookup tables are normalized but not executed'), 'unexpected target first blocker')
        member = members[row['signature']]
        capture = safe_file(sample, member['response_file']).read_bytes()
        require(sha(capture) == row['old_engine']['capture_sha256'], 'BEFORE/capture linkage differs')
        rpc = json.loads(capture)['result']
        message, meta = rpc['transaction']['message'], rpc['meta']
        require(rpc['transaction']['signatures'][0] == row['signature'] and rpc['slot'] == row['slot'] and rpc['version'] == 0, 'frozen target identity differs')
        targets.append({'signature': row['signature'], 'execution_slot': rpc['slot'], 'capture_file': member['response_file'], 'capture_sha256': sha(capture), 'message_version': 0, 'header': message['header'], 'recent_blockhash': message['recentBlockhash'], 'static_account_keys': message['accountKeys'], 'compiled_instructions': message['instructions'], 'address_table_lookups': message['addressTableLookups'], 'rpc_loaded_writable': meta['loadedAddresses']['writable'], 'rpc_loaded_readonly': meta['loadedAddresses']['readonly'], 'recognized_actions': row['recognized_actions'], 'on_chain_success': meta['err'] is None, 'on_chain_error': meta['err'], 'before_first_blocker': row['old_engine']['first_blocker']})
    require(len(targets) == 8, 'eight-target denominator differs')
    return targets


def safe_file(root, name):
    path = (root / name).resolve()
    require(path.is_relative_to(root.resolve()), 'artifact path leaves evidence directory')
    return path


def requested_contexts(targets):
    return sorted({(lookup['accountKey'], target['execution_slot']) for target in targets for lookup in target['address_table_lookups']})


def pubkey_text(raw):
    # Display official decoded Address values as pubkeys; never parse ALT bytes.
    alphabet='123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
    value=int.from_bytes(bytes(raw),'big');encoded=''
    while value:
        value,remainder=divmod(value,58);encoded=alphabet[remainder]+encoded
    return '1'*(len(raw)-len(bytes(raw).lstrip(b'\0')))+encoded


def validate_capture(root, targets):
    capture = read(root / 'acquisition.json')
    require(capture['sample_fingerprint'] == FINGERPRINT and capture['kind'] == 'experimental_historical_lut_capture' and capture['complete'] is True, 'invalid U3B capture')
    wanted = requested_contexts(targets)
    rows = capture['requests']
    keys = [(r['table_pubkey'], r['execution_slot']) for r in rows]
    require(len(set(keys)) == len(keys) and sorted(keys) == wanted, 'historical request membership differs')
    for name, expected in capture['raw_artifact_hashes'].items():
        require(sha(safe_file(root, name).read_bytes()) == expected, 'historical raw artifact hash differs')
    provider = capture.get('provider')
    if provider:
        baseline.hygiene(provider)
        require(provider['genesis_hash'] == GENESIS and provider['visibility'] == 'finalized_end_of_execution_slot', 'archive network/semantics differ')
        genesis_file=capture.get('genesis_response_file')
        require(genesis_file in capture['raw_artifact_hashes'] and read(safe_file(root,genesis_file)).get('result') == GENESIS, 'archive raw genesis differs or is absent')
        validation = safe_file(root, capture['archive_validation_file']).read_bytes()
        require(sha(validation) == provider['validation_artifact_sha256'], 'archive validation identity differs')
        validate_archive_validation(json.loads(validation), provider, root, capture['raw_artifact_hashes'])
    for request in rows + capture.get('slot_hashes_requests', []):
        require(request['status'] in ('success', 'failure'), 'unresolved historical request')
        require(request['requested_slot'] == request['execution_slot'], 'historical query boundary differs')
        require(request['method'] == 'getAccountInfo' and request['params'] == [request['table_pubkey'], {'encoding': 'base64', 'commitment': 'finalized', 'slot': request['execution_slot']}], 'historical request parameters differ')
        if request in capture.get('slot_hashes_requests', []):
            require(request['table_pubkey'] == SLOT_HASHES, 'wrong historical SlotHashes identity')
        if request.get('response_file'):
            require(request['response_file'] in capture['raw_artifact_hashes'] and request['response_sha256'] == capture['raw_artifact_hashes'][request['response_file']], 'unbound failure/success response')
        if request['status'] == 'success':
            require(provider is not None, 'historical success without validated provider')
            name = request['response_file']
            require(name in capture['raw_artifact_hashes'] and request['response_sha256'] == capture['raw_artifact_hashes'][name], 'unbound historical response')
            raw = read(safe_file(root, name))
            require(raw.get('error') is None and raw['result']['context']['slot'] == request['execution_slot'] and request['returned_context_slot'] == request['execution_slot'], 'archive context differs')
            if capture.get('schema_version') == 2 and request in rows:
                require(request['attempted'] is True, 'historical success was not attempted')
                account=raw['result']['value']
                require(request['decoded_account_sha256'] == sha(base64.b64decode(account['data'][0],validate=True)), 'decoded historical account hash differs')
                require(all(request[k] == account[k] for k in ('owner','lamports','executable')), 'historical account attributes differ')
        else:
            require(request['failure_reason'], 'failure membership lacks reason')
        if capture.get('schema_version') == 2 and request in rows:
            signatures=[t['signature'] for t in targets if t['execution_slot']==request['execution_slot'] and any(l['accountKey']==request['table_pubkey'] for l in t['address_table_lookups'])]
            require(request['transaction_signatures']==signatures, 'historical transaction membership differs')
    hashes_slots = [r['execution_slot'] for r in capture.get('slot_hashes_requests', [])]
    require(len(set(hashes_slots)) == len(hashes_slots) and set(hashes_slots).issubset({t['execution_slot'] for t in targets}), 'SlotHashes membership differs')
    return capture


def validate_archive_validation(validation, provider, root, raw_hashes):
    if validation['kind'] == 'existing_historical_archive_contract':
        require(validation['provider'] == provider['scheme_host'] and validation['genesis_hash'] == GENESIS and validation['visibility'] == provider['visibility'], 'provider contract provenance differs')
        require(validation['qualification_reused'] is True and validation['lut_specific_cross_check_required'] is True, 'provider contract cannot waive LUT proof')
        for name, expected in validation['qualification_artifacts'].items():
            require(name in raw_hashes and raw_hashes[name] == expected, 'prior provider qualification artifact not bound')
        require(len(validation['qualification_artifacts']) >= 3, 'prior provider qualification evidence absent')
        if validation.get('optional_validation_file'):
            name=validation['optional_validation_file']
            require(name in raw_hashes, 'optional validation not retained')
            validate_archive_validation(read(safe_file(root,name)),provider,root,raw_hashes)
        return
    require(validation['kind'] == 'historical_lut_archive_validation' and validation['passed'] is True and validation['synthetic'] is False, 'production archive capability not validated')
    require(validation['provider'] == provider['scheme_host'] and validation['genesis_hash'] == GENESIS and validation['visibility'] == provider['visibility'], 'validation provenance differs')
    require(validation['semantics_reference'] and validation['independent_fixture_reference'], 'archive semantics and independent byte expectations required')
    fixture_file=validation['independent_fixture_file']
    require(fixture_file in raw_hashes and raw_hashes[fixture_file] == validation['independent_fixture_sha256'], 'independent fixture not retained or hash differs')
    fixture=read(safe_file(root,fixture_file))
    require(fixture['kind'] == 'independent_historical_lut_validation_fixture' and fixture['synthetic'] is False, 'independent fixture is not real historical evidence')
    require(fixture['semantics_reference'] == validation['semantics_reference'] and fixture['independent_fixture_reference'] == validation['independent_fixture_reference'], 'fixture provenance differs')
    observations = validation['observations']
    require(len(observations) >= 2 and len({o['requested_slot'] for o in observations}) >= 2 and len({o['expected_account_sha256'] for o in observations}) >= 2 and len({o['pubkey'] for o in observations}) == 1, 'historically divergent LUT validation fixture required')
    require([{k:o[k]for k in ('pubkey','requested_slot','expected_account_sha256')}for o in observations] == [{k:o[k]for k in ('pubkey','requested_slot','expected_account_sha256')}for o in fixture['observations']], 'independent fixture observations differ')
    for o in observations:
        require(o['response_file'] in raw_hashes and sha(safe_file(root,o['response_file']).read_bytes()) == raw_hashes[o['response_file']], 'validation response not retained')
        response = read(safe_file(root,o['response_file']))['result']
        require(response['context']['slot'] == o['requested_slot'], 'validation context differs')
        account = response['value']
        require(account['owner'] == 'AddressLookupTab1e1111111111111111111111111' and account['data'][1] == 'base64', 'validation LUT owner/encoding differs')
        require(sha(base64.b64decode(account['data'][0], validate=True)) == o['expected_account_sha256'], 'archive differs from independent historical byte expectation')


def derive(root=EVIDENCE, sample=SAMPLE, reverse=False, workers=1, timings=None):
    validation_started=time.perf_counter()
    targets = frozen_targets(sample)
    capture = validate_capture(root, targets)
    validation_seconds=time.perf_counter()-validation_started
    membership = {(r['table_pubkey'], r['execution_slot']): r for r in capture['requests']}
    slots = {r['execution_slot']: r for r in capture.get('slot_hashes_requests', [])}
    inputs=[]
    for t in targets:
        evidence=[]
        lookups = t['address_table_lookups'][::-1] if reverse else t['address_table_lookups']
        entries = [membership[l['accountKey'], t['execution_slot']] for l in lookups]
        if t['execution_slot'] in slots:
            entries.append(slots[t['execution_slot']])
        for r in entries:
            if r['status'] == 'success':
                evidence.append({'pubkey': r['table_pubkey'], 'provider': capture['provider'], 'raw_response_base64': base64.b64encode(safe_file(root,r['response_file']).read_bytes()).decode()})
        inputs.append({'result':read(safe_file(sample,t['capture_file']))['result'], 'genesis':GENESIS, 'evidence':evidence})
    if reverse:
        inputs.reverse()
    # Remove all acquisition configuration; binary has no transport calls.
    env={k:v for k,v in os.environ.items() if not any(w in k for w in ('RPC','API_KEY','ARCHIVE','ALCHEMY','HELIUS'))}
    binary=REPO/'target/debug/examples/reconstruct_lut'
    def run(batch):
        p=subprocess.run([str(binary)],input=canonical(batch),capture_output=True,env=env,check=True)
        return json.loads(p.stdout)
    derivation_started=time.perf_counter()
    if workers==1:
        results=run(inputs)
    else:
        from concurrent.futures import ThreadPoolExecutor
        with ThreadPoolExecutor(max_workers=workers) as pool:
            results=[r for batch in pool.map(lambda row:run([row]),inputs) for r in batch]
    by_signature={r['signature']:r for r in results}
    derivation_seconds=time.perf_counter()-derivation_started
    require(len(by_signature)==8, 'proof result membership differs')
    outputs={'targets.json':canonical({'sample_fingerprint':FINGERPRINT,'targets':targets})}
    stage_rows=[]
    for t in targets:
        r=by_signature[t['signature']]
        entries=[membership[l['accountKey'],t['execution_slot']] for l in t['address_table_lookups']]
        failure=r['failure']
        # Retain acquisition reasons instead of generic missing-proof diagnosis.
        missing=[{'table_pubkey':e['table_pubkey'],'reason':e['failure_reason']}for e in entries if e['status']=='failure']
        if missing:
            require(failure and failure['stage']==1, 'missing raw state was accepted')
            failure=dict(failure, acquisition_failures=missing)
        if capture.get('schema_version') == 2 and failure:
            failure=dict(failure, category=failure_category(failure, entries, t['address_table_lookups']))
        stages=[{'stage':i,'name':name,'status':('passed' if not failure or i<failure['stage'] else 'failed' if i==failure['stage'] else 'not_attempted')}for i,name in enumerate(STAGES,1)]
        artifact={'kind':'experimental_transaction_lut_evidence','sample_fingerprint':FINGERPRINT,'signature':t['signature'],'execution_slot':t['execution_slot'],'capture_sha256':t['capture_sha256'],'table_membership':entries,'stages':stages,**r,'failure':failure}
        if capture.get('schema_version') == 2:
            diagnostics=[]
            if r['proof']:
                for table in r['proof']['tables']:
                    diagnostic={k:table[k]for k in ('table_pubkey','requested_slot','returned_context_slot','raw_account_sha256','raw_response_sha256','metadata','active_addresses_len','writable_indexes','readonly_indexes')}
                    diagnostic.update(resolved_writable=[pubkey_text(a) for a in table['resolved_writable']],resolved_readonly=[pubkey_text(a) for a in table['resolved_readonly']],proof_status='proven')
                    diagnostics.append(diagnostic)
            else:
                for lookup, entry in zip(t['address_table_lookups'],entries):
                    diagnostics.append({'table_pubkey':lookup['accountKey'],'writable_indexes':lookup['writableIndexes'],'readonly_indexes':lookup['readonlyIndexes'],'resolved_writable':None,'resolved_readonly':None,'metadata':entry.get('metadata'),'proof_status':'unproven','failure_category':entry.get('failure_category') or failure['category']})
            artifact.update(historically_lut_proven=r['proof'] is not None, message_reconstruction_ready=r['proof'] is not None, advanced_past_lut_gate=r['passed_previous_lut_gate'], table_diagnostics=diagnostics)
            if r['proof']:
                keys=[k['address']for k in r['proof']['full_account_keys']]
                artifact['compiled_instruction_diagnostics']=[{'outer_index':i,'program_index':ix['programIdIndex'],'program_pubkey':keys[ix['programIdIndex']],'account_indexes':ix['accounts'],'account_pubkeys':[keys[a]for a in ix['accounts']],'raw_data_base64':base64.b64encode(baseline.b58decode(ix['data'])).decode()}for i,ix in enumerate(t['compiled_instructions'])]
        outputs[f"proofs/{t['signature']}.json"]=canonical(artifact)
        stage_rows.append({'signature':t['signature'],'execution_slot':t['execution_slot'],'on_chain_success':t['on_chain_success'],'lookup_table_count':len(t['address_table_lookups']),'loaded_address_count':len(t['rpc_loaded_writable'])+len(t['rpc_loaded_readonly']),'passed_previous_lut_gate':r['passed_previous_lut_gate'],'failure':failure,'next_old_engine_rejection':r['next_old_engine_rejection'],'stages':stages})
    passed=sum(r['passed_previous_lut_gate'] for r in stage_rows)
    if timings is not None:
        timings.update(frozen_and_capture_integrity_seconds=validation_seconds,offline_message_derivation_seconds=derivation_seconds,real_historical_proofs_derived=passed,real_historical_decode_timing='unavailable'if passed==0 else 'included_in_message_derivation')
    outputs['stage-table.json']=canonical({'sample_fingerprint':FINGERPRINT,'before_lut_blocked':8,'after_exact_historical_proof':passed,'after_unproven':8-passed,'production_baseline_replays':0,'rows':stage_rows})
    states={r['response_sha256'] for r in capture['requests'] if r['status']=='success'}
    account_states={}
    referenced_account_bytes=0
    for r in capture['requests']:
        if r['status']=='success':
            account_bytes=base64.b64decode(read(safe_file(root,r['response_file']))['result']['value']['data'][0],validate=True)
            account_states[(r['table_pubkey'],sha(account_bytes))]=len(account_bytes)
    for t in targets:
        for l in t['address_table_lookups']:
            r=membership[l['accountKey'],t['execution_slot']]
            if r['status']=='success':
                referenced_account_bytes+=len(base64.b64decode(read(safe_file(root,r['response_file']))['result']['value']['data'][0],validate=True))
    outputs['summary.json']=canonical({'sample_fingerprint':FINGERPRINT,'target_transactions':8,'unique_lut_accounts':len({k for k,s in requested_contexts(targets)}),'table_slot_contexts':len(membership),'acquired_contexts':sum(r['status']=='success'for r in capture['requests']),'distinct_raw_response_states':len(states),'distinct_historical_lut_states':len(account_states),'raw_account_bytes_distinct':sum(account_states.values()),'raw_account_bytes_referenced':referenced_account_bytes,'raw_account_duplication_bytes':referenced_account_bytes-sum(account_states.values()),'raw_artifact_bytes':sum(safe_file(root,n).stat().st_size for n in capture['raw_artifact_hashes']),'transaction_evidence_bytes':sum(len(v)for k,v in outputs.items()if k.startswith('proofs/')),'decoded_proof_bytes':sum(len(canonical(r['proof']))for r in results if r['proof']),'exact_historical_proofs':passed,'unproven_transactions':8-passed,'next_blockers_measured':passed,'decision':'A. HISTORICAL LUT RECONSTRUCTION PROVEN' if passed==8 else 'B. PARTIAL LUT RECONSTRUCTION — SOME FROZEN TRANSACTIONS REMAIN UNPROVABLE','decision_qualification':'No real historical proof established' if passed==0 else 'Partial real proof established' if passed<8 else 'Strong A: all eight','no_protocol_accounts_acquired':True,'replay_record_schema_unchanged':True})
    if capture.get('schema_version') == 2:
        summary=json.loads(outputs['summary.json'])
        summary.update(historically_lut_proven=sum(r['proof'] is not None for r in results), advanced_past_lut_gate=passed, optional_divergent_fixture_required=False)
        if passed:
            summary['decision']='A. HISTORICAL LUT RECONSTRUCTION PROVEN'
        elif any(row['failure']['category'] not in ('archive_unavailable','historical_state_missing','slot_hashes_missing') for row in stage_rows):
            summary['decision']='C. HISTORICAL LUT RECONSTRUCTION FAILED / ASSUMPTION INVALID'
        summary['failure_distribution']={category:sum(row['failure'] is not None and row['failure']['category']==category for row in stage_rows) for category in sorted({row['failure']['category'] for row in stage_rows if row['failure']})}
        outputs['summary.json']=canonical(summary)
    outputs['checksums.sha256']=''.join(f'{sha(v)}  {n}\n'for n,v in sorted(outputs.items())).encode()
    return outputs


def failure_category(failure, entries, lookups=()):
    categories=[e.get('failure_category') for e in entries if e['status']=='failure']
    if categories and categories[0]:
        return categories[0]
    if failure['stage'] == 3:
        for entry, lookup in zip(entries,lookups):
            meta=entry.get('metadata')
            if meta and meta['last_extended_slot']==entry['execution_slot'] and any(meta['last_extended_slot_start_index'] <= index < entry['address_count'] for index in lookup['writableIndexes']+lookup['readonlyIndexes']):
                return 'warmup_visibility_failure'
    text=failure['detail'].lower()
    for needle, category in [('slothashes','slot_hashes_missing'),('context','slot_context_mismatch'),('future slot','slot_context_mismatch'),('owner','wrong_owner'),('decode','malformed_table'),('metadata/address length','malformed_table'),('visibility','deactivated'),('lookup:','lookup_index_invalid'),('loaded addresses differ','rpc_loaded_address_mismatch'),('accountloadedtwice','duplicate_account'),('compiled','compiled_index_mismatch'),('unavailable','historical_state_missing'),('missing or extra historical','historical_state_missing')]:
        if needle in text:
            return category
    return 'other'


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sample',type=Path,default=SAMPLE)
    parser.add_argument('--evidence',type=Path,default=EVIDENCE)
    parser.add_argument('--output',type=Path)
    parser.add_argument('--verify',action='store_true')
    parser.add_argument('--reverse',action='store_true')
    parser.add_argument('--workers',type=int,default=1)
    parser.add_argument('--timing-output',type=Path)
    args=parser.parse_args()
    require(args.workers>0,'worker count must be positive')
    require(not args.verify or args.output is None,'verify cannot write artifacts')
    timings={}
    start=time.perf_counter();outputs=derive(args.evidence,args.sample,args.reverse,args.workers,timings);elapsed=time.perf_counter()-start
    for n,v in outputs.items():
        if args.verify:
            require(safe_file(args.evidence,n).read_bytes()==v,f'U3B derived bytes differ: {n}')
        else:
            target=safe_file(args.output or args.evidence,n);require(not target.is_relative_to(args.sample.resolve()),'cannot write frozen sample');target.parent.mkdir(parents=True,exist_ok=True);target.write_bytes(v)
    if args.timing_output:
        require(not args.timing_output.resolve().is_relative_to(args.sample.resolve()),'cannot write frozen sample')
        args.timing_output.parent.mkdir(parents=True,exist_ok=True)
        args.timing_output.write_bytes(canonical({'offline_rebuild_seconds':elapsed,'includes_frozen_integrity_checks':True,'workers':args.workers,'reverse':args.reverse,'network_transport_called':False,**timings}))
    print(outputs['summary.json'].decode(),end='')

if __name__=='__main__':
    main()
