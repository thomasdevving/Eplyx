#!/usr/bin/env python3
"""Repeat this one retained Stake Pool upgrade case on macOS arm64."""
import argparse
import hashlib
import json
import pathlib
import platform
import subprocess
import sys

case = pathlib.Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--out-dir', required=True, type=pathlib.Path, help='A fresh output directory')
args = parser.parse_args()
if platform.system() != 'Darwin' or platform.machine() != 'arm64':
    raise SystemExit('This retained executable was tested on macOS arm64 only.')
manifest = json.loads((case / 'case-manifest.json').read_text())
def commitment(path):
    data = path.read_bytes()
    return {'sha256': hashlib.sha256(data).hexdigest(), 'len': len(data)}
for relative, expected in manifest['artifacts'].items():
    path = case / relative
    if path.resolve().is_relative_to(case) is False or commitment(path) != expected:
        raise SystemExit(f'Artifact identity mismatch: {relative}')
out = args.out_dir.resolve()
out.mkdir(parents=True, exist_ok=False)
policy = case / 'tools/offline.sb'
prefix = ['/usr/bin/sandbox-exec', '-f', str(policy), '/usr/bin/env', '-i', 'PATH=/usr/bin:/bin']
receipt = {'network_denial_enforced': True, 'artifact_identities_verified': True, 'commands': [], 'report_commitments': {}}
probe_code = """import errno,json,socket,sys
try:
    with socket.socket() as sock:
        sock.settimeout(1)
        sock.connect(('127.0.0.1', 9))
except OSError as error:
    print(json.dumps({'errno':error.errno,'error':str(error)}))
    sys.exit(0 if error.errno in (errno.EPERM, errno.EACCES) else 1)
sys.exit(1)
"""
probe = subprocess.run(prefix + [sys.executable, '-c', probe_code], capture_output=True, cwd=case, timeout=5)
(out / 'network-probe.stdout').write_bytes(probe.stdout)
(out / 'network-probe.stderr').write_bytes(probe.stderr)
if probe.returncode != 0:
    raise SystemExit('Network-denial probe did not establish syscall permission denial; analysis withheld.')
receipt['network_probe'] = {'exit_code': probe.returncode, 'result': json.loads(probe.stdout), 'address': '127.0.0.1:9'}
tool = str(case / 'tools/eplyx')
bundle = 'input/bundle'
base = bundle + '/binaries/current.so'
candidate = 'candidate/spl_stake_pool.so'
compare = ['compare', '--corpus', bundle + '/corpus/corpus.json', '--v1', base, '--dependencies', bundle + '/binaries/dependencies', '--no-minimize', '--format', 'json']
commands = [
    ('bundle-verification', ['bundle', 'verify', '--bundle', bundle, '--format', 'json']),
    ('baseline-fidelity', compare + ['--v2', base]),
    ('candidate-ci', ['ci', 'check', '--bundle', bundle, '--change-spec', 'change-spec.json', '--candidate', candidate, '--format', 'json']),
    ('candidate-replay', compare + ['--v2', candidate]),
]
for name, command in commands:
    output = out / (name + '.json')
    if name != 'bundle-verification':
        command += ['--out', str(output)]
    result = subprocess.run(prefix + [tool] + command, cwd=case, capture_output=True, timeout=120)
    if name == 'bundle-verification':
        output.write_bytes(result.stdout)
    else:
        (out / (name + '.stdout')).write_bytes(result.stdout)
    (out / (name + '.stderr')).write_bytes(result.stderr)
    receipt['commands'].append({'argv': [tool] + command, 'sandbox_prefix': prefix, 'cwd': str(case), 'exit_code': result.returncode})
    if result.returncode != 0:
        (out / 'repeat-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
        raise SystemExit(f'{name} failed with exit {result.returncode}; retained outputs were not repaired.')
    if output.read_bytes() != (case / 'reports' / (name + '.json')).read_bytes():
        raise SystemExit(f'{name} report differs from the retained report; outputs were not repaired.')
    receipt['report_commitments'][name] = commitment(output)
receipt['deterministic_reports_byte_identical'] = True
(out / 'repeat-receipt.json').write_text(json.dumps(receipt, indent=2) + '\n')
print(f'All input identities verified; four reports byte-identical under network denial. Outputs: {out}')
