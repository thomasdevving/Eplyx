#!/usr/bin/env python3
"""Separate provider-neutral exact-slot LUT acquisition; never changes U3A.

Reads SOLANA_ARCHIVE_RPC_URL for an already qualified historical provider.
A divergent LUT fixture is optional defense in depth. No
fallback to transaction/current RPC and no overwriting an existing attempt.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import subprocess
import time
from urllib.parse import urlsplit
import kamino_u3b_lut as lut


def write(path, value):
    path.parent.mkdir(parents=True,exist_ok=True)
    path.write_bytes(lut.canonical(value))


def safe_body(body):
    # Do not serialize endpoint echoes or credential-bearing provider responses.
    # Failure membership explicitly records the withheld response hash.
    try:
        value=json.loads(body)
    except (ValueError, UnicodeError):
        value=body.decode('utf8',errors='replace')
    lut.baseline.hygiene(value)


def returned_context(value):
    result=value.get('result')if isinstance(value,dict)else None
    context=result.get('context')if isinstance(result,dict)else None
    return context.get('slot')if isinstance(context,dict)else None


class RawArchive:
    def __init__(self, endpoint, origin=''):
        parsed=urlsplit(endpoint)
        lut.require(parsed.scheme in ('http','https') and parsed.hostname and not parsed.username and not parsed.password,'archive requires HTTP(S) without URL userinfo')
        lut.require(not any(c in endpoint+origin for c in '\r\n"\\'),'invalid archive transport configuration')
        self.identity=f'{parsed.scheme}://{parsed.hostname}'
        if parsed.port:
            self.identity+=f':{parsed.port}'
        self.endpoint,self.origin=endpoint,origin

    def call(self,method,params):
        payload=json.dumps({'jsonrpc':'2.0','id':1,'method':method,'params':params})
        config=f'url = "{self.endpoint}"\nheader = "Content-Type: application/json"\n'
        if self.origin:
            config+=f'header = "Origin: {self.origin}"\n'
        try:
            result=subprocess.run(['curl','--config','-','--silent','--max-time','15','--request','POST','--data',payload],input=config.encode(),capture_output=True,timeout=18)
            body=result.stdout
            error=f'transport_exit_{result.returncode}'if result.returncode else None
        except subprocess.TimeoutExpired as timeout:
            body,error=timeout.stdout or b'','transport_timeout'
        try:
            value=json.loads(body)
        except (ValueError,UnicodeError):
            return body,None,error or 'invalid_json_response'
        if not isinstance(value,dict):
            return body,value,error or 'invalid_rpc_envelope'
        if value.get('error') is not None:
            return body,value,error or f"rpc_error_{value['error'].get('code','unknown')}"
        if value.get('result') is None:
            return body,value,error or 'null_result'
        return body,value,error


def capture(root, targets, rpc=None, fixture=None, unavailable_reason=None):
    capture_started=time.perf_counter()
    lut.require(not root.resolve().is_relative_to(lut.SAMPLE.resolve()),'cannot write frozen U3A')
    lut.require(not root.exists(),'capture directory already exists; use a separate version/attempt')
    root.mkdir(parents=True)
    contexts=lut.requested_contexts(targets)
    requests=[{'table_pubkey':key,'execution_slot':slot,'requested_slot':slot,'method':'getAccountInfo','params':[key,{'encoding':'base64','commitment':'finalized','slot':slot}], 'attempted':False,'transaction_signatures':[t['signature'] for t in targets if t['execution_slot']==slot and any(l['accountKey']==key for l in t['address_table_lookups'])],'decoded_account_sha256':None,'failure_category':'archive_unavailable','status':'failure','failure_reason':unavailable_reason or 'acquisition_not_started','returned_context_slot':None,'response_file':None,'response_sha256':None}for key,slot in contexts]
    manifest={'schema_version':2,'kind':'experimental_historical_lut_capture','complete':False,'sample_fingerprint':lut.FINGERPRINT,'provider':None,'archive_validation_file':None,'requests':requests,'slot_hashes_requests':[],'raw_artifact_hashes':{},'attempt_policy':{'max_attempts_per_context':1,'timeout_seconds':15,'provider_fallback':False}}
    write(root/'acquisition.json',manifest)
    elapsed=[]
    def call(name,method,params):
        start=time.perf_counter();body,value,error=rpc.call(method,params);elapsed.append({'request':name,'seconds':time.perf_counter()-start})
        reference=f'rpc/{name}.body'
        if body:
            try:
                safe_body(body)
            except ValueError:
                return None,None,'sensitive_response_withheld',lut.sha(body)
            path=root/reference;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(body)
            manifest['raw_artifact_hashes'][reference]=lut.sha(body)
        else:
            reference=None
        return reference,value,error,lut.sha(body) if body else None
    failure=unavailable_reason
    if rpc:
        name,value,error,digest=call('genesis','getGenesisHash',[])
        manifest['genesis_response_file']=name
        if error or value['result']!=lut.GENESIS:
            failure=error or 'archive_genesis_mismatch'
        else:
            # Layer 1 reuses Eplyx's previously validated provider contract.
            # Configuring an endpoint selects that qualified provider; this is
            # explicit provider trust, not new empirical qualification or a
            # cryptographic assertion. Layer 2 must still prove every LUT.
            qualification={
                'kind':'prior_historical_archive_qualification',
                'reference':'docs/phase-6-historical-state.md',
                'reference_sha256':lut.sha((lut.REPO/'docs/phase-6-historical-state.md').read_bytes()),
                'mainnet_replay_pre_state_sha256':'5b29e923314147626739e5f1a3af6ce22af68ed43601e77f0d789f31e1182687',
                'mainnet_replay_post_state_sha256':'c447c3c994cbdc7671153c353d4669b766984d88fd7f4e9d94d768636d58063a',
                'boundary':'latest account write at or before finalized requested slot',
                'qualification_reused_per_user_instruction':True}
            retained={
                'qualification/record.json':(lut.REPO/'docs/examples/mainnet-replay-record.json').read_bytes(),
                'qualification/provider.rs':(lut.REPO/'engine/src/historical.rs').read_bytes(),
                'qualification/reference.json':lut.canonical(qualification)}
            for ref,body in retained.items():
                path=root/ref;path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(body)
                manifest['raw_artifact_hashes'][ref]=lut.sha(body)
            validation={'kind':'existing_historical_archive_contract','provider':rpc.identity,'genesis_hash':lut.GENESIS,'visibility':'finalized_end_of_execution_slot','qualification_reused':True,'trust_boundary':'operator selects the previously qualified archive using environment configuration','lut_specific_cross_check_required':True,'qualification_artifacts':{ref:lut.sha(body) for ref,body in retained.items()},'optional_divergent_fixture':'not_supplied'}
            if fixture is not None:
                # This additional witness is retained and checked, but does not
                # establish/replace provider trust and is never mandatory.
                lut.require(fixture.get('kind')=='independent_historical_lut_validation_fixture' and fixture.get('synthetic') is False,'optional fixture must contain independent real historical expectations')
                safe_body(lut.canonical(fixture))
                write(root/'independent-validation-fixture.json',fixture)
                manifest['raw_artifact_hashes']['independent-validation-fixture.json']=lut.sha((root/'independent-validation-fixture.json').read_bytes())
                optional={'kind':'historical_lut_archive_validation','passed':True,'synthetic':False,'provider':rpc.identity,'genesis_hash':lut.GENESIS,'visibility':validation['visibility'],'semantics_reference':fixture['semantics_reference'],'independent_fixture_reference':fixture['independent_fixture_reference'],'observations':[],'independent_fixture_file':'independent-validation-fixture.json','independent_fixture_sha256':manifest['raw_artifact_hashes']['independent-validation-fixture.json']}
                for index,o in enumerate(fixture['observations']):
                    key,slot=o['pubkey'],o['requested_slot']
                    lut.require(len(lut.baseline.b58decode(key))==32 and type(slot)is int and slot>=0,'invalid optional fixture query')
                    name,value,error,digest=call(f'validation-{index}','getAccountInfo',[key,{'encoding':'base64','commitment':'finalized','slot':slot}])
                    optional['observations'].append(dict(o,response_file=name,observed_error=error))
                candidate={'scheme_host':rpc.identity,'genesis_hash':lut.GENESIS,'visibility':validation['visibility']}
                try:
                    lut.validate_archive_validation(optional,candidate,root,manifest['raw_artifact_hashes'])
                    validation['optional_divergent_fixture']='passed'
                except (ValueError,KeyError,TypeError):
                    optional['passed']=False
                    validation['optional_divergent_fixture']='failed'
                    failure='optional_independent_fixture_contradicts_archive'
                write(root/'optional-archive-validation.json',optional)
                manifest['raw_artifact_hashes']['optional-archive-validation.json']=lut.sha((root/'optional-archive-validation.json').read_bytes())
                validation['optional_validation_file']='optional-archive-validation.json'
            manifest['archive_validation_file']='archive-validation.json'
            write(root/'archive-validation.json',validation)
            manifest['provider']={'scheme_host':rpc.identity,'genesis_hash':lut.GENESIS,'visibility':validation['visibility'],'validation_artifact_sha256':lut.sha((root/'archive-validation.json').read_bytes())}
    if failure:
        for r in requests:
            r['failure_reason']=failure
    else:
        for index,r in enumerate(requests):
            r['attempted']=True
            name,value,error,digest=call(f'table-{index}','getAccountInfo',r['params'])
            context=returned_context(value)
            if not error and context!=r['execution_slot']:
                error='archive_did_not_honor_exact_execution_slot'
            if not error and value['result'].get('value') is None:
                error='historical_table_absent'
            r.update(failure_category=('slot_context_mismatch'if error=='archive_did_not_honor_exact_execution_slot' else 'historical_state_missing'if error in ('historical_table_absent','null_result') else 'archive_unavailable')if error else None,status='failure'if error else 'success',failure_reason=error,response_file=name,response_sha256=digest,returned_context_slot=context)
            write(root/'acquisition.json',manifest)
        # Capture SlotHashes only if decoded official table metadata needs it.
        # The read-only decoder below uses the pinned official interface.
        binary=lut.REPO/'target/debug/examples/inspect_lut'
        needed=set()
        for r in requests:
            if r['status']=='success':
                body=(root/r['response_file']).read_bytes()
                raw=json.loads(body)['result']['value']
                try:
                    r.update(decoded_account_sha256=lut.sha(base64.b64decode(raw['data'][0],validate=True)),owner=raw['owner'],lamports=raw['lamports'],executable=raw['executable'])
                except (ValueError,KeyError,TypeError,IndexError):
                    r.update(status='failure',failure_reason='malformed_historical_account_envelope',failure_category='malformed_table')
                    continue
                result=subprocess.run([str(binary)],input=body,capture_output=True)
                if result.returncode==0:
                    decoded=json.loads(result.stdout)
                    meta=decoded['metadata']
                    r.update(metadata=meta,address_count=decoded['address_count'])
                    if meta['last_extended_slot']<=r['execution_slot'] and meta['deactivation_slot']<r['execution_slot']:
                        needed.add(r['execution_slot'])
                else:
                    r['official_decode_status']='rejected'
        for index,slot in enumerate(sorted(needed)):
            params=[lut.SLOT_HASHES,{'encoding':'base64','commitment':'finalized','slot':slot}]
            name,value,error,digest=call(f'slot-hashes-{index}','getAccountInfo',params)
            context=returned_context(value)
            if not error and context!=slot:
                error='archive_did_not_honor_exact_execution_slot'
            if not error and value['result'].get('value') is None:
                error='historical_slot_hashes_absent'
            manifest['slot_hashes_requests'].append({'attempted':True,'table_pubkey':lut.SLOT_HASHES,'execution_slot':slot,'requested_slot':slot,'method':'getAccountInfo','params':params,'status':'failure'if error else 'success','failure_reason':error,'returned_context_slot':context,'response_file':name,'response_sha256':digest})
    manifest['complete']=True
    write(root/'acquisition.json',manifest)
    write(root/'acquisition-timing.json',{'network_requests':len(elapsed),'account_state_requests':sum(r['request']!='genesis'for r in elapsed),'successful_table_requests':sum(r['status']=='success'for r in requests),'failed_table_requests':sum(r['attempted'] and r['status']=='failure'for r in requests),'requests':elapsed,'acquisition_seconds':sum(r['seconds']for r in elapsed),'full_capture_wall_clock_seconds':time.perf_counter()-capture_started,'measurement':'not_attempted'if not rpc else 'wall_clock_transport','historical_contexts_acquired':sum(r['status']=='success'for r in requests)})
    return manifest


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--sample',type=Path,default=lut.SAMPLE)
    parser.add_argument('--validation-fixture',type=Path)
    parser.add_argument('--record-unavailable',action='store_true')
    args=parser.parse_args()
    targets=lut.frozen_targets(args.sample)
    print(lut.canonical({'sample_fingerprint':lut.FINGERPRINT,'unique_lut_accounts':len({k for k,s in lut.requested_contexts(targets)}),'table_slot_contexts':len(lut.requested_contexts(targets)),'targets':[{'signature':t['signature'],'execution_slot':t['execution_slot'],'lut_descriptors':len(t['address_table_lookups']),'loaded_writable':len(t['rpc_loaded_writable']),'loaded_readonly':len(t['rpc_loaded_readonly']),'tables':[l['accountKey']for l in t['address_table_lookups']]}for t in targets]}).decode(),flush=True)
    if args.record_unavailable:
        lut.require(not os.environ.get('SOLANA_ARCHIVE_RPC_URL'),'archive is configured; do not record configuration unavailable')
        capture(args.output,targets,unavailable_reason='historical_account_archive_not_configured')
    else:
        endpoint=os.environ.get('SOLANA_ARCHIVE_RPC_URL')
        lut.require(endpoint,'configure SOLANA_ARCHIVE_RPC_URL explicitly; no current-RPC fallback')
        rpc=RawArchive(endpoint,os.environ.get('SOLANA_ARCHIVE_RPC_ORIGIN',''))
        capture(args.output,targets,rpc,lut.read(args.validation_fixture)if args.validation_fixture else None)
    print('U3B attempt recorded; offline proof rebuild is separate.')

if __name__=='__main__':
    try:
        main()
    except Exception:
        # Transport/provider errors may embed secrets. Inspect retained safe
        # evidence and membership; never print exception objects from acquisition.
        print('U3B capture stopped; inspect safe failure membership. Endpoint redacted.',file=__import__('sys').stderr)
        raise SystemExit(1)
