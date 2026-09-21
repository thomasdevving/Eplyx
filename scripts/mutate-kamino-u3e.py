#!/usr/bin/env python3
"""Isolated behavioral mutations. Simulations do not count as runtime evidence."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

import kamino_u3b_lut as lut

T = 'historical_transport.py'
S = 'kamino_u3e_state.py'
OLD = 'kamino_u3d_state.py'
EXACT = "lut.canonical(params) == exact and self.transport.provider == self.provider"
FAULTS = [
    (1, 'empty body becomes missing account', 'TransportTests.test_empty_response_is_never_missing_account',
     [(T, "return 'empty_http_body', None", "return 'missing_account', None")]),
    (2, 'current state fallback after transport error', 'TransportTests.test_four_exact_attempts_with_no_current_or_provider_fallback',
     [(T, EXACT, 'True'), (T, "requested_slot=params[1]['slot']", "requested_slot=params[1].get('slot')"),
      (T, "if failure not in self.policy['retryable'] or record['response_withheld']:", "params[1].pop('slot', None)\n            if failure not in self.policy['retryable'] or record['response_withheld']:")]),
    (3, 'retry changes historical slot', 'TransportTests.test_four_exact_attempts_with_no_current_or_provider_fallback',
     [(T, EXACT, 'True'), (T, "if failure not in self.policy['retryable'] or record['response_withheld']:", "params[1]['slot'] += 1\n            if failure not in self.policy['retryable'] or record['response_withheld']:")]),
    (4, 'silent provider switch', 'TransportTests.test_four_exact_attempts_with_no_current_or_provider_fallback',
     [(T, EXACT, 'True'), (T, "record['provider'] == self.provider", 'True'),
      (T, "if failure not in self.policy['retryable'] or record['response_withheld']:", "self.transport.provider = 'https://other.example'\n            if failure not in self.policy['retryable'] or record['response_withheld']:")]),
    (5, 'infinite retry policy', 'TransportTests.test_four_exact_attempts_with_no_current_or_provider_fallback',
     [(T, "enumerate(self.policy['backoff_seconds'], 1)", "enumerate(__import__('itertools').cycle(self.policy['backoff_seconds']), 1)"),
      (T, "number <= self.policy['max_attempts_per_context']", 'True')]),
    (6, 'credential URL persisted', 'TransportTests.test_no_credential_url_or_echo_is_persisted',
     [(T, "'provider': self.provider, 'response_withheld': False", "'provider': self.endpoint, 'response_withheld': False"),
      (T, 'lut.baseline.hygiene(record)', 'pass')]),
    (7, 'contradictory account ignored', 'TransportTests.test_contradictory_account_not_retried_or_ignored',
     [(T, "record.update(failure_class='contradictory_account', validation_failure=scrub(str(exc)))", "record.update(failure_class=None, validation_failure=None)")]),
    (8, 'success deletes failure provenance', 'TransportTests.test_later_success_keeps_failed_attempt_provenance',
     [(T, "self.receipt['attempts'].append(record)", "self.receipt['attempts'].clear()\n            self.receipt['attempts'].append(record)")]),
    (9, 'required state absence accepted', 'StateTests.test_missing_required_account_rejected',
     [(OLD, 'expected_lamports == 0', 'True'), (S, "permits_absence(row, item), 'required historical state absent'", "True, 'required historical state absent'")]),
    (10, 'wrong owner accepted', 'StateTests.test_wrong_historical_owner_rejected',
     [(OLD, "account['owner'] == envelope.SCOPE and not account['executable']", 'True'), (S, "account['owner'] == expected", 'True')]),
    (11, 'wrong historical pre hash accepted', 'StateTests.test_wrong_known_pre_hash_rejected',
     [(S, "prior is None or facts['data_sha256'] == prior", 'True')]),
    (12, 'post context seeds pre boundary', 'StateTests.test_post_state_cannot_seed_pre_boundary',
     [(OLD, "response.get('context', {}).get('slot') == item['slot']", 'True')]),
    (13, 'same-slot interference ignored', 'StateTests.test_same_slot_conflict_rejected',
     [(OLD, "and not screen['conflicts'] and required <= set(screen['required_accounts'])", 'and True')]),
    (14, 'Scope mapping omitted', 'StateTests.test_mapping_omission_rejected_before_requests',
     [(S, "derived['planned_boundary_requests'] == state.state_plan(row, binaries)", 'True')]),
    (15, 'ATA existence accepted from metadata', 'StateTests.test_ata_metadata_never_substitutes_missing_bytes',
     [(S, "facts = state.account_check(row, item, response)", "if item['address'] == row['transaction']['instructions'][1]['accounts'][1]['address']:\n        return {'present': True, 'source': 'metadata_only_mutant'}\n    facts = state.account_check(row, item, response)")]),
    (16, 'current oracle state substituted', 'StateTests.test_current_oracle_context_rejected',
     [(OLD, "response.get('context', {}).get('slot') == item['slot']", 'True')]),
]


def main():
    scripts = lut.REPO / 'scripts'
    sources = {name: (scripts / name).read_text() for name in (T, S, OLD)}
    env = {k: v for k, v in os.environ.items() if not any(x in k.upper() for x in ('RPC', 'ARCHIVE', 'API_KEY'))}
    env['PYTHONPATH'] = str(scripts)
    results = []
    with tempfile.TemporaryDirectory(prefix='eplyx-u3e-mutants-') as tmp:
        tmp = Path(tmp)
        for name in ('test_historical_transport.py', 'test_kamino_u3e_state.py', 'capture-kamino-u3b-luts.py'):
            shutil.copyfile(scripts / name, tmp / name)
        for number, description, test, edits in FAULTS:
            for name, source in sources.items():
                (tmp / name).write_text(source)
            file = 'test_historical_transport.py' if test.startswith('Transport') else 'test_kamino_u3e_state.py'
            command = ['python3', '-B', str(tmp / file), test]
            control = subprocess.run(command, env=env, capture_output=True, text=True, timeout=30)
            lut.require(control.returncode == 0, f'control failed: {test}\n{control.stderr}')
            changed = dict(sources)
            for name, old, new in edits:
                lut.require(changed[name].count(old) == 1, f'mutation target not unique: {number}: {old}')
                changed[name] = changed[name].replace(old, new)
            for name, source in changed.items():
                (tmp / name).write_text(source)
            run = subprocess.run(command, env=env, capture_output=True, text=True, timeout=30)
            output = run.stdout + run.stderr
            killed = run.returncode != 0 and 'AssertionError' in output and 'FAIL:' in output and 'ERROR:' not in output and 'SyntaxError' not in output
            lut.require(killed, f'mutant survived or failed outside assertion: {number}\n{output}')
            results.append({'number': number, 'description': description, 'named_assertion': test,
                            'unmutated_control_passed': True, 'killed_by_assertion': True,
                            'mutated_sources_sha256': {n: lut.sha(v.encode()) for n, v in changed.items() if v != sources[n]},
                            'assertion_output': output, 'evidence_boundary': 'acquisition guards and simulated faults; no runtime execution'})
            print(f'{number}: killed: {description}', flush=True)
    lut.require(all((scripts / name).read_text() == source for name, source in sources.items()), 'working sources changed')
    (lut.REPO / 'docs/examples/phase-u3e-validation/mutations.json').write_bytes(lut.canonical({
        'executed': len(results), 'killed': len(results), 'results': results,
        'source_sha256': {n: lut.sha(v.encode()) for n, v in sources.items()},
        'runtime_mutations_17_28': 'not reached: execution prerequisite', 'multi_action_mutations_29_32': 'not reached'}))


if __name__ == '__main__':
    main()
