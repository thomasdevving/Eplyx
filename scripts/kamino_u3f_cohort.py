#!/usr/bin/env python3
"""Offline full-native-v0 cohort experiments; raw evidence precedes execution."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import kamino_u3f_cohort_state as s
b,f,lut,t=s.b,s.f,s.lut,s.t


def checked_client(root):
    receipt=lut.read(root/'receipt.json')
    contexts=list({t.request_id(a['method'],a['params']):(a['method'],a['params']) for a in receipt['attempts']}.values())
    # Actual role/absence validators are applied again below; transport evidence
    # must independently satisfy its original wire classification.
    if root.name=='phase-u3f-cohort-state':
        validators={};allow=set()
        for name in ('T2','T3'):
            row,binaries=s.row_for(name)
            for item in s.state.state_plan(row,binaries):
                key=t.request_id(item['method'],item['params'])
                validators[key]=lambda value,r=row,i=item:s.state.account_check(r,i,value)
                if s.absent_allowed(row,item):allow.add(key)
            for namevar,address in f.runtime.REQUIREMENTS[:3]:
                item={'name':namevar,'params':[address,{'encoding':'base64','commitment':'finalized','slot':row['transaction']['slot']}]}
                validators[t.request_id('getAccountInfo',item['params'])]=lambda value,r=row,i=item:s.runtime_check(r,i,value)
        groups=t.verify(root,contexts,validators,allow)
    else:groups=t.verify(root,contexts)
    result={}
    for key,attempts in groups.items():
        last=attempts[-1]
        if last['failure_class'] is None:
            raw=root/last['body_file']
            result[key]={'result':lut.read(raw)['result'],'sources':[{'file':str(raw.relative_to(lut.REPO)),'sha256':last['body_sha256']}], 'method':last['method'],'params':last['params']}
    return result


def binary_cache():
    cache=b.retained()
    cache.update(checked_client(f.ROOT/'phase-u3f-cohort-binaries'))
    # Original resolver usage supplies exact request metadata also for reused data.
    for use in lut.read(f.ROOT/'phase-u3f-cohort-binaries/resolver-requests.json'):
        key=t.request_id(use['method'],use['params'])
        lut.require(key in cache,'missing offline dependency response')
        cache[key].update(method=use['method'],params=use['params'])
    return cache


def prove_binaries(row,expected,cache):
    with tempfile.TemporaryDirectory(prefix='eplyx-u3f-binaries-') as tmp:
        process=subprocess.Popen([str(lut.REPO/'target/debug/examples/acquire_envelope_dependencies')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        process.stdin.write(json.dumps({'transaction':row['transaction'],'output':tmp})+'\n');process.stdin.flush()
        actual=None
        for line in process.stdout:
            message=json.loads(line)
            if message['kind']=='capture_result':actual=message['result'];continue
            key=t.request_id(message['method'],message['params'])
            lut.require(key in cache,'offline resolver requested uncaptured context')
            process.stdin.write(json.dumps({'result':cache[key]['result']})+'\n');process.stdin.flush()
        stderr=process.stderr.read();code=process.wait();process.stdin.close();process.stdout.close();process.stderr.close();lut.require(code==0,stderr)
        lut.require(actual==expected and actual['failure'] is None,'binary/screen revalidation differs')
        for program in actual['programs']:
            if program['source']!='builtin':lut.require(lut.sha((Path(tmp)/program['binary_file']).read_bytes())==program['provenance']['sha256'],'reconstructed ELF differs')


def program_seeds(slot,cache,programs):
    needed={p['program_id'] for p in programs}|{p['provenance']['programdata_address'] for p in programs if p.get('provenance',{}).get('programdata_address')}
    pieces={}
    for entry in cache.values():
        if entry.get('method')!='getAccountInfo' or entry['params'][1]['slot']!=slot-1 or entry['params'][0] not in needed:continue
        address,config=entry['params'];value=entry['result'];a=value['value']
        lut.require(value['context']['slot']==slot-1 and a is not None,'program context differs')
        metadata={k:v for k,v in a.items() if k!='data'};data=base64.b64decode(a['data'][0],validate=True)
        piece=pieces.setdefault(address,{'metadata':metadata,'chunks':{},'sources':[]})
        lut.require(piece['metadata']==metadata,'program chunk metadata differs')
        offset=config.get('dataSlice',{}).get('offset',0)
        if offset in piece['chunks']:lut.require(piece['chunks'][offset]==data,'contradictory chunk')
        piece['chunks'][offset]=data;piece['sources']+=entry['sources']
    lut.require(set(pieces)==needed,'binary seed closure incomplete')
    result=[]
    for address,piece in sorted(pieces.items()):
        data=b''
        for offset,chunk in sorted(piece['chunks'].items()):
            lut.require(offset==len(data),'program byte interval gap');data+=chunk
        lut.require(len(data)==piece['metadata']['space'],'program account incomplete')
        result.append({'address':address,'slot':slot-1,'kind':'program','account':piece['metadata']|{'data':[base64.b64encode(data).decode(),'base64']},'data_sha256':lut.sha(data),'provenance':{'sources':piece['sources']}})
    return result


def prepare(name, state_only=False):
    f.preserved()
    row,binaries=s.row_for(name);tx=row['transaction'];slot=tx['slot']
    plan=s.state.state_plan(row,binaries)
    cache=binary_cache();prove_binaries(row,binaries,cache)
    seeds=program_seeds(slot,cache,binaries['programs'])
    captured=checked_client(f.ROOT/'phase-u3f-cohort-state')
    refs={};absent=[];typed=[]
    target=tx['instructions'][row['envelope']['targets'][0]['outer_index']]['accounts']
    types={target[1]['address']:'Obligation',target[2]['address']:'LendingMarket',target[5]['address']:'Mint'}
    for ix in tx['instructions'][2:row['envelope']['targets'][0]['outer_index']-1]:types[ix['accounts'][0]['address']]='Reserve'
    for token in tx['pre_token_balances']:types[tx['account_keys'][token['account_index']]['address']]='TokenAccount'
    for item in plan:
        key=t.request_id(item['method'],item['params']);lut.require(key in captured,'historical boundary missing')
        entry=captured[key];value=entry['result'];s.state.account_check(row,item,value);a=value['value']
        refs[(item['address'],item['boundary'])]=a
        if a is None:
            lut.require(s.absent_allowed(row,item),'unproved absence')
            if item['boundary']=='pre':absent.append(item['address'])
            continue
        data=base64.b64decode(a['data'][0]);kind=types.get(item['address'])
        if kind:
            if kind in ('Reserve','Obligation','LendingMarket'):lut.require(a['owner']==f.envelope.KLEND if hasattr(f.envelope,'KLEND') else a['owner']=='KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD','protocol owner differs')
            else:lut.require(a['owner'] in ('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA','TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb'),'token/mint owner differs')
            typed.append({'address':item['address'],'boundary':item['boundary'],'type':kind,'account':a})
        if item['boundary']=='pre':seeds.append({'address':item['address'],'slot':slot-1,'kind':'ordinary','account':a,'data_sha256':lut.sha(data),'provenance':entry['sources'][0]})
    for address in {i['address'] for i in plan}:
        pre,post=refs[(address,'pre')],refs[(address,'post')]
        lut.require((pre is None)==(post is None),'unsupported lifecycle')
        if pre:
            lut.require(pre['owner']==post['owner'] and pre['executable']==post['executable'],'owner/executable change')
            meta=next(a for a in tx['account_keys'] if a['address']==address)
            if not meta['is_writable']:lut.require(pre==post,'readonly historical state changed')
    decoded=f.runtime.proof.run_verifier({'mode':'decode','accounts':typed})
    scope=tx['instructions'][0]['accounts'];price,mapping,twap=[base64.b64decode(refs[(a['address'],'pre')]['data'][0]) for a in scope[:3]]
    for label,data in zip(('OraclePrices','OracleMappings','OracleTwaps'),(price,mapping,twap)):lut.require(data[:8]==hashlib.sha256(f'account:{label}'.encode()).digest()[:8],'Scope discriminator')
    lut.require(len(price)==40+512*56 and len(mapping)==8+512*58 and len(twap)==72+512*672,'Scope layout')
    lut.require(price[8:40]==lut.baseline.b58decode(scope[1]['address']) and twap[8:40]==lut.baseline.b58decode(scope[0]['address']) and twap[40:72]==lut.baseline.b58decode(scope[1]['address']),'Scope relationships')
    data=bytes(tx['instructions'][0]['data']) if isinstance(tx['instructions'][0]['data'],list) else None
    # Normalized instruction data is hexadecimal.
    if data is None:data=bytes.fromhex(tx['instructions'][0]['data'])
    tokens=struct.unpack('<'+'H'*((len(data)-12)//2),data[12:])
    for token,a in zip(tokens,scope[4:]):lut.require(mapping[8+32*token:40+32*token]==lut.baseline.b58decode(a['address']),'Scope token mapping differs')
    if state_only:
        return {'target':name,'typed_checks':decoded,'Scope_relationships':True,'paired_ordinary_accounts':len(refs)//2,'binary_and_same_slot_screen_revalidated':True,'historical_runtime_context_complete':False,'runtime_executed':False}
    for var,address in f.runtime.REQUIREMENTS[:3]:
        item={'name':var,'params':[address,{'encoding':'base64','commitment':'finalized','slot':slot}]};entry=captured[t.request_id('getAccountInfo',item['params'])]
        s.runtime_check(row,item,entry['result']);a=entry['result']['value']
        seeds.append({'address':address,'slot':slot,'kind':'runtime','account':a,'data_sha256':lut.sha(base64.b64decode(a['data'][0])),'provenance':entry['sources'][0]})
    _,inputs,_=f.envelope.frozen_inputs();frozen=next(i for i in inputs if i['result']['transaction']['signatures'][0]==tx['signature'])
    proof=lut.read(f.envelope.U3B/f'proofs/{tx["signature"]}.json')['proof']
    for item in frozen['evidence']:
        value=json.loads(base64.b64decode(item['raw_response_base64']))['result'];a=value['value']
        seeds.append({'address':item['pubkey'],'slot':slot,'kind':'lut','account':a,'data_sha256':lut.sha(base64.b64decode(a['data'][0])),'provenance':{'proof_id':proof['proof_id']}})
    watch=[a['address'] for a in tx['account_keys'] if a['address']!=s.state.INSTRUCTIONS]
    pre={a['address']:a['account'] for a in seeds if a['address'] in watch};pre.update({a:None for a in absent})
    post=dict(pre);post.update({address:a for (address,boundary),a in refs.items() if boundary=='post'})
    return {'frozen':frozen,'proof':proof,'programs':binaries['programs'],'seeds':seeds,'absent':absent,'watch':watch},post,decoded


def run(name,output):
    lut.require(not output.exists(),'new experiment directory required')
    payload,refs,typed=prepare(name);output.mkdir(parents=True)
    (output/'typed-proof.json').write_bytes(lut.canonical(typed))
    (output/'input-manifest.json').write_bytes(lut.canonical({k:v for k,v in payload.items() if k not in ('frozen','seeds')}|{'seeds':[{k:v for k,v in a.items() if k!='account'} for a in payload['seeds']]}))
    env={k:v for k,v in os.environ.items() if not any(x in k.upper() for x in ('RPC','ARCHIVE','API_KEY','ALCHEMY','HELIUS'))}
    results=[]
    for variant in ('default','empty','different'):
        payload['variant']=variant
        proc=subprocess.run([str(lut.REPO/'target/debug/examples/execute_envelope_v0')],input=lut.canonical(payload),capture_output=True,env=env,timeout=180)
        (output/f'{variant}.stderr').write_bytes(proc.stderr)
        lut.require(proc.returncode==0,proc.stderr.decode(errors='replace'))
        execution=json.loads(proc.stdout);(output/f'{variant}.json').write_bytes(lut.canonical(execution))
        reconciliation=b.fidelity.compare(payload,execution,refs);reconciliation['decision']=reconciliation['decision'].replace('T1',name)
        (output/f'{variant}-fidelity.json').write_bytes(lut.canonical(reconciliation));results.append(execution)
        print(json.dumps({'target':name,'variant':variant,'success':execution['evidence']['success'],'fidelity':reconciliation['fidelity'],'failures':reconciliation['failures']}),flush=True)
    (output/'summary.json').write_bytes(lut.canonical({'all_succeeded':all(r['evidence']['success'] for r in results),'execution_evidence_identical':all(r['evidence']==results[0]['evidence'] for r in results),'production_replay_eligible':False}))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--target',choices=['T2','T3'],required=True);parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();run(args.target,args.output.resolve())
