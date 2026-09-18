#!/usr/bin/env python3
"""Additive offline U3C envelope analysis; original U3A/U3B bytes are immutable."""
import argparse,base64,json,os,subprocess
from pathlib import Path
import kamino_u3b_lut as lut
REPO=lut.REPO
OUTPUT=REPO/'docs/examples/phase-u3c-envelope'
U3B=REPO/'docs/examples/phase-u3b2-lut'
SCOPE='HFn8GnPADiny6XqUoWE8uRPPxb29ikn4yTuPa9MF2fWJ'

def structure_analysis(records):
    analysis={'source':'exact frozen U3A captures and U3B.2 proof artifacts','multi_action':[],'ata_metadata':[],'legacy_system':None}
    for record in sorted(records,key=lambda r:r['envelope']['signature']):
        tx=record['transaction'];observations=record['envelope']['targets']
        raw=lut.read(lut.SAMPLE/'transactions'/f"{tx['signature']}.json")['result']
        if len(observations)>1:
            targets=[]
            for observation in observations:
                ix=tx['instructions'][observation['outer_index']]
                targets.append({**observation,'obligation':ix['accounts'][1]['address'],'market':ix['accounts'][2]['address'],'reserve':ix['accounts'][4]['address'],'compiled_target_accounts':ix['accounts'],'instruction_data':ix['data'],'inner_group':next((g for g in raw['meta']['innerInstructions']if g['index']==observation['outer_index']),None)})
            analysis['multi_action'].append({'signature':tx['signature'],'slot':tx['slot'],'on_chain_success':record['on_chain_success'],'observations':targets,'shared_obligation':targets[0]['obligation']==targets[1]['obligation'],'shared_reserve':targets[0]['reserve']==targets[1]['reserve'],'semantic_attribution':'unsupported; existing evaluator uses whole-transaction token balances; instruction-local transfers alone do not establish independent reserve/obligation/rounding deltas','runtime_execution_attempted':False})
        for index,ix in enumerate(tx['instructions']):
            if ix['program'].startswith('AToken'):
                address=ix['accounts'][1]['address'];key=next(i for i,a in enumerate(tx['account_keys'])if a['address']==address)
                analysis['ata_metadata'].append({'signature':tx['signature'],'outer_index':index,'account':address,'mint':ix['accounts'][3]['address'],'token_program':ix['accounts'][5]['address'],'data':ix['data'],'pre_lamports':tx['pre_balances'][key],'post_lamports':tx['post_balances'][key],'pre_token_balance':next((v for v in raw['meta']['preTokenBalances']if v['accountIndex']==key),None),'post_token_balance':next((v for v in raw['meta']['postTokenBalances']if v['accountIndex']==key),None),'inner_group':next((g for g in raw['meta']['innerInstructions']if g['index']==index),None),'interpretation':'existing-token/no-creation metadata witness; historical account bytes and local execution not established','primary_group':SCOPE in record['before_rejection']})
    legacy=next(r for r in lut.read(lut.SAMPLE/'before-rejections.json')if r['message_version']=='legacy')
    raw=lut.read(lut.SAMPLE/'transactions'/f"{legacy['signature']}.json")['result']
    timeline=legacy['old_engine']['normalized_top_level_instructions']
    analysis['legacy_system']={'signature':legacy['signature'],'slot':legacy['slot'],'excluded_from_eight_LUT_targets':True,'first_rejection':legacy['old_engine']['first_blocker'],'timeline':timeline,'outer_0_system_transfer_lamports':int.from_bytes(bytes.fromhex(timeline[0]['data'])[4:],'little'),'wrapped_sol_account':timeline[1]['accounts'][1]['address'],'wrapped_sol_pre_lamports':raw['meta']['preBalances'][7],'wrapped_sol_post_lamports':raw['meta']['postBalances'][7],'subsequent_token_instructions':{'outer_2':'SyncNative','outer_8':'CloseAccount'},'runtime_execution_attempted':False,'support':'not implemented; lifecycle distinct from primary existing-ATA path'}
    return analysis

