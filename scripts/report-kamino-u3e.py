#!/usr/bin/env python3
"""Derive U3E reporting from retained evidence; no network or execution."""
import base64
from collections import Counter
from datetime import datetime
import json
from pathlib import Path
import re

import kamino_u3e_runtime as runtime

s = runtime.s
lut = s.lut
VALIDATION = s.ROOT / 'phase-u3e-validation'


def write(name, value):
    (VALIDATION / name).write_bytes(lut.canonical(value))


def main():
    freeze = lut.read(VALIDATION / 'preimplementation.json')
    old = lut.read(s.ROOT / 'phase-u3d2-validation/final-report.json')
    state = lut.read(s.ROOT / 'phase-u3e-state/result.json')
    boundary = lut.read(s.ROOT / 'phase-u3e-boundary-proof/result.json')
    bank = lut.read(s.ROOT / 'phase-u3e-runtime/result.json')
    probe = lut.read(s.ROOT / 'phase-u3e-mapping-probe/result.json')
    _, _, inventory = s.inputs()
    for row in inventory['account_rows']:
        row['captured_boundary_facts'] = [a for a in state['acquired'] if a['address'] == row['address']]
        row['ordinary_boundary_bytes_validated'] = bool(row['captured_boundary_facts']) or row['historically_acquired']
        if row['captured_boundary_facts']:
            pre = next(a['facts'] for a in row['captured_boundary_facts'] if a['boundary'] == 'pre')
            row.update(historically_acquired=True, historical_bytes_still_required=False,
                       owner=pre.get('owner'), owner_evidence='historical_raw_boundary_response' if pre['present'] else 'paired_explicit_historical_absence',
                       historical_capture={'source': 'phase-u3e-state', 'pre_boundary_facts': pre},
                       post_reference_slot=s.inventory.SLOT)
    inventory['historical_runtime_inputs'] = bank
    inventory['runtime_dependency_closure_proven'] = False
    inventory['protocol_state_accounts_acquired'] = sum('ProtocolState' in a['categories'] for a in inventory['account_rows'])
    inventory['missing_historical_message_accounts'] = []
    inventory['inventory_complete_for_message_and_observed_binaries'] = True
    write('final-account-inventory.json', inventory)
    captures = {}
    totals = Counter()
    account_images = []
    for name in ('mapping-probe', 'state', 'boundary-proof', 'runtime'):
        root = s.ROOT / f'phase-u3e-{name}'
        receipt = lut.read(root / 'receipt.json')
        attempts = receipt['attempts']
        start = datetime.fromisoformat(attempts[0]['started_at_utc']).timestamp()
        end = datetime.fromisoformat(attempts[-1]['started_at_utc']).timestamp() + attempts[-1]['elapsed_seconds']
        sizes = [p.stat().st_size for p in root.rglob('*') if p.is_file()]
        images = []
        for a in attempts:
            if a['account'] and a['failure_class'] is None:
                value = lut.read(root / a['body_file'])['result']['value']
                if value is not None:
                    images.append(base64.b64decode(value['data'][0]))
            totals[a['failure_class'] or 'success'] += 1
        account_images.extend(images)
        captures[name] = {'requests': len(attempts), 'failed_attempts': sum(a['failure_class'] is not None for a in attempts),
                          'retries': sum(a['attempt'] > 1 for a in attempts),
                          'raw_response_bytes': sum(a['body_bytes'] for a in attempts),
                          'decoded_account_bytes': sum(map(len, images)),
                          'stored_bytes': sum(sizes), 'files': len(sizes),
                          'transport_seconds': sum(a['elapsed_seconds'] for a in attempts),
                          'first_request_start_to_last_response_seconds': end - start}
    unique_images = {lut.sha(data): len(data) for data in account_images}
    storage = {'capture_groups': captures, 'raw_acquisition_bytes': sum(c['raw_response_bytes'] for c in captures.values()),
               'decoded_account_images_bytes_with_repetition': sum(map(len, account_images)),
               'decoded_unique_image_bytes': sum(unique_images.values()),
               'duplicate_decoded_image_bytes': sum(map(len, account_images)) - sum(unique_images.values()),
               'new_binary_bytes': 0, 'binary_storage': 'T1 existing U3C/U3D.2 lossless captures referenced; no new binary requests',
               'runtime_context_record_bytes': 0, 'experimental_replay_record_bytes': 0,
               'post_reference_account_bytes': sum(a['facts'].get('data_bytes', 0) for a in state['acquired'] if a['boundary'] == 'post'),
               'deduplication_implemented': False}
    write('storage.json', storage)
    write('performance.json', {'captures': captures, 'historical_binary_acquisition': 'not repeated',
                              'historical_runtime_input_acquisition': captures['runtime'],
                              'runtime_context_construction': 'not completed', 'native_v0_execution': 'not attempted',
                              'post_state_fidelity_comparison': 'not attempted', 'semantic_evaluation': 'not attempted',
                              'timing_boundary': 'per-request curl/response parsing; span excludes prerequisites and final validation/receipt writes'})
    model = {'decision': 'T1-D', 'failure': bank['failure'], 'historical_inputs': bank['acquired'],
             'pinned_runtime': 'LiteSVM 0.16.0 existing mainnet feature snapshot; no changes',
             'feature_equivalence': 'pinned current-compatible behavior, not exact historical validator equivalence',
             'instructions_sysvar': 'must be runtime-produced from the complete original eight-instruction v0 message; not constructed here',
             'recent_blockhash': 'existing local historical execution disables signature/blockhash validity checks; no T1 message rewritten or executed',
             'slothashes_dependency': 'LiteSVM load_lookup_table_addresses obtains the SlotHashes sysvar cache before lookup',
             'active_lut_limit': 'T1 deactivation_slot is u64::MAX, so this table status check does not inspect historical hash membership. This is not a proof that a replacement sysvar is authoritative for the full historical environment.',
             'null_response_claim': 'qualified archive supplied no SlotHashes account at requested S; not proof of mainnet nonexistence or permanent archive absence',
             'default_sysvar_substitution': 'not performed', 'runtime_context_complete': False,
             'native_v0_execution_attempted': False, 'runtime_managed_exclusions_added': [],
             'source_boundaries': 'Scope interface source is pinned but has not been reproducibly matched to historical ELF'}
    write('runtime-model.json', model)
    funnel = dict(old['modern_funnel'])
    funnel.update(complete_historical_state_sets=1, complete_ordinary_account_boundary_sets=1,
                  complete_historical_execution_input_sets=0, runtime_context_complete=0,
                  targets_with_partial_state_capture=0, semantic_evaluation_possible=0,
                  state_count_definition='ordinary message state and references; runtime bank inputs counted separately')
    write('modern-funnel.json', funnel)
    rows = []
    summary = lut.read(s.ROOT / 'phase-u3c-envelope/summary.json')
    for index, sig in enumerate(summary['primary_signatures'], 1):
        target = f'T{index}'
        rows.append({'target': target, 'signature': sig, 'C1': 'passed', 'C2': 'passed', 'C3': 'passed',
                     'C4': 'passed' if index == 1 else 'failed_in_retained_U3D2_attempt',
                     'C5': 'ordinary_account_boundaries_validated' if index == 1 else 'not_attempted',
                     'runtime_context': 'partial_acquisition_blocked' if index == 1 else 'not_attempted',
                     **{f'C{k}': 'not_attempted' for k in range(6, 11)},
                     'terminal_cause': 'RuntimeContext' if index == 1 else 'Transport',
                     'terminal_evidence': bank['failure'] if index == 1 else old['T2_T4_binary_result'][target],
                     'new_requests': 39 if index == 1 else 0})
    write('primary-stage-table.json', rows)
    blockers = [{'signature': row['signature'], 'cause': row['terminal_cause'], 'evidence': row['terminal_evidence']} for row in rows]
    for path in sorted((s.ROOT / 'phase-u3c-envelope/transactions').glob('*.json')):
        row = lut.read(path)
        if row['transaction']['signature'] in summary['primary_signatures']:
            continue
        cause = 'FailedOriginalPolicy' if not row['on_chain_success'] else 'SemanticAttribution' if row['envelope']['attribution'] == 'unsupported_multi_action_attribution' else 'Envelope'
        blockers.append({'signature': row['transaction']['signature'], 'cause': cause,
                         'evidence': row['envelope']['blockers'], 'execution_attempted': False})
    legacy = lut.read(s.ROOT / 'phase-u3c-envelope/structure-analysis.json')['legacy_system']
    blockers.append({'signature': legacy['signature'], 'cause': 'Envelope', 'evidence': legacy['first_rejection'], 'execution_attempted': False})
    distribution = dict(Counter(row['cause'] for row in blockers))
    lut.require(len(blockers) == len({row['signature'] for row in blockers}) == 9, 'terminal blocker denominator differs')
    write('terminal-blockers.json', {'transactions': blockers, 'distribution': distribution,
                                     'interpretation': 'multi-action attribution is the first experimental policy blocker; additional envelope dependency blockers remain visible, and no whole-transaction replay was attempted'})
    rustlog = (VALIDATION / 'rust-tests.log').read_text()
    rust = [int(n) for n in re.findall(r'test result: ok\. (\d+) passed', rustlog)]
    python = {}
    for log in sorted(VALIDATION.glob('test_*.py.log')):
        value = log.read_text()
        match = re.search(r'Ran (\d+) tests?', value)
        lut.require(match and '\nOK\n' in value, f'Python test failed: {log}')
        python[log.name] = int(match[1])
    lut.require(sum(rust) == 603 and sum(python.values()) == 114, 'test counts differ')
    mutations = lut.read(VALIDATION / 'mutations.json')
    controls = lut.read(VALIDATION / 'after/controls.json')
    report = {
        '01_initial_commit_worktree': freeze,
        '02_stages_reached': {'0_5': 'completed ordinary boundary acquisition, checks and references', '6': 'partial historical runtime input acquisition; stopped at SlotHashes null', '7_12': 'not reached', '13': 'T1-D', '14_23': 'not reached', '24_27': 'reporting and decisions completed'},
        '03_transport_architecture': 'historical_transport.py; stdin-only endpoint config, curl diagnostics, safe header allowlist, exact logical contexts, raw bodies including empty artifacts, independent offline reclassification',
        '04_retry_policy': lut.read(s.transport.POLICY_PATH),
        '05_T1_inventory': 'final-account-inventory.json: 23 message keys, 4 ProgramData, 1 LUT; 16 ordinary accounts at two boundaries, 15 present and 1 authority absent at both',
        '06_T1_state_acquisition': {'ordinary_boundaries_complete': True, 'requests': 32, 'present_account_snapshots': 30, 'absence_proofs': 2, 'typed_decodes': 18, 'all_execution_inputs_complete': False},
        '07_same_slot': boundary['final_same_slot_screen'],
        '08_runtime_context_model': model,
        '09_native_v0_implementation': 'not started; generic executor and replay admission unchanged',
        '10_Scope_execution': 'not attempted; retained archive price changes are not local execution',
        '11_Scope_causal_test': 'not attempted; execution prerequisite missing',
        '12_ATA_execution': 'not attempted; complete historical existing-account bytes validated, no runtime path proven',
        '13_KLend_execution': 'not attempted', '14_T1_outcome_fidelity': 'not attempted', '15_T1_post_state_fidelity': 'not attempted',
        '16_T1_decision': 'T1-D', '17_production_semantic_subjects': 0, '18_first_production_record': None,
        '19_offline_replay': 'not attempted; complete offline evidence reconstruction passed with live transport forbidden',
        '20_ReplayRecord_schema_decision': 'deferred until T1-A; no changes',
        '21_T2_T4_transport_recovery': 'not attempted this phase; T1-A gate not satisfied',
        '22_T2_T4_replay_results': 'not attempted; retained binary transport gaps unchanged',
        '23_experimental_corpus': None, '24_baseline_self_check': 'no production corpus; existing product baseline unchanged',
        '25_candidate_differential': 'not reached; existing U2 synthetic controls passed separately',
        '26_wrapped_SOL_lifecycle': 'not reached', '27_multi_action_replay': 'not reached', '28_attribution': 'unsupported; unchanged',
        '29_modern_replay_funnel': funnel,
        '30_terminal_blockers': {'distribution': distribution, 'evidence': 'terminal-blockers.json',
                                 'boundary': 'experimental policy blockers; semantic attribution blocks two multi-action contracts before execution, with additional envelope gaps retained'},
        '31_tests': {'Rust': sum(rust), 'Rust_groups': rust, 'Python': python, 'total': sum(rust) + sum(python.values()), 'product_controls': controls,
                     'clippy_warnings_denied': True, 'rust_formatting': True, 'primary_VM_executions': 0},
        '32_mutations': {'executed': mutations['executed'], 'killed': mutations['killed'], 'evidence': 'mutations.json', 'runtime_and_multi_action': 'not reached; none counted'},
        '33_performance': 'performance.json', '34_storage': 'storage.json', '35_security_scan': 'security-scan.json',
        '36_correctness_findings': ['Old bodyless evidence lacked enough diagnostics to infer cause; new runs distinguish HTTP null from empty transport.',
                                    'Offline diagnostics also cross-check retained request identity, headers, timing, classification and qualified provider.',
                                    'Explicit zero-lamport authority absence is narrowly allowed; required state and ATA cannot use that exception.',
                                    'No production executor correctness bug was established; no production runtime was exercised.'],
        '37_justified_claims': ['Exact failed mappings context now succeeds; old cause remains unknown.', 'T1 ordinary historical account bytes, references and final same-slot screen are complete under existing archive qualification.', 'Historical Clock, Rent and EpochSchedule acquired.', '39 diagnosed requests: 38 successes and 1 required-runtime-account null; zero live retries and zero empty bodies.'],
        '38_prohibited_claims': ['Complete historical runtime environment', 'native-v0 or Scope/ATA/KLend execution', 'Scope causality', 'outcome/post-state fidelity', 'production-assured semantics or production replay records', 'Kamino production replay above 0/4'],
        '39_adapter_4_decision': 'B. GENERIC MODERN REPLAY STILL BLOCKS NEW PROTOCOL WORK',
        '40_adapter_contract_v2_decision': 'STILL NEED ADAPTER #4; defer new protocol work until generic replay is resolved',
        '41_next_phase': 'Resolve the SlotHashes historical-runtime boundary in a separate versioned study: acquire/reconstruct exact context or prove the narrow active-LUT runtime treatment sufficient for the entire envelope. Then native-v0 execution and mandatory causality/fidelity tests. No blind transport retry of JSON null.',
        'production_replay_matches': {'matched': 0, 'primary_targets': 4}, 'new_attempt_outcomes': dict(totals), 'probe': probe,
        'historical_artifacts_mutated': False, 'commit_created': False,
    }
    write('final-report.json', report)
    print(json.dumps({'report': str(VALIDATION / 'final-report.json'), 'tests': report['31_tests']['total'], 'requests': sum(totals.values()), 'decision': 'T1-D'}))


if __name__ == '__main__':
    main()
