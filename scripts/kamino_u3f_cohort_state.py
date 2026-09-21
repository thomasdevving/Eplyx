#!/usr/bin/env python3
"""Paired historical cohort accounts, unchanged interference rules and bounded transport."""
import argparse
import base64
import json
import os
from pathlib import Path
import struct
import kamino_u3f_binaries as b
f, lut, t = b.f, b.lut, b.t
state = f.s.state


def row_for(name):
    result = lut.read(f.ROOT / f'phase-u3f-cohort-binaries/{name}/result.json')
    row = lut.read(f.ROOT / f'phase-u3c-envelope/transactions/{result["signature"]}.json')
    return row, result


def absent_allowed(row,item):
    tx=row['transaction'];target=tx['instructions'][row['envelope']['targets'][0]['outer_index']]
    address=target['accounts'][3]['address']
    index=next(i for i,a in enumerate(tx['account_keys']) if a['address']==item['address'])
    return item['address']==address and tx['pre_balances'][index]==tx['post_balances'][index]==0


def runtime_check(row,item,response):
    slot=row['transaction']['slot'];a=response['value']
    lut.require(response['context']['slot']==slot and a is not None and a['owner']==f.runtime.SYSVAR_OWNER and a['executable'] is False,'historical sysvar context/owner differs')
    data=base64.b64decode(a['data'][0],validate=True)
    lut.require(a['data'][1]=='base64' and a.get('space',len(data))==len(data),'sysvar incomplete')
    formats={'Clock':'<QqQQq','Rent':'<QdB','EpochSchedule':'<QQBQQ'}
    values=struct.unpack(formats[item['name']],data)
    if item['name']=='Clock':lut.require(values[0]==slot and values[-1]==row['transaction']['block_time'],'historical Clock fields disagree')
    return {'sha256':lut.sha(data),'fields':values}


def run(root):
    lut.require(b.fidelity.derive()[0]['decision']=='T1-A','T1 gate')
    qualified=lut.read(f.envelope.U3B/'acquisition.json')['provider']
    rpc=t.CurlArchive(os.environ['SOLANA_ARCHIVE_RPC_URL'],os.environ.get('SOLANA_ARCHIVE_RPC_ORIGIN',''))
    lut.require(rpc.provider==qualified['scheme_host'],'qualified archive required')
    client=t.EvidenceClient(root,rpc,lut.read(t.POLICY_PATH),qualified)
    results=[]
    for name in ('T2','T3','T4'):
        row,binaries=row_for(name)
        result={'target':name,'signature':row['transaction']['signature'],'acquired':[],'failure':None,'runtime_executed':False}
        results.append(result)
        try:
            plan=state.state_plan(row,binaries)
            result['planned']=plan
        except ValueError as e:
            result['failure']={'reason':str(e)}
            continue
        for item in plan:
            def validate(value,item=item):
                facts=state.account_check(row,item,value)
                if value['value'] is None:lut.require(absent_allowed(row,item),'unproven historical absence')
                return facts
            _,facts,failure=client.call(item['method'],item['params'],validate,absent_allowed(row,item))
            if failure:
                result['failure']={'reason':failure,'request':item};break
            result['acquired'].append(item|{'facts':facts})
            print(json.dumps({'target':name,'boundary':item['boundary'],'acquired':len(result['acquired'])}),flush=True)
        result['raw_boundaries_complete']=result['failure'] is None
        result['runtime_inputs']=[]
        if result['failure'] is None:
            for var,address in f.runtime.REQUIREMENTS[:3]:
                item={'name':var,'method':'getAccountInfo','params':[address,{'encoding':'base64','commitment':'finalized','slot':row['transaction']['slot']}]}
                _,facts,failure=client.call(item['method'],item['params'],lambda value,i=item:runtime_check(row,i,value))
                if failure:result['failure']={'reason':failure,'request':item};break
                result['runtime_inputs'].append(item|{'facts':facts})
        (root/f'{name}.json').write_bytes(lut.canonical(result))
        print(json.dumps({'target':name,'raw_complete':result.get('raw_boundaries_complete'),'failure':result['failure']}),flush=True)
    client.finish({'targets':results})
    (root/'checksums.json').write_bytes(lut.canonical({str(p.relative_to(root)):lut.sha(p.read_bytes()) for p in root.rglob('*') if p.is_file()}))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,required=True)
    run(parser.parse_args().output.resolve())
