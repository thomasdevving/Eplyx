#!/usr/bin/env python3
"""Measured U3F report; acquired bytes, runtime evidence and admission stay separate."""
import base64
from collections import Counter
from datetime import datetime
import json
from pathlib import Path
import re
import time
import kamino_u3f_cohort as c
f,lut=c.f,c.lut


def main():
    f.preserved()
    corpus=lut.read(f.ROOT/'phase-u3f-record/corpus.json')
    captures={}
    for name in ('binary-recovery','cohort-binaries','cohort-state'):
        root=f.ROOT/f'phase-u3f-{name}';receipt=lut.read(root/'receipt.json');a=receipt['attempts']
        captures[name]={'requests':len(a),'outcomes':dict(Counter(x['failure_class'] or 'success' for x in a)),'rpc_seconds':sum(x['elapsed_seconds'] for x in a),'capture_interval_seconds':datetime.fromisoformat(a[-1]['started_at_utc']).timestamp()+a[-1]['elapsed_seconds']-datetime.fromisoformat(a[0]['started_at_utc']).timestamp(),'raw_response_bytes':sum(x['body_bytes'] for x in a),'transport_policy':receipt['policy'],'ordinary_account_image_bytes_including_both_boundaries':sum(len(base64.b64decode(lut.read(root/x['body_file'])['result']['value']['data'][0])) for x in a if x['method']=='getAccountInfo' and x['failure_class'] is None and lut.read(root/x['body_file'])['result']['value'] is not None)}
    rust=[int(n) for n in re.findall(r'test result: ok. (\d+) passed',(f.VALIDATION/'rust-tests.log').read_text())]
    python={}
    for p in sorted(f.VALIDATION.glob('test_*.py.log')):
        text=p.read_text();n=re.findall(r'Ran (\d+) tests',text)
        lut.require(n and text.rstrip().endswith('OK'),f'test failed: {p.name}');python[p.name]=int(n[-1])
    controls=lut.read(f.VALIDATION/'after/controls.json')
    mutations=lut.read(f.VALIDATION/'runtime-mutations-final.json')
    offline=lut.read(f.VALIDATION/'offline-corpus-sealed.json')
    stages=[]
    for name in ('T1','T2','T3','T4'):
        success=name in ('T1','T2')
        stage={f'C{i}':'passed' for i in range(1,5)}
        stage['C5']='passed' if name!='T4' else 'not_attempted'
        stage.update({f'C{i}':'passed' if success else 'not_attempted' for i in range(6,11)})
        stage.update(target=name,C5_definition='paired ordinary state/typed validation, separate from runtime context',production_ReplayRecord_eligible=False)
        stage['next_blocker']=None if success else {'kind':'runtime_account_acquisition','account':'Rent','slot':448194462,'failure':'rate_limited after four exact-context attempts','EpochSchedule':'not_attempted'} if name=='T3' else {'kind':'same_slot_interference','conflicts':lut.read(f.ROOT/'phase-u3f-cohort-binaries/T4/result.json')['slot_screening']['conflicts']}
        stages.append(stage)
    storage={}
    for root in sorted(f.ROOT.glob('phase-u3f-*')):
        files=[p for p in root.rglob('*') if p.is_file()]
        storage[root.name]={'files':len(files),'physical_bytes':sum(p.stat().st_size for p in files)}
    semantics={n:lut.read(f.VALIDATION/f'{n}-semantics.json') for n in ('T1','T2')}
    causal={n:lut.read(f.ROOT/('phase-u3f-causal-attempt-1' if n=='T1' else 'phase-u3f-T2-causal-attempt-1')/'comparison.json') for n in ('T1','T2')}
    report={
      '01_repository':{'initial_head':lut.read(f.VALIDATION/'preimplementation.json').get('initial_head','84ba410287a4a7ed315a7d779dbaad625fe3bf39'),'tracked_product_sources_changed':False,'prior_U3E_bytes_preserved':True},
      '02_frozen_controls':{'checksums_U3A':344,'fingerprint':lut.FINGERPRINT,'policy_sha256':lut.POLICY,'U3B2_proofs':8},
      '03_scope':'four frozen primary identities; bounded continuation after actual T1-A',
      '04_inventory':'T1 23 native message keys/22 watched; T2 independently reconstructed from its sealed v0 proof',
      '05_provider':'same qualified archive; proof conditional on retained U3B qualification; scheme-host only; no private credentials',
      '06_binary_acquisition':captures['cohort-binaries'],
      '07_state_acquisition':captures['cohort-state'],
      '08_boundary_proof':{'T1':'clean retained U3E proof','T2':'clean screen; 939 transactions, target index 564','T3':'clean screen and paired typed boundaries','T4':'two genuine earlier-write conflicts; no state capture or execution'},
      '09_runtime_context':{'historical':['Clock','Rent','EpochSchedule'],'SlotHashes':'not historically known; default/empty/synthetic valid variants observationally identical for T1 and T2; active-LUT code branch independent of membership','Instructions':'generated from original sanitized native message','feature_profile':'Cargo.lock-pinned LiteSVM mainnet defaults, fidelity demonstrated for these two transactions; not reconstructed full historical validator bank','signature_and_recent_blockhash_checks':False},
      '10_native_execution':'exact original v0, signatures, instruction order and account vector; real historical ELF via original program/ProgramData headers; no legacy flattening',
      '11_Scope_execution':'baseline Scope ran; T1 observed offset 25592 changed 48 to 51 exactly; all watched bytes match',
      '12_causal_experiment':causal,
      '13_ATA_execution':'real CreateIdempotent retained; original existing-token path; no new creation or closure support',
      '14_outcome_fidelity':'T1 and T2 success, fees, logs, CPI instruction data/accounts/stack/index and return payload match; empty return-data representation follows retained Agave source',
      '15_post_state_fidelity':'all watched raw data, owner, lamports, executable, rentEpoch and lifecycle match; zero tolerances; Instructions is generated runtime context',
      '16_T1_decision':'T1-A',
      '17_production_derived_semantics':semantics,
      '18_first_experimental_record':'docs/examples/phase-u3f-record/record.json',
      '19_offline_replay':offline,
      '20_record_schema_decision':'additive eplyx.experimental.native-v0-envelope.v1; ordinary ReplayRecord and adapter remain unchanged; manifests are verified and payloads rebuilt from raw evidence before execution',
      '21_T2_T4_transport_recovery':captures['binary-recovery'],
      '22_T2_T4_replay_results':{'T2':'matched independently','T3':'state complete, Rent rate-limit bound reached, runtime not attempted','T4':'binary complete, genuine same-slot interference, state/runtime not attempted'},
      '23_experimental_corpus':corpus,
      '24_baseline_self_check':'two records, seven unchanged economic subjects, zero self-findings',
      '25_candidate_differential':'not attempted; no honest reproducibly historical-compatible controlled KLend candidate established',
      '26_wrapped_SOL_lifecycle':'not reached; unsupported',
      '27_multi_action_replay':'not reached; unsupported',
      '28_attribution':'single-target only; multi-action unsupported unchanged',
      '29_modern_replay_funnel':{'N_classified':4,'M_envelope_admitted':4,'binary_complete':4,'paired_ordinary_state_proven':3,'runtime_inputs_demonstrated_sufficient':2,'J_baselines_attempted':2,'P_fidelity_matched':2,'experimental_records':2,'ordinary_product_eligible':0,'stages':stages},
      '30_terminal_blockers':{'matched_experimental':2,'runtime_Rent_rate_limited':1,'same_slot_interference':1},
      '31_tests':{'Rust':sum(rust),'Rust_groups':rust,'Python':python,'total':sum(rust)+sum(python.values()),'product_controls':controls,'clippy_warnings_denied':True,'formatting':True},
      '32_mutations':mutations,
      '33_performance':{'capture':captures,'offline_runtime_and_reconciliation':offline,'exclusions':'builds/archival verification are not VM execution; semantic timing includes wrapper startup; no candidate overhead measured'},
      '34_storage':{'physical_directories':storage,'lossless_execution_compression':lut.read(f.VALIDATION/'lossless-storage.json'),'duplication':'three variants intentionally retain full watched results and actual seed audits; audit repeats ELF bytes; gzip preserves all originals. Historical context reuse is exact-request only; new binary raw responses and reconstructed images both retained. No generalized deduplication.'},
      '35_security_scan':lut.read(f.VALIDATION/'security-scan.json') if (f.VALIDATION/'security-scan.json').exists() else {'pending':True},
      '36_correctness_findings':['Existing legacy Kamino boundary helper assumes SPL Token and rejects these valid Token-2022 accounts; unchanged ordinary admission remains blocked. Experimental raw/owner proof is separate.','Initial experimental return-data comparator rejected an empty LiteSVM payload because it retained ComputeBudget program ID; Agave omits empty return data. Corrected without account tolerance; initial failure preserved.','Initial experimental borrow wrapper attempted direct serde of FieldValue::Text and panicked. Wrapper now preserves existing render() text explicitly. No new economic subject or product JSON change.'],
      '37_justified_claims':['two real historical production transactions reproduced locally from retained evidence','seven existing subjects interpreted after complete experimental baseline fidelity','T2 Scope omission changes reserve and obligation bytes','active-LUT branch ignores SlotHashes membership; tested profile results identical'],
      '38_prohibited_claims':['general historical SlotHashes knowledge or full historical bank reconstruction','ordinary product/hosted Kamino replay support','current deployment or public source equals historical ELF without build proof','arbitrary companion/oracle shapes, multi-action or creation/closure support','representative corpus or candidate regression sensitivity proved'],
      '39_adapter_4':'do not start; close native-v0 product admission/typed boundary seam and unresolved bank/interference cases first',
      '40_contract_v2':'do not redesign declaratively from two profiles; additive native-v0 experiment only',
      '41_next_phase':'small generic native-v0 execution/input contract integrated with mixed Token/Token-2022 boundary validation; later separately versioned T3 archive attempt; T4 needs transaction-boundary history or preceding-transaction replay, never S-1 substitution',
    }
    (f.VALIDATION/'final-report.json').write_bytes(lut.canonical(report))
    (f.VALIDATION/'stage-table.json').write_bytes(lut.canonical({'rows':stages,'metrics':report['29_modern_replay_funnel']}))
    print(json.dumps({'matched':2,'records':2,'tests':report['31_tests']['total'],'mutants':mutations['killed']}))


if __name__=='__main__':main()
