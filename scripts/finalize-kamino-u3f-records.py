#!/usr/bin/env python3
"""Seal additive experimental manifests after measured fidelity, not product admission."""
from pathlib import Path
import kamino_u3f_cohort as c
f,lut=c.f,c.lut


def hashes(paths):
    files=set()
    for p in paths:
        if p.is_dir():files.update(x for x in p.rglob('*') if x.is_file() and '__pycache__' not in str(x))
        elif p.is_file():files.add(p)
    return {str(p.relative_to(lut.REPO)):lut.sha(p.read_bytes()) for p in sorted(files)}


def main():
    f.preserved()
    root=f.ROOT/'phase-u3f-record'
    common=[f.VALIDATION/'preimplementation.json',lut.REPO/'interface/Cargo.toml',lut.REPO/'Cargo.lock',lut.REPO/'Cargo.toml',lut.REPO/'engine/Cargo.toml',lut.REPO/'engine/src',lut.REPO/'interface/src',f.ROOT/'phase-u3f-interface']
    common += list((lut.REPO/'scripts').glob('*u3*.py'))+[lut.REPO/'scripts/historical_transport.py']
    common += [lut.REPO/'engine/examples'/n for n in ('execute_envelope_v0.rs','evaluate_envelope_semantics.rs','verify_envelope_state.rs','acquire_envelope_dependencies.rs')]
    common += [lut.SAMPLE] + list(f.ROOT.glob('phase-u3d*')) + list(f.ROOT.glob('phase-u3e*')) + list(f.ROOT.glob('phase-u3a*'))+[f.ROOT/'phase-u3b2-lut',f.ROOT/'phase-u3c-envelope',f.ROOT/'phase-u3c-dependencies',f.ROOT/'phase-u3f-validation/lossless-storage.json']
    records=[]
    for name in ('T1','T2'):
        semantics=lut.read(f.VALIDATION/f'{name}-semantics.json')
        if name=='T1':
            record=lut.read(root/'record.json')
            extra=[f.ROOT/n for n in ('phase-u3e-state','phase-u3e-runtime','phase-u3e-boundary-proof','phase-u3f-materiality-attempt-1','phase-u3f-causal-attempt-1')]
            extra += [f.VALIDATION/'T1-fidelity.json',f.VALIDATION/'T1-semantics.json']
            evidence=f.read_capture(f.ROOT/'phase-u3f-materiality-attempt-1/empty.json')['evidence']
            filename='record.json'
        else:
            row,binaries=c.s.row_for(name)
            record={'schema':'eplyx.experimental.native-v0-envelope.v1','signature':row['transaction']['signature'],'slot':row['transaction']['slot'],'target_outer_index':7,'message_proof_id':row['envelope']['message_proof_id'],'source_sample_fingerprint':lut.FINGERPRINT,'seed_manifest':lut.read(f.ROOT/'phase-u3f-T2-attempt-1/input-manifest.json'),'runtime_context':{'historical':['Clock','Rent','EpochSchedule'],'SlotHashes':'default/empty/different valid profiles; not claimed historical','Instructions':'runtime-generated from complete native message'},'ordinary_ReplayRecord':False,'Scope_semantic_support_added':False,'semantic_assurance':'experimental production-derived observation after complete baseline fidelity; ordinary ReplayRecord admission unchanged'}
            extra=[f.ROOT/n for n in ('phase-u3d2-binaries','phase-u3f-binary-recovery','phase-u3f-cohort-binaries','phase-u3f-cohort-state','phase-u3f-T2-attempt-1','phase-u3f-T2-causal-attempt-1')]
            extra += [f.VALIDATION/'T2-semantics.json']
            evidence=f.read_capture(f.ROOT/'phase-u3f-T2-attempt-1/empty.json')['evidence'];filename='T2.json'
        record.update(evidence=hashes(common+extra),fidelity='matched',replay_decision=f'{name}-A',semantic_subjects=[s['subject'] for s in semantics['subjects'] if s['domain']=='economic'],pre_state_hash=semantics['pre_state_hash'],post_state_hash=semantics['post_state_hash'],execution_evidence_sha256=lut.sha(lut.canonical(evidence)),baseline_self_findings=semantics['baseline_self_findings'],offline_replay_command='python3 scripts/replay-kamino-u3f-corpus.py --output /tmp/eplyx-u3f-offline.json',legacy_boundary_prover=semantics['legacy_boundary_prover'])
        (root/filename).write_bytes(lut.canonical(record));records.append({'target':name,'record':filename,'sha256':lut.sha((root/filename).read_bytes()),'signature':record['signature'],'slot':record['slot'],'target_outer_index':record['target_outer_index']})
    rows=[f.s.inventory.frozen_inputs()[0],c.s.row_for('T2')[0]]
    targets=[r['transaction']['instructions'][r['envelope']['targets'][0]['outer_index']]['accounts'] for r in rows]
    corpus={'kind':'experimental_production_derived_Kamino_corpus','records':records,'record_count':2,'deposit':1,'borrow':1,'unique_target_reserves':len({a[4]['address'] for a in targets}),'unique_obligations':len({a[1]['address'] for a in targets}),'ordered_envelope_shapes':2,'shape_definition':'Scope/ATA/2 or 4 reserve-refreshes/obligation/deposit or borrow/compute-limit/price','economic_subjects':7,'baseline_self_findings':[],'ordinary_product_admission':False,'representative_of_Kamino_traffic':False,'candidate_differential':'not attempted; no reproducibly buildable historical-compatible controlled candidate established'}
    (root/'corpus.json').write_bytes(lut.canonical(corpus))
    print(lut.canonical(corpus).decode())


if __name__=='__main__':main()
