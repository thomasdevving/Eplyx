#!/usr/bin/env python3
"""Reexecute both experimental mainnet-derived records, entirely offline."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import time
from unittest.mock import patch
import kamino_u3f_cohort as c
f,lut=c.f,c.lut
spec=importlib.util.spec_from_file_location('u3f_replay',Path(__file__).with_name('replay-kamino-u3f.py'))
r=importlib.util.module_from_spec(spec);spec.loader.exec_module(r)


def evaluate_t2(payload,execution,post):
    proof=c.b.fidelity.compare(payload,execution,post)
    lut.require(proof['fidelity']=='matched','semantic promotion requires strict fidelity')
    row,_=c.s.row_for('T2')
    pre={a['address']:a['account'] for a in payload['seeds'] if a['address'] in payload['watch']};pre.update({a:None for a in payload['absent']})
    value={'transaction':row['transaction'],'target_outer_index':7,'watch':payload['watch'],'absent':payload['absent'],'pre':pre,'post':execution['evidence']['post_accounts'],'expected_post':post,'outcome':execution['evidence']}
    process=subprocess.run([str(lut.REPO/'target/debug/examples/evaluate_envelope_semantics')],cwd=lut.REPO,input=lut.canonical(value),capture_output=True,timeout=60)
    lut.require(process.returncode==0,process.stderr.decode(errors='replace'))
    return json.loads(process.stdout)


def main(output,verify_record=True):
    started=time.perf_counter()
    first=r.run(None,verify_record)
    if verify_record:
        record=lut.read(f.ROOT/'phase-u3f-record/T2.json')
        for name,digest in record['evidence'].items():lut.require(lut.sha((lut.REPO/name).read_bytes())==digest,f'record evidence changed: {name}')
    env={k:v for k,v in os.environ.items() if not any(x in k.upper() for x in ('RPC','ARCHIVE','API_KEY','ALCHEMY','HELIUS'))}
    with patch.object(c.t.CurlArchive,'once',side_effect=AssertionError('offline transport forbidden')):
        payload,post,_=c.prepare('T2')
        expected=f.read_capture(f.ROOT/'phase-u3f-T2-attempt-1/empty.json')
        results=[]
        for variant in ('different','empty','default'):
            payload['variant']=variant
            process=subprocess.run([str(lut.REPO/'target/debug/examples/execute_envelope_v0')],cwd=lut.REPO,env=env,input=lut.canonical(payload),capture_output=True,timeout=180)
            lut.require(process.returncode==0,process.stderr.decode(errors='replace'))
            execution=json.loads(process.stdout);r.verify_pre(payload,execution)
            lut.require(execution['evidence']==expected['evidence'],'T2 offline execution evidence differs')
            tick=time.perf_counter();proof=c.b.fidelity.compare(payload,execution,post);comparison_seconds=time.perf_counter()-tick
            lut.require(proof['fidelity']=='matched','T2 raw fidelity differs')
            tick=time.perf_counter();semantics=evaluate_t2(payload,execution,post);semantic_seconds=time.perf_counter()-tick
            lut.require(semantics==lut.read(f.VALIDATION/'T2-semantics.json'),'T2 existing semantics differ')
            results.append({'variant':variant,'fidelity':'matched','pre_state_exact':True,'baseline_self_findings':semantics['baseline_self_findings'],'evidence_sha256':lut.sha(lut.canonical(execution['evidence'])),'preparation_seconds':execution['preparation_seconds'],'execution_seconds':execution['execution_seconds'],'comparison_seconds':comparison_seconds,'semantic_seconds_including_wrapper':semantic_seconds})
    report={'records':2,'offline_reexecution_passed':True,'record_hashes_verified':verify_record,'T1':first,'T2':results,'elapsed_seconds':time.perf_counter()-started}
    lut.require(not output.exists(),'new offline verification report required');output.write_bytes(lut.canonical(report));print(json.dumps({'offline_corpus_records':2,'matched':2}))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,required=True);main(parser.parse_args().output)
