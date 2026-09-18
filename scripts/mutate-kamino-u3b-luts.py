#!/usr/bin/env python3
"""Twelve real Rust behavioral faults, isolated source copies, assertion kills.

Uses a separate compiled cache and never edits production source. Every named
control test must pass first; compiler/transport/runtime failures are not kills.
"""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import kamino_u3b_lut as lut

MUTATIONS = [
    (1, 'loaded writable/readonly vectors swapped', 'independent_resolution_preserves_writable_readonly_order', [('loaded.writable.extend_from_slice(&writable);\n        loaded.readonly.extend_from_slice(&readonly);','loaded.writable.extend_from_slice(&readonly);\n        loaded.readonly.extend_from_slice(&writable);')]),
    (2, 'loaded addresses sorted', 'independent_resolution_preserves_writable_readonly_order', [('    stage(\n        4,','    loaded.writable.sort();\n    loaded.readonly.sort();\n    stage(\n        4,')]),
    (3, 'second lookup table ignored', 'independent_resolution_preserves_writable_readonly_order', [('for lookup in &frozen.message.address_table_lookups {','for lookup in frozen.message.address_table_lookups.iter().take(1) {')]),
    (4, 'current state queried instead of execution-slot state', 'acquisition_never_substitutes_current_state_for_historical', [('{"encoding":"base64", "commitment":"finalized", "slot":execution_slot}','{"encoding":"base64", "commitment":"finalized"}')]),
    (5, 'wrong table owner accepted', 'wrong_owner_is_rejected_before_address_comparison', [('account.owner == program::id().to_string()','true')]),
    (6, 'same-slot extension warmup ignored', 'same_slot_extension_only_exposes_old_prefix', [('.lookup(frozen.slot, &writable_indexes, &hashes)','.lookup(frozen.slot + 1, &writable_indexes, &hashes)'),('.lookup(frozen.slot, &readonly_indexes, &hashes)','.lookup(frozen.slot + 1, &readonly_indexes, &hashes)')]),
    (7, 'out-of-range writable index clamped', 'writable_and_readonly_out_of_range_indexes_reject', [('let writable_indexes = lookup.writable_indexes.clone();','let writable_indexes = lookup.writable_indexes.iter().map(|i| (*i).min(table.addresses.len().saturating_sub(1) as u8)).collect::<Vec<_>>();')]),
    (8, 'RPC loaded addresses trusted without historical proof', 'rpc_loaded_metadata_is_never_a_table_proof', [('by_key.len() == wanted.len() && by_key.keys().all(|k| wanted.contains(*k))','true'),('for lookup in &frozen.message.address_table_lookups {','for lookup in frozen.message.address_table_lookups.iter().filter(|l| by_key.contains_key(l.account_key.to_string().as_str())) {'),('    stage(\n        4,','    if evidence.is_empty() { loaded = frozen.rpc_loaded.clone(); }\n    stage(\n        4,')]),
    (9, 'loaded writable key promoted to signer', 'independent_resolution_preserves_writable_readonly_order', [('let is_signer = official_loaded.is_signer(i);','let is_signer = official_loaded.is_signer(i) || (i >= frozen.message.account_keys.len() && i < frozen.message.account_keys.len() + loaded.writable.len());')]),
    (10, 'execution slot off by one in evidence validation', 'independent_resolution_preserves_writable_readonly_order', [('item.account(frozen.slot, &frozen.genesis)','item.account(frozen.slot + 1, &frozen.genesis)')]),
    (11, 'table pubkey removed from evidence identity', 'table_identity_binds_pubkey_bytes_slot_and_genesis', [('            &self.pubkey,\n','')]),
    (12, 'compiled instruction account indexes resolved against static keys only', 'independent_resolution_preserves_writable_readonly_order', [('full.get(usize::from(*i))','full[..frozen.message.account_keys.len()].get(usize::from(*i))')]),
]

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--production',action='store_true',help='also kill faults using acquired production evidence')
    args=parser.parse_args()
    production=lut.REPO/'engine/src/message.rs';original=production.read_bytes();source=original.decode()
    env={k:v for k,v in os.environ.items() if not any(s in k for s in ('RPC','API_KEY','ARCHIVE','ALCHEMY','HELIUS'))}
    env['CARGO_TARGET_DIR']=str(lut.REPO/'target/u3b-lut-mutants');env['CARGO_NET_OFFLINE']='true'
    results=[]
    with tempfile.TemporaryDirectory(prefix='eplyx-u3b-mutants-') as tmp:
        root=Path(tmp)
        for name in ('Cargo.toml','Cargo.lock','rust-toolchain.toml'):
            shutil.copyfile(lut.REPO/name,root/name)
        for name in ('engine','interface','server'):
            shutil.copytree(lut.REPO/name,root/name,ignore=shutil.ignore_patterns('target','node_modules'))
        # Fixture bytes remain the real immutable inputs; no writes into them.
        for name in ('fixtures','docs'):
            (root/name).symlink_to(lut.REPO/name,target_is_directory=True)
        copied=root/'engine/src/message.rs'
        def test(name):
            return subprocess.run(['cargo','test','--offline','-q','-p','eplyx-engine','--test','historical_lut',name,'--','--exact'],cwd=root,env=env,capture_output=True)
        mutations=list(MUTATIONS)
        if args.production:
            for original_number,test_name in [(1,'real_five_table_55_loaded_historical_proof'),(2,'real_five_table_55_loaded_historical_proof'),(3,'real_five_table_historical_proof'),(6,'real_table_bytes_in_explicit_synthetic_warmup_context'),(8,'real_historical_bytes_cannot_be_replaced_by_rpc_metadata'),(10,'real_single_table_historical_proof'),(12,'real_five_table_55_loaded_historical_proof')]:
                fault=MUTATIONS[original_number-1]
                mutations.append((len(mutations)+1,'production evidence: '+fault[1],test_name,fault[3]))
            mutations.append((len(mutations)+1,'wrong historical bytes bypass RPC equality','real_wrong_historical_bytes_are_rejected',[('loaded == frozen.rpc_loaded','true')]))
        for number,description,name,replacements in mutations:
            copied.write_bytes(original)
            control=test(name)
            if control.returncode:
                raise ValueError(f'control test failed: {name}\n{control.stdout.decode()}\n{control.stderr.decode()}')
            changed=source
            for old,new in replacements:
                if changed.count(old)!=1:
                    raise ValueError(f'M{number} target count {changed.count(old)}: {old}')
                changed=changed.replace(old,new)
            copied.write_text(changed)
            try:
                process=test(name);output=(process.stdout+process.stderr).decode()
                # Rust test assertion panic required, no compilation-only failures.
                assertion=process.returncode!=0 and 'test result: FAILED.' in output and 'panicked at' in output and 'assertion' in output and 'error[E' not in output and 'could not compile' not in output
                result={'mutation':number,'description':description,'named_test':name,'control_passed':True,'evidence_boundary':'real table bytes with explicitly synthetic visibility context'if name=='real_table_bytes_in_explicit_synthetic_warmup_context'else 'acquired frozen production evidence'if number>12 else 'original synthetic controls','result':'killed_by_assertion'if assertion else 'FAILED','test_exit':process.returncode,'mutated_source_sha256':lut.sha(changed.encode())}
                results.append(result);print(f'M{number:02}: {result["result"]} — {name}',flush=True)
                if not assertion:
                    raise ValueError(f'M{number} survived or failed outside assertion:\n{output}')
            finally:
                copied.write_bytes(original)
                if copied.read_bytes()!=original:
                    raise ValueError('isolated source not restored')
    if production.read_bytes()!=original:
        raise ValueError('production source changed')
    report={'mutations_executed':len(results),'killed_by_named_assertions':len(results),'production_source_sha256_before':lut.sha(original),'production_source_sha256_after':lut.sha(production.read_bytes()),'all_isolated_sources_restored':True,'results':results}
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_bytes(lut.canonical(report))

if __name__=='__main__':
    main()