def frozen_inputs():
    control=lut.read(REPO/'docs/examples/phase-u3c-validation/preimplementation.json')
    for name,expected in control['frozen_artifact_hashes'].items():
        lut.require(lut.sha((REPO/name).read_bytes())==expected,'frozen U3A/U3B input changed; stop')
    targets=lut.frozen_targets();capture=lut.validate_capture(U3B,targets)
    membership={(r['table_pubkey'],r['execution_slot']):r for r in capture['requests']}
    inputs=[]
    for t in targets:
        entries=[membership[l['accountKey'],t['execution_slot']]for l in t['address_table_lookups']]
        inputs.append({'result':lut.read(lut.SAMPLE/t['capture_file'])['result'],'genesis':lut.GENESIS,'evidence':[{'pubkey':r['table_pubkey'],'provider':capture['provider'],'raw_response_base64':base64.b64encode((U3B/r['response_file']).read_bytes()).decode()}for r in entries]})
    before=lut.read(U3B/'stage-table.json')
    reasons=[r['next_old_engine_rejection']for r in before['rows']]
    distribution={'scope_companion':sum(SCOPE in r for r in reasons),'ata_companion':sum('AToken' in r for r in reasons),'multiple_actions':sum('one KLend action' in r for r in reasons),'failed_original':sum('successfully captured' in r for r in reasons)}
    lut.require(distribution=={'scope_companion':4,'ata_companion':1,'multiple_actions':2,'failed_original':1},'measured S8 distribution changed')
    return targets,inputs,distribution

def derive(reverse=False):
    targets,inputs,distribution=frozen_inputs()
    env={k:v for k,v in os.environ.items()if not any(x in k for x in ('RPC','API_KEY','ARCHIVE','ALCHEMY','HELIUS'))}
    if reverse:inputs.reverse()
    process=subprocess.run([str(REPO/'target/debug/examples/audit_kamino_envelope')],input=lut.canonical(inputs),capture_output=True,env=env,check=True)
    rows=json.loads(process.stdout);by_signature={r['signature']:r for r in rows}
    outputs={};primary=[];records=[]
    for t in targets:
        r=by_signature[t['signature']]
        original=lut.read(U3B/f"proofs/{t['signature']}.json")
        lut.require(r['proof']==original['proof'],'U3B proof changed in U3C')
        result={'sample_fingerprint':lut.FINGERPRINT,'frozen_transaction_sha256':t['capture_sha256'],'frozen_lut_proof_id':r['proof']['proof_id'],'on_chain_success':t['on_chain_success'],'envelope':r['envelope'],'transaction':r['normalized_transaction'],'before_rejection':original['next_old_engine_rejection'],'runtime_executed':False,'semantic_findings_emitted':False}
        outputs[f"transactions/{t['signature']}.json"]=lut.canonical(result)
        records.append(result)
        if SCOPE in original['next_old_engine_rejection']:primary.append(result)
    summary={'frozen_fingerprint':lut.FINGERPRINT,'U3B_proofs_identical':8,'S8_distribution':distribution,'primary_targets':4,'structurally_classified':sum(p['envelope']['structurally_classified']for p in primary),'envelope_admissible':sum(p['envelope']['envelope_admissible']for p in primary),'primary_signatures':[p['envelope']['signature']for p in primary],'production_replay_eligible':0,'runtime_execution_attempted':0,'baseline_fidelity_matches':0}
    outputs['summary.json']=lut.canonical(summary)
    outputs['structure-analysis.json']=lut.canonical(structure_analysis(records))
    outputs['checksums.sha256']=''.join(f'{lut.sha(body)}  {name}\n'for name,body in sorted(outputs.items())).encode()
    return outputs

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,default=OUTPUT);parser.add_argument('--verify',action='store_true');parser.add_argument('--reverse',action='store_true');args=parser.parse_args()
    lut.require(not args.output.resolve().is_relative_to(lut.SAMPLE.resolve()) and not args.output.resolve().is_relative_to(U3B.resolve()),'cannot write frozen evidence')
    outputs=derive(args.reverse)
    for name,body in outputs.items():
        path=lut.safe_file(args.output,name)
        if args.verify:lut.require(path.read_bytes()==body,'U3C canonical derived bytes differ')
        else:path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(body)
    print(outputs['summary.json'].decode())
if __name__=='__main__':main()
