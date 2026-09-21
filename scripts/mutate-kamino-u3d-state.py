#!/usr/bin/env python3
"""Isolated acquisition faults, killed only by named test assertions."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

import kamino_u3b_lut as lut

FAULTS = [
    ('current/post state context accepted', 'test_current_or_post_scope_state_cannot_seed_pre_state',
     [("response.get('context', {}).get('slot') == item['slot']", 'True')]),
    ('Scope owner ignored', 'test_scope_owner_and_validator_balance_must_match',
     [("account['owner'] == envelope.SCOPE and not account['executable']", 'True')]),
    ('validator balance contradiction ignored', 'test_scope_owner_and_validator_balance_must_match',
     [("type(account.get('lamports')) is int and account['lamports'] == expected_lamports", 'True')]),
    ('missing oracle state accepted', 'test_missing_or_truncated_oracle_state_is_rejected',
     [('expected_lamports == 0', 'True')]),
    ('same-slot conflict and coverage ignored', 'test_same_slot_ambiguity_and_missing_coverage_rejected',
     [("and not screen['conflicts'] and required <= set(screen['required_accounts'])", 'and True')]),
    ('raw response hash mismatch ignored', 'test_raw_hash_mismatch_cannot_be_ignored',
     [("lut.sha(body) == request['response_sha256'] == receipt['raw_artifact_hashes'][ref]\n                        and len(body) == request['response_bytes']", 'True')]),
    ('retry bound exceeded', 'test_transport_stops_at_bounded_limit_and_retains_empty_bodies',
     [('for attempt in (1, 2):', 'for attempt in (1, 2, 3):'), ('if not retryable or attempt == 2:', 'if not retryable or attempt == 3:')]),
    ('ATA wallet check bypassed', 'test_synthetic_ata_bytes_must_match_wallet_mint_and_initialized_state',
     [("data[32:64] == lut.baseline.b58decode(ata[2]['address'])", 'True')]),
]


def main():
    scripts = lut.REPO / 'scripts'
    source = (scripts / 'kamino_u3d_state.py').read_text()
    env = {k: v for k, v in os.environ.items() if not any(x in k for x in ('RPC', 'ARCHIVE', 'API_KEY'))}
    results = []
    with tempfile.TemporaryDirectory(prefix='eplyx-u3d-state-mutants-') as tmp:
        tmp = Path(tmp)
        env['PYTHONPATH'] = str(scripts)
        for file in ('test_kamino_u3d_state.py', 'capture-kamino-u3b-luts.py'):
            shutil.copyfile(scripts / file, tmp / file)
        for description, test, replacements in FAULTS:
            path = tmp / 'kamino_u3d_state.py'
            path.write_text(source)
            shutil.rmtree(tmp / '__pycache__', ignore_errors=True)
            command = ['python3', '-B', str(tmp / 'test_kamino_u3d_state.py'), f'StateTests.{test}']
            control = subprocess.run(command, env=env, capture_output=True, text=True)
            lut.require(control.returncode == 0, f'unmutated control failed: {test}\n{control.stderr}')
            changed = source
            for old, new in replacements:
                lut.require(changed.count(old) == 1, 'mutation target must be unique')
                changed = changed.replace(old, new)
            path.write_text(changed)
            run = subprocess.run(command, env=env, capture_output=True, text=True)
            output = run.stdout + run.stderr
            killed = run.returncode != 0 and 'AssertionError' in output and 'FAIL:' in output and 'ERROR:' not in output and 'SyntaxError' not in output
            lut.require(killed, f'mutant survived or failed outside assertion: {description}\n{output}')
            results.append({'description': description, 'named_test': test, 'control_passed': True,
                            'killed_by_assertion': True, 'mutated_source_sha256': lut.sha(changed.encode()),
                            'evidence_boundary': 'raw partial capture and explicitly synthetic negative controls; no local execution'})
            print(f'killed: {description}', flush=True)
    lut.require((scripts / 'kamino_u3d_state.py').read_text() == source, 'working source changed')
    output = lut.REPO / 'docs/examples/phase-u3d2-validation/state-mutations.json'
    output.write_bytes(lut.canonical({'mutations_executed': len(results), 'killed_by_named_assertions': len(results),
                                     'working_source_unchanged': True, 'source_sha256': lut.sha(source.encode()), 'results': results}))


if __name__ == '__main__':
    main()
