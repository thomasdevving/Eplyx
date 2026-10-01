#!/usr/bin/env python3
"""Qualify one explicit integrated CLI against verified private Step 15A evidence.

No acquisition or fallback executable. Requires the preservation manifests and
their exact retained packages, Python 3 and macOS arm64 sandbox-exec.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys

PROPOSAL = '7f24680b92d65edec79f97175be9a81cb35dbf8670581af13519c8f444da3a11'
CANDIDATE = '1113fbd035048c456d2768325e4e6d3699672070267e68b96c01abbc080fdb62'
BUNDLE = '5e5b67ac13e4f6b8249348ad81db29885ee8ee897ba78f6de213b55793f4285f'

def digest(path):
    data = path.read_bytes()
    return {'sha256': hashlib.sha256(data).hexdigest(), 'len': len(data)}

def read(path):
    return json.loads(path.read_text())

def canonical(value):
    return (json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + '\n').encode()

def seal_report(value):
    value.pop('report_sha256', None)
    value['report_sha256'] = hashlib.sha256(canonical(value)).hexdigest()

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', required=True, type=Path)
    parser.add_argument('--evidence-root', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    cli, evidence, out = args.cli.resolve(strict=True), args.evidence_root.resolve(strict=True), args.out.resolve()
    if platform.system() != 'Darwin' or platform.machine() != 'arm64':
        raise SystemExit('Qualification requires the already qualified macOS arm64 host.')
    if out.exists() or out.is_relative_to(evidence):
        raise SystemExit('Output must be fresh and outside immutable preserved evidence.')
    os.umask(0o077)
    out.mkdir(parents=True, mode=0o700)
    receipt = {'status': 'integration_blocked', 'cli': str(cli), 'cli_commitment': digest(cli),
               'evidence_root': str(evidence), 'host': platform.platform(), 'commands': [], 'checks': {}}

    def save():
        (out / 'integration-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')

    try:
        verified = 0
        for name in ['preservation-manifest.json', 'supplemental-preservation-manifest.json', 'migration-preservation-manifest.json']:
            manifest = read(evidence / name)
            for entry in manifest['files']:
                path = Path(entry['preserved'])
                if not path.is_relative_to(evidence) or any(p.is_symlink() for p in [path, *path.parents]):
                    raise ValueError(f'Unsafe preserved reference: {path}')
                if digest(path) != {k: entry[k] for k in ('sha256', 'len')}:
                    raise ValueError(f'Missing or changed required evidence: {entry["logical_name"]}')
                if path.stat().st_mode & 0o111 != entry['mode'] & 0o111:
                    raise ValueError(f'Executable permission changed: {entry["logical_name"]}')
                verified += 1
        receipt['verified_preserved_files'] = verified
        candidate = evidence / 'evidence/step13a'
        case_manifest = read(candidate / 'case-manifest.json')
        for relative, expected in case_manifest['artifacts'].items():
            if digest(candidate / relative) != expected:
                raise ValueError(f'Original candidate package commitment differs: {relative}')
        assert digest(candidate / 'candidate/spl_stake_pool.so') == {'sha256': CANDIDATE, 'len': 293200}
        assert case_manifest['bundle']['bundle_sha256'] == BUNDLE
        initial = read(evidence / 'initial-inventory.json')
        owner_home = Path.home()
        denied = ['/private/tmp', *[v['path'] for v in initial.values()],
                  *[str(owner_home / p) for p in ['.ssh', '.aws', '.codex', '.config', '.cargo']]]
        policy = out / 'offline.sb'
        rules = '\n'.join('(deny file-read* (subpath ' + json.dumps(p) + '))' for p in denied)
        policy.write_text('(version 1)\n(allow default)\n(deny network*)\n' + rules + '\n'
                          + '(allow file-read* (subpath ' + json.dumps(str(out)) + '))\n')
        prefix = ['/usr/bin/sandbox-exec', '-f', str(policy), '/usr/bin/env', '-i', 'PATH=/usr/bin:/bin']

        def run(name, argv, expected=0, document=None):
            result = subprocess.run(prefix + [str(cli), *map(str, argv)], cwd=out, capture_output=True, timeout=900)
            (out / (name + '.stdout')).write_bytes(result.stdout)
            (out / (name + '.stderr')).write_bytes(result.stderr)
            receipt['commands'].append({'name': name, 'argv': [str(cli), *map(str, argv)], 'exit_code': result.returncode,
                                        'stdout': digest(out / (name + '.stdout')), 'stderr': digest(out / (name + '.stderr'))})
            save()
            if (expected == 0 and result.returncode != 0) or (expected != 0 and result.returncode == 0):
                raise RuntimeError(f'{name}: unexpected exit {result.returncode}; inspect retained stderr')
            if expected != 0:
                return None
            if document:
                return read(document)
            return json.loads(result.stdout) if result.stdout.strip().startswith(b'{') else result.stdout.decode()

        probe = "import errno,json,socket,sys\ntry:\n s=socket.socket();s.connect(('127.0.0.1',9))\nexcept OSError as e:\n print(json.dumps({'errno':e.errno}));sys.exit(0 if e.errno in (errno.EPERM,errno.EACCES) else 1)\nsys.exit(1)"
        control = subprocess.run(prefix + [sys.executable, '-c', probe], cwd=out, capture_output=True, timeout=5)
        assert control.returncode == 0, 'Network denial control did not establish permission denial'
        receipt['network_control'] = json.loads(control.stdout)
        control = subprocess.run(prefix + ['/bin/cat', initial['step12a']['path'] + '/engine/src/executor.rs'], cwd=out, capture_output=True)
        assert control.returncode != 0 and b'Operation not permitted' in control.stderr, 'Source-worktree read denial was not established'
        receipt['source_read_denial_control'] = {'exit_code': control.returncode, 'stderr': control.stderr.decode()}
        receipt['sandbox_boundary'] = {'network': 'deny network* including nested workers', 'denied_reads': denied,
                                       'available': 'durable evidence, explicit CLI/output and normal system/runtime files; no cross-platform qualification'}
        receipt['version'] = run('version', ['version', '--json'])
        for command in [['parameter'], *[['parameter', x] for x in ['analyse', 'reproduce', 'search', 'verify-search', 'reproduce-search', 'reproduce-witness']],
                        ['parameter', 'cases'], *[['parameter', 'cases', x] for x in ['prepare', 'analyse', 'verify', 'reproduce']], ['interaction']]:
            run('help-' + '-'.join(command), [*command, '--help'])
        run('dispatch-required-arguments', ['parameter', 'search'], expected=2)
        run('dispatch-mutual-exclusion', ['parameter', 'analyse', '--change', 'missing', '--capture', 'a', '--input', 'b', '--out', 'c'], expected=2)

        parent = evidence / 'evidence/search-parent'
        archived_search = evidence / 'evidence/step12a/parameter-search'
        args_search = ['parameter', 'search', '--change', parent / 'change.json', '--parent-report', parent / 'parent-report.json', '--spec', parent / 'search-spec.json', '--format', 'json']
        fresh = out / 'search'
        search = run('search', [*args_search, '--out', fresh])
        assert search['change_spec_id'] == PROPOSAL
        expected = {'unique_evaluated_amounts': 64, 'total_vm_calls': 128, 'refinement_evaluations': 16, 'matching_cases': 25,
                    'untested_amount_count': '9936', 'completion_status': 'budget_exhausted', 'smallest_matching_amount_among_executed_cases_raw': '6732'}
        for key, value in expected.items():
            assert search['summary'][key] == value, f'Search reference regression: {key}'
        assert search['operation_vm_calls'] == 128
        # Inspect the retained ledger projection, including the zero-credit rounding control.
        summary = read(fresh / 'summary.json')
        receipt['checks']['search_projection_keys'] = list(summary)
        assert search['report_sha256'] == read(evidence / 'evidence/step12a/search-receipt.json')['report_sha256'], 'Search identity/order changed despite unchanged search runtime'
        def resolve(value):
            while isinstance(value, dict) and set(value) == {'parameter_search_cas'}:
                ref = value['parameter_search_cas']
                path = fresh / 'evidence' / ref['kind'] / ref['sha256']
                assert digest(path)['sha256'] == ref['sha256']
                value = read(path)
            return value
        report = resolve(read(fresh / 'manifest.json')['report'])
        ledger = resolve(report['ledger'])
        zero = next(resolve(c) for c in ledger if resolve(resolve(c)['candidate'])['amount_raw'] == '1')
        assert [resolve(zero['outcome'])[k] for k in ['baseline_credit_raw', 'proposed_credit_raw', 'loss_raw']] == ['0', '0', '0']
        run('fresh-search-output-protection', [*args_search, '--out', fresh], expected=2)
        bad_spec = read(parent / 'search-spec.json');bad_spec['extra'] = True
        (out / 'bad-spec.json').write_bytes(canonical(bad_spec))
        run('extra-search-field-rejected', [*args_search[:6], '--spec', out / 'bad-spec.json', '--out', out / 'bad-search', '--format', 'json'], expected=2)
        for label, artifact in [('fresh', fresh), ('archived', archived_search)]:
            verification = run(label + '-verify-search', ['parameter', 'verify-search', '--artifact', artifact, '--format', 'json'])
            assert verification['operation_vm_calls'] == 0
            reproduction = run(label + '-reproduce-search', ['parameter', 'reproduce-search', '--artifact', artifact, '--format', 'json'])
            assert reproduction['operation_vm_calls'] == 128
            assert reproduction['report_sha256'] == verification['report_sha256']
            witness = next(w for w in verification['witnesses'] if w['amount_raw'] == '6732')
            assert [witness['outcome'][k] for k in ['baseline_credit_raw', 'proposed_credit_raw', 'loss_raw']] == ['6698', '6597', '101']
            replay = run(label + '-reproduce-witness', ['parameter', 'reproduce-witness', '--artifact', artifact, '--witness', witness['witness_sha256'], '--format', 'json'])
            assert replay['vm_calls'] == 2 and replay['reproduced'] is True
            receipt['checks'][label + '_search'] = {'input': verification['search_input_sha256'], 'report': verification['report_sha256'], 'counts': expected, 'verify_vm_calls': 0, 'reproduce_vm_calls': 128, 'witness_vm_calls': 2}
        for w in search['witnesses']:
            if w['amount_raw'] == '10000':
                assert [w['outcome'][k] for k in ['baseline_credit_raw', 'proposed_credit_raw', 'loss_raw']] == ['9950', '9800', '150']

        archived_cases = evidence / 'evidence/step14a'
        inputs = [archived_cases / f'input-{i}.json' for i in range(2)]
        original_inputs = [digest(p) for p in inputs]
        request, cases = out / 'cases-request', out / 'cases'
        prepared = run('cases-prepare', ['parameter', 'cases', 'prepare', '--change', archived_cases / 'change.json', '--input', *inputs, '--out', request])
        analysed = run('cases-analyse', ['parameter', 'cases', 'analyse', '--manifest', request / 'manifest.json', '--out', cases])
        assert analysed['vm_calls'] == 4
        expected_quantities = {'17621': ['17532', '17268', '-264', '89', '353'], '309138': ['307592', '302955', '-4637', '1546', '6183']}
        for label, package in [('fresh', cases), ('archived', archived_cases)]:
            summary = read(package / 'summary.json')
            assert summary['change_spec_id'] == PROPOSAL
            assert summary['counts'] == {'selected_cases': 2, 'executed_pairs': 2, 'reconciled_pairs': 2, 'measured_consequence': 2, 'no_observed_consequence': 0, 'unavailable_or_failed': 0}
            assert len({r['source'] for r in summary['rows']}) == 2
            for row in summary['rows']:
                assert [row['quantities'][k] for k in ['baseline_recipient_credit_raw', 'proposed_recipient_credit_raw', 'recipient_credit_difference_raw', 'baseline_destination_withheld_change_raw', 'proposed_destination_withheld_change_raw']] == expected_quantities[row['amount_raw']]
            for operation in ['verify', 'reproduce']:
                result = run(label + '-cases-' + operation, ['parameter', 'cases', operation, '--package', package])
                assert result['summary_verified'] and result['status'] == 'selected_case_set_evaluated'
                assert result['reproduction_performed'] == (operation == 'reproduce')
            for i in range(2):
                run(label + f'-case-{i}-single-reproduction', ['parameter', 'reproduce', '--change', package / 'change.json', '--report', package / f'report-{i}.json'])
            receipt['checks'][label + '_cases'] = {'case_set_id': summary['case_set_id'], 'result': summary['result_sha256'], 'counts': summary['counts'], 'vm_calls_per_analysis_or_repeat': 4, 'verification_vm_calls': 0}
        assert [digest(p) for p in inputs] == original_inputs
        assert read(archived_cases / 'manifest.json')['case_set_id'] == '5149236f71f18bec3b5ecda0ffeb3e387d222e2b0c1598e8b6be5c0e3cfbc73b'
        assert read(archived_cases / 'summary.json')['result_sha256'] == 'd53da5d4a16955d30683aa217be582d0e81d1f91f1b01a5c470cef01edc681b1'
        # Fully reseal each negative report, so rejection establishes semantic/runtime checks.
        for mutation in ['runtime', 'shared-runtime', 'measured-output']:
            report = read(archived_cases / 'report-0.json')
            if mutation == 'runtime':
                report['runtime']['profile'] = 'mutated'
                report['shared_execution']['runtime'] = report['runtime'].copy()
                report['shared_execution_sha256'] = hashlib.sha256(canonical(report['shared_execution'])).hexdigest()
            elif mutation == 'shared-runtime':
                report['shared_execution']['runtime']['profile'] = 'mutated'
                report['shared_execution_sha256'] = hashlib.sha256(canonical(report['shared_execution'])).hexdigest()
            else:
                report['proposed']['reconciliation']['output_received_raw'] = '0'
            seal_report(report); path = out / ('negative-' + mutation + '.json');path.write_bytes(canonical(report))
            run('reject-resealed-' + mutation, ['parameter', 'reproduce', '--change', archived_cases / 'change.json', '--report', path], expected=2)

        for logical in ['single-parameter']:
            package = evidence / 'evidence' / logical
            result = run('archived-' + logical, ['parameter', 'reproduce', '--change', package / 'change.json', '--report', package / 'report.json'])
            assert result['reproduced'] is True
        # This extra scratch package is not the published frozen Step 8 receipt.
        # Its a837.../4417... pair was rejected by Step 12A before integration.
        scratch = evidence / 'evidence/prior-token-parameter'
        run('reject-unsupported-prior-scratch', ['parameter', 'reproduce', '--change', scratch / 'change.json', '--report', scratch / 'report.json'], expected=2)
        assert 'unsupported prior Token-2022 contract source/lock' in (out / 'reject-unsupported-prior-scratch.stderr').read_text()
        receipt['checks']['unsupported_prior_scratch'] = {'report': read(scratch / 'report.json')['report_sha256'],
                                                        'runtime': read(scratch / 'report.json')['runtime'],
                                                        'supported': False, 'original_bytes_preserved': True,
                                                        'limitation': 'Pre-existing unsupported source/lock pair; not the published frozen Step 8 report. Original generating executable not located; no compatibility expansion.'}
        bundle = candidate / 'input/bundle'; baseline = bundle / 'binaries/current.so'
        compare = ['compare', '--corpus', bundle / 'corpus/corpus.json', '--v1', baseline, '--dependencies', bundle / 'binaries/dependencies', '--no-minimize', '--format', 'json']
        run('bundle-verification', ['bundle', 'verify', '--bundle', bundle, '--format', 'json'])
        base_report = out / 'baseline.json'
        base = run('baseline-fidelity', [*compare, '--v2', baseline, '--out', base_report], document=base_report)
        candidate_report = out / 'candidate.json'
        upgraded = run('candidate-replay', [*compare, '--v2', candidate / 'candidate/spl_stake_pool.so', '--out', candidate_report], document=candidate_report)
        gate_path = out / 'candidate-ci.json'
        gate = run('candidate-ci', ['ci', 'check', '--bundle', bundle, '--change-spec', candidate / 'change-spec.json', '--candidate', candidate / 'candidate/spl_stake_pool.so', '--format', 'json', '--out', gate_path], document=gate_path)
        assert gate['summary']['passed'] and gate['summary']['exit_code'] == 0
        assert len(base['observations']) == len(upgraded['observations']) == 10
        assert all(o['fidelity'] == 'matched' for o in base['observations'])
        for o in upgraded['observations']:
            assert o['post_v1_state_hash'] == o['post_v2_state_hash']
            assert o['cpi_graph_v1'] == o['cpi_graph_v2'] and not o['cpi_graph_changed']
            assert all(x['v1'] == x['v2'] for x in o['economic_summary'] if x['economic'])
        assert all(d['v1']['success'] and d['v2']['success'] for d in upgraded['analysis']['diffs'])
        assert base == read(candidate / 'reports/baseline-fidelity.json')
        assert upgraded == read(candidate / 'reports/candidate-replay.json')
        assert gate == read(candidate / 'reports/candidate-ci.json')
        receipt['checks']['candidate'] = {'sha256': CANDIDATE, 'len': 293200, 'source_commit': case_manifest['candidate']['commit'], 'bundle': BUNDLE, 'matched': 10, 'candidate_successes': 10, 'gate_exit': 0, 'ordinary_upgrade_only': True}
        historical = subprocess.run([sys.executable, str(candidate / 'rerun.py'), '--out-dir', str(out / 'historical-candidate-repeat')],
                                    cwd=candidate, env={'PATH': '/usr/bin:/bin'}, capture_output=True, timeout=900)
        (out / 'historical-candidate.stdout').write_bytes(historical.stdout)
        (out / 'historical-candidate.stderr').write_bytes(historical.stderr)
        assert historical.returncode == 0, 'Original candidate engine/script rerun failed'
        receipt['checks']['historical_candidate'] = {'engine': digest(candidate / 'tools/eplyx'), 'separate_original_engine': True,
                                                  'receipt': read(out / 'historical-candidate-repeat/repeat-receipt.json')}
        receipt['status'] = 'integrated_and_verified'
        save()
        print(json.dumps({'status': receipt['status'], 'receipt': str(out / 'integration-receipt.json')}))
    except Exception as error:
        receipt['error'] = str(error)
        save()
        raise SystemExit(f'Integration check failed: {error}; receipt: {out / "integration-receipt.json"}')

if __name__ == '__main__':
    main()
