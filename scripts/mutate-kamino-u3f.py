#!/usr/bin/env python3
"""Real source faults, isolated additive Rust example; no compiler-only kills."""
import argparse
import base64
import copy
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import types
from unittest.mock import patch
import kamino_u3f_fidelity as fidelity
f,lut=fidelity.f,fidelity.lut
spec=importlib.util.spec_from_file_location('replay',lut.REPO/'scripts/replay-kamino-u3f.py');replay=importlib.util.module_from_spec(spec);spec.loader.exec_module(replay)


def run(output):
    lut.require(not output.exists(),'new mutation evidence required')
    payload,_=f.prepare();payload['variant']='empty'
    _,post=fidelity.references(payload)
    expected=lut.read(f.ROOT/'phase-u3f-materiality-attempt-1/empty.json')
    path=lut.REPO/'engine/examples/u3f_fault.rs'
    lut.require(not path.exists(),'mutation example must be isolated')
    source=(lut.REPO/'engine/examples/execute_envelope_v0.rs').read_text()
    anchor='    let seeded_pre_accounts:'
    message_faults={
        17:('skip Scope','if let VersionedMessage::V0(m) = &mut submitted_message { m.instructions.remove(0); }'),
        18:('seed post-Scope state','let key: Address = "3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH".parse()?; let mut a=svm.get_account(&key).unwrap(); a.data[25592]=51; svm.set_account(key,a).unwrap();'),
        19:('reorder Scope','if let VersionedMessage::V0(m) = &mut submitted_message { m.instructions.swap(0,1); }'),
        22:('reduce original native v0 envelope','if let VersionedMessage::V0(m) = &mut submitted_message { m.instructions.pop(); }'),
        23:('incorrect Instructions sysvar account','if let VersionedMessage::V0(m) = &mut submitted_message { m.instructions[0].accounts[3]=0; }'),
    }
    sources={n:(name,source.replace(anchor,'    '+fault+'\n'+anchor)) for n,(name,fault) in message_faults.items()}
    for n,name,program in [(20,'wrong Scope ELF','HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ'),(21,'wrong KLend ELF','KLend2g3cP87fffoy8q1mQqGKjrxjC8boSyAYavgmjD')]:
        insert=f'    let p: Address = "{program}".parse()?; let pd: Address = Address::new_from_array(seeds[&p].data[4..36].try_into()?); let a=seeds.get_mut(&pd).unwrap(); *a.data.last_mut().unwrap() ^= 1;\n'
        sources[n]=(name,source.replace('    for program in input["programs"]',insert+'    for program in input["programs"]'))
    sources[24]=('current Clock',source.replace('    let clock = svm.get_sysvar::<Clock>();','    let mut bad_clock=svm.get_sysvar::<Clock>(); bad_clock.slot += 100; svm.set_sysvar(&bad_clock);\n    let clock = svm.get_sysvar::<Clock>();'))
    results=[]
    try:
        for number,(name,mutant) in sorted(sources.items()):
            lut.require(mutant!=source,'mutation site missing')
            path.write_text(mutant)
            build=subprocess.run(['cargo','build','--offline','-q','-p','eplyx-engine','--example','u3f_fault'],cwd=lut.REPO,capture_output=True)
            lut.require(build.returncode==0,'compiler failure is not a killed mutant: '+build.stderr.decode())
            proc=subprocess.run([str(lut.REPO/'target/debug/examples/u3f_fault')],cwd=lut.REPO,input=lut.canonical(payload),capture_output=True,timeout=180)
            assertion=None
            if proc.returncode:
                message=proc.stderr.decode()
                expected_guard='historical ELF hash differs' if number in (20,21) else 'historical Clock differs' if number==24 else None
                lut.require(expected_guard and expected_guard in message,'unexpected process failure is not a kill')
                assertion=expected_guard
            else:
                observed=json.loads(proc.stdout)
                try:
                    replay.verify_pre(payload,observed)
                    lut.require(fidelity.compare(payload,observed,post)['fidelity']=='matched','strict raw/outcome historical fidelity')
                    lut.require(observed['evidence']==expected['evidence'],'exact native execution evidence')
                except ValueError as e:assertion=str(e)
            lut.require(assertion is not None,f'mutant survived: {number} {name}')
            results.append({'id':number,'fault':name,'compiled':True,'killed':True,'named_assertion':assertion,'fault_source_sha256':lut.sha(mutant.encode()),'boundary':'real native v0 execution or runtime construction guard'})
            print(json.dumps(results[-1]),flush=True)
    finally:
        path.unlink(missing_ok=True)
    original=(lut.REPO/'scripts/kamino_u3f_fidelity.py').read_text()
    faulty=copy.deepcopy(expected);a=next(a for a in faulty['evidence']['post_accounts'].values() if a and a['data'][0]);data=bytearray(base64.b64decode(a['data'][0]));data[-1]^=1;a['data'][0]=base64.b64encode(data).decode()
    for number,name,mutant in [(25,'Ok means fidelity',original.replace("    return {'kind': 'strict_native", "    failures = []\n    return {'kind': 'strict_native")),(26,'ignore raw data differences',original.replace("'rentEpoch', 'data')", "'rentEpoch')")),(27,'semantics before fidelity',original.replace("    lut.require(fidelity['fidelity'] == 'matched', 'strict historical fidelity failed; no semantics')",'    pass # faulty early promotion'))]:
        module=types.ModuleType('fidelity_mutant');exec(compile(mutant,'<fidelity-mutant>','exec'),module.__dict__)
        if number in (25,26):killed=module.compare(payload,faulty,post)['fidelity']!='mismatched';assertion='unexpected raw byte must reject even when transaction is Ok'
        else:
            with patch.object(f,'prepare',return_value=(payload,{})),patch.object(module,'compare',return_value={'fidelity':'mismatched'}),patch.object(f.s.inventory,'frozen_inputs',return_value=({'transaction':{}},None,None)),patch.object(module.subprocess,'run',side_effect=AssertionError('semantic evaluator called before strict fidelity')):
                try:module.derive();killed=False
                except AssertionError:killed=True
            assertion='semantic evaluator must not be called before strict fidelity'
        lut.require(killed,f'mutant survived: {number}')
        results.append({'id':number,'fault':name,'killed':True,'named_assertion':assertion,'fault_source_sha256':lut.sha(mutant.encode()),'boundary':'strict comparator/gating source mutation'})
    # Semantic promotion fault is exercised through a compiled existing-evaluator
    # wrapper; the test requires its exact existing economic subject set.
    source=(lut.REPO/'engine/examples/evaluate_envelope_semantics.rs').read_text()
    mutant=source.replace('"subjects":capabilities','"subjects":json!(["scope_refresh_prices"])')
    path.write_text(mutant)
    try:
        build=subprocess.run(['cargo','build','--offline','-q','-p','eplyx-engine','--example','u3f_fault'],capture_output=True,cwd=lut.REPO)
        lut.require(build.returncode==0,'compiler failure does not count')
        row,_,_=f.s.inventory.frozen_inputs();pre,_=fidelity.references(payload)
        value={'transaction':row['transaction'],'watch':payload['watch'],'absent':payload['absent'],'pre':pre,'post':expected['evidence']['post_accounts'],'expected_post':post,'outcome':expected['evidence']}
        proc=subprocess.run([str(lut.REPO/'target/debug/examples/u3f_fault')],input=lut.canonical(value),capture_output=True,cwd=lut.REPO)
        lut.require(proc.returncode==0,'semantic mutant must execute')
        lut.require(json.loads(proc.stdout)['subjects']!=lut.read(f.VALIDATION/'T1-semantics.json')['subjects'],'semantic mutant survived')
        results.append({'id':28,'fault':'promote Scope semantic support','compiled':True,'killed':True,'named_assertion':'exact unchanged U2 economic subject set; no Scope subject','fault_source_sha256':lut.sha(mutant.encode()),'boundary':'compiled semantic evaluation output'})
    finally:path.unlink(missing_ok=True)
    output.write_bytes(lut.canonical({'runtime_source_mutants':results,'killed':len(results),'compiler_only_kills':0,'original_sources_unchanged':True}))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,required=True);run(parser.parse_args().output)
