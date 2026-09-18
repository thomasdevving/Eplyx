#!/usr/bin/env python3
"""Isolated behavioral faults; named assertion failures only, no compiler kills."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import kamino_u3b_lut as lut

POLICY = 'engine/src/protocol/kamino/envelope.rs'
CORE = 'engine/src/envelope.rs'
FAULTS = [
    ('arbitrary external program admitted as Scope', 'arbitrary_external_program_cannot_replace_scope', [(POLICY, '_ => Err(anyhow::anyhow!(\n                "unknown external companion; no program wildcard"\n            )),', '_ => { row.identity="refreshPriceList".into(); row.role=InstructionRole::ExecutionDependency; Ok(()) },')]),
    ('Scope dependency order ignored', 'scope_after_reserve_refresh_is_rejected', [(POLICY, 'consumers.iter().any(|i| *i <= index)', 'false'), (POLICY, 'tx.instructions[..index]\n                .iter()\n                .any(|ix| ix.program != COMPUTE_BUDGET_PROGRAM_ID)', 'false'), (POLICY, 'if !exact_profile {', 'if !exact_profile && false {')]),
    ('Scope stripped from execution plan', 'complete_sequence_cannot_strip_or_reorder_dependency', [(CORE, 'instructions == self.message.transaction().instructions,', 'true,')]),
    ('current or intermediate dependency state accepted', 'current_or_intermediate_scope_state_is_rejected', [(CORE, 'source.context_slot==self.analysis.execution_slot-1', 'true')]),
    ('current external binary accepted', 'current_external_binary_is_rejected', [(CORE, 'entry.observed_slot == Some(pre_slot),', 'true,')]),
    ('execution dependency given Scope semantic coverage', 'dependency_has_no_scope_semantic_support', [(POLICY, 'SCOPE_ID => scope_tokens(ix).map(|_| {', 'SCOPE_ID => scope_tokens(ix).map(|_| { row.semantic_supported = true;')]),
    ('only first of multiple targets preserved', 'real_multi_action_observations_preserve_both_outer_indices', [(POLICY, 'targets.push(TargetObservation {', 'if targets.is_empty() { targets.push(TargetObservation {'), (POLICY, 'instruction_identity: op.name().into(),\n                    });', 'instruction_identity: op.name().into(),\n                    }); }')]),
    ('two action identities merged', 'real_multi_action_ids_are_not_merged', [(POLICY, 'action_id: op.action_id().into(),', 'action_id: "deposit_reserve_liquidity_and_obligation_collateral".into(),')]),
    ('whole-transaction delta assigned as action attribution', 'real_multi_action_whole_transaction_delta_is_unevaluable', [(POLICY, '{\n            "unsupported_multi_action_attribution"\n        } else {', '{\n            "whole_transaction_delta_assigned_to_first_action"\n        } else {')]),
    ('unknown extra companion hidden after valid envelope', 'unknown_extra_companion_cannot_hide_behind_required_dependencies', [(POLICY, 'tx.instructions.iter().enumerate() {', 'tx.instructions.iter().enumerate().take(8) {'), (CORE, 'self.instructions.len() == message.transaction().instructions.len()', 'true')]),
    ('failed original loses explicit rejection', 'real_failed_original_policy_survives_exact_lut_proof', [(POLICY, 'if !tx.success || tx.error.is_some() {', 'if false {')]),
    ('ATA creation treated as existing account without witness', 'primary_ata_existing_evidence_cannot_be_replaced_by_creation', [(POLICY, 'tx.pre_balances.as_ref().and_then(|v|v.get(key)).is_some_and(|b|*b>0) && tx.pre_token_balances.as_ref().is_some_and(|v|v.iter().any(|b|b.account_index==key && b.mint==a[3].address && b.program_id==a[5].address)) && !tx.inner_instruction_frames.iter().any(|f|usize::from(f.outer_index)==index)', 'true')]),
    ('unexpected recognized duplicate dependency admitted', 'unexpected_known_extra_dependency_is_rejected', [(POLICY, 'if !exact_profile {', 'if !exact_profile && false {')]),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    originals = {name: (lut.REPO / name).read_bytes() for name in (POLICY, CORE)}
    env = {k: v for k, v in os.environ.items() if not any(s in k for s in ('RPC', 'API_KEY', 'ARCHIVE', 'ALCHEMY', 'HELIUS'))}
    env['CARGO_TARGET_DIR'] = str(lut.REPO / 'target/u3c-envelope-mutants')
    env['CARGO_NET_OFFLINE'] = 'true'
    results = []
    with tempfile.TemporaryDirectory(prefix='eplyx-u3c-mutants-') as tmp:
        root = Path(tmp)
        for name in ('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml'):
            shutil.copyfile(lut.REPO / name, root / name)
        for name in ('engine', 'interface', 'server'):
            shutil.copytree(lut.REPO / name, root / name, ignore=shutil.ignore_patterns('target', 'node_modules'))
        for name in ('fixtures', 'docs'):
            (root / name).symlink_to(lut.REPO / name, target_is_directory=True)
        def test(name):
            return subprocess.run(['cargo', 'test', '--offline', '-q', '-p', 'eplyx-engine', '--test', 'transaction_envelope', name, '--', '--exact'], cwd=root, env=env, capture_output=True)
        for number, (description, name, replacements) in enumerate(FAULTS, 1):
            for file, body in originals.items():
                (root / file).write_bytes(body)
            control = test(name)
            lut.require(control.returncode == 0, f'named unmutated control failed: {name}\n{(control.stdout + control.stderr).decode()}')
            changed = {file: body.decode() for file, body in originals.items()}
            for file, old, new in replacements:
                lut.require(changed[file].count(old) == 1, f'M{number} source target is not unique')
                changed[file] = changed[file].replace(old, new)
            try:
                for file, body in changed.items():
                    (root / file).write_text(body)
                run = test(name)
                output = (run.stdout + run.stderr).decode()
                killed = run.returncode != 0 and 'test result: FAILED.' in output and 'panicked at' in output and 'assertion' in output and 'could not compile' not in output and 'error[E' not in output
                boundary = 'transformed frozen message with simulated provenance records; no execution' if number in (4, 5) else 'frozen production message or explicitly transformed negative-control message; no execution'
                results.append({'mutation': number, 'description': description, 'named_test': name, 'control_passed': True, 'evidence_boundary': boundary, 'result': 'killed_by_assertion' if killed else 'FAILED', 'test_exit': run.returncode, 'mutated_source_hashes': {file: lut.sha(body.encode()) for file, body in changed.items()}})
                print(f'M{number:02}: {results[-1]["result"]} — {name}', flush=True)
                lut.require(killed, f'M{number} survived or failed outside an assertion:\n{output}')
            finally:
                for file, body in originals.items():
                    (root / file).write_bytes(body)
                    lut.require((root / file).read_bytes() == body, 'isolated source not restored')
    for file, body in originals.items():
        lut.require((lut.REPO / file).read_bytes() == body, 'production source changed')
    report = {'mutations_executed': len(results), 'killed_by_named_assertions': len(results), 'all_isolated_sources_restored': True, 'separate_cargo_output': 'target/u3c-envelope-mutants', 'production_source_hashes_before': {f: lut.sha(b) for f, b in originals.items()}, 'production_source_hashes_after': {f: lut.sha((lut.REPO / f).read_bytes()) for f in originals}, 'runtime_state_effect_test': 'not attempted; no baseline execution reached', 'results': results}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_bytes(lut.canonical(report))


if __name__ == '__main__':
    main()
