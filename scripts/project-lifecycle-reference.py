#!/usr/bin/env python3
"""Create reviewed MAIN encodings of pinned STA lifecycle analytical records.

Raw observations remain byte-identical. Only integer encoding, multiplier bits,
MAIN's approved exit mapping and digests whose preimages change are projected.
The generator never runs the engine or derives an expected economic outcome.
"""
from pathlib import Path
import hashlib
import json
import copy
import struct
import argparse
ROOT = Path(__file__).resolve().parents[1]
PIN = 'c411ff7226bb515533774ec822829f787fe4bf6910f9816dbbf2e6f2b55baa3e'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--out', type=Path, default=ROOT / 'fixtures/lifecycle/main')
parser.add_argument('--record', type=Path, default=ROOT / 'fixtures/lifecycle/encoding-projection.json')
parser.add_argument('--position-timestamps', action='store_true', help='Append the T7 decimal-string position timestamp projection')
args = parser.parse_args()
if args.position_timestamps and (args.out.resolve() == (ROOT / 'fixtures/lifecycle/main').resolve() or args.record.resolve() == (ROOT / 'fixtures/lifecycle/encoding-projection.json').resolve()):
    raise ValueError('T7 projection requires separate --out and --record paths')
root = ROOT / 'fixtures/lifecycle'
source = root / 'sta'
out = args.out
if not out.resolve() != source.resolve():
    raise ValueError('The original reference cannot be a projection destination')
manifest_bytes = (root / 'provenance.json').read_bytes()
if not hashlib.sha256(manifest_bytes).hexdigest() == PIN:
    raise ValueError('Fixture allowlist changed')
manifest = json.loads(manifest_bytes)
selected = lambda p: p.startswith(('snapshots/', 'reports/', 'policies/', 'scenarios/', 'probes/')) and (not p.startswith('probes/phase7-captures/fixtures/')) and (p not in {'probes/spacex-usdc-dlmm-fixture.json', 'reports/phase14-validation/historical-inputs-before.json'})
fields = set(['active_epoch', 'amount', 'captured_slot', 'class', 'compute_units', 'creation_slot_established', 'deployment_slot', 'discovered_at_slot', 'entity', 'entity_path_context', 'enumeration_slot', 'epoch', 'epoch_start_timestamp', 'feasibility', 'leader_schedule_epoch', 'max_observed_slot', 'min_observed_slot', 'path', 'redundancy_penalty', 'slot', 'state_shape', 'total', 'transaction_fee_lamports', 'unix_timestamp', 'venue'])
if args.position_timestamps:
    fields.add('last_updated_at')
extension_fields = {'withheldAmount', 'maximumFee', 'epoch', 'maximumPendingBalanceCreditCounter', 'pendingBalanceCreditCounter', 'expectedPendingBalanceCreditCounter', 'actualPendingBalanceCreditCounter', 'lastUpdateTimestamp', 'initializationTimestamp', 'newMultiplierEffectiveTimestamp', 'maxSize', 'size', 'memberNumber'}
documents = {}
original = {}
reencoded = {}
hashes = {}
for e in manifest['files']:
    path = source / e['file']
    if not (not Path(e['file']).is_absolute() and '..' not in Path(e['file']).parts):
        raise ValueError('Pinned projection contract mismatch')
    if not all((not p.is_symlink() for p in [path, *path.parents] if p != source.parent)):
        raise ValueError('Symlink in source path')
    if not path.stat().st_size == e['bytes']:
        raise ValueError('Source byte count changed')
    with path.open('rb') as handle:
        data = handle.read(e['bytes'] + 1)
    if not (len(data) == e['bytes'] and hashlib.sha256(data).hexdigest() == e['sha256']):
        raise ValueError('Source digest changed')
    original[e['file']] = data
    hashes[e['sha256']] = e['sha256']
    if selected(e['file']):
        documents[e['file']] = json.loads(data)

def encode(v):
    return (json.dumps(v, ensure_ascii=False, indent=2, allow_nan=False) + '\n').encode()

def convert(v, hashes, key='', in_config=False):
    if isinstance(v, str):
        return hashes.get(v, v)
    if isinstance(v, bool) or v is None:
        return v
    if isinstance(v, int):
        if key in {'readiness_exit_code', 'evaluation_command_exit_code', 'exit_code'}:
            return {3: 1, 4: 5}.get(v, v)
        return str(v) if key in (extension_fields if in_config else fields) else v
    if isinstance(v, float):
        if key == 'uiAmount':
            return v
        if in_config and key in {'multiplier', 'newMultiplier'}:
            return str(struct.unpack('<Q', struct.pack('<d', v))[0])
        raise ValueError('unreviewed floating-point field ' + key)
    if isinstance(v, list):
        return [convert(x, hashes, key, in_config) for x in v]
    # ExpectedGain uses small counts; score weights with the same names are wide integers.
    if {'entities', 'authorities', 'represented_raw', 'entity_path_contexts', 'venue', 'path_type', 'state_shape'} == set(v):
        return v
    # Raw observation envelopes and account payloads keep their original wire bytes.
    if {'method', 'params', 'result'}.issubset(v):
        return v
    if {'data', 'owner', 'lamports'}.issubset(v) and isinstance(v.get('data'), list):
        return v
    return {k + 'Bits' if in_config and k in {'multiplier', 'newMultiplier'} else k: convert(x, hashes, k, in_config or (k == 'config' and 'extension_type' in v)) for k, x in v.items()}
# Internal preimages are verified against STA before any digest is projected.
snapshot = documents['snapshots/spacex-exposure.json']
oldbase = dict(snapshot)
oldbase['schema_version'] = 1
del oldbase['exposures']
oldbasehash = hashlib.sha256(encode(oldbase)).hexdigest()
if not oldbasehash == snapshot['exposures']['source_snapshot_sha256']:
    raise ValueError('Pinned projection contract mismatch')
bundle = documents['probes/spacex-phase8-evidence-bundle.json']
import posixpath
member = lambda ref: posixpath.normpath('probes/' + ref['file'])
oldcoverage = copy.deepcopy(documents[member(bundle['baseline'])])
oldcoverage['entities'].sort(key=lambda e: e['entity_id'])
oldcoveragehash = hashlib.sha256(encode(oldcoverage)).hexdigest()
if not oldcoveragehash == documents[member(bundle['expansion_plan'])]['coverage_sha256']:
    raise ValueError('Pinned projection contract mismatch')
derived = {oldbasehash: oldbase, oldcoveragehash: oldcoverage}
preimages = {oldbasehash: ['snapshots/spacex-exposure.json: schema_version=1, omit exposures'], oldcoveragehash: [member(bundle['baseline']) + ': sort entities by entity_id']}

def register(expected, value, description):
    actual = hashlib.sha256(encode(value)).hexdigest()
    if actual != expected:
        raise ValueError('derived source hash does not match its pinned input')
    if expected in derived and derived[expected] != value:
        raise ValueError('ambiguous derived hash')
    derived[expected] = value
    preimages.setdefault(expected, []).append(description)

def embedded(value, location):
    if isinstance(value, list):
        for i, child in enumerate(value):
            embedded(child, location + '/' + str(i))
    elif isinstance(value, dict):
        for h, key in [('policy_semantic_sha256', 'policy'), ('candidate_plan_sha256', 'candidate'), ('readiness_sha256', 'readiness')]:
            if h in value and key in value:
                register(value[h], value[key], location + '/' + key)
        for key, child in value.items():
            embedded(child, location + '/' + key)
for name, value in documents.items():
    embedded(value, name + '#')
cf = documents['reports/spacex-counterfactual-lifecycle.json']
identity = cf['production_world']
register(identity['production_state_digest'], [identity[k] for k in ['snapshot_sha256', 'exposure_snapshot_sha256', 'position_fixture_sha256', 'position_discovery_sha256', 'raw_position_sha256']], 'counterfactual production_world: tuple(snapshot, exposure snapshot, position fixture, discovery, raw position)')
register(cf['lifecycle_policy_sha256'], cf['lifecycle_scenario']['policy'], 'counterfactual lifecycle_scenario/policy')
register(cf['scenarios'][0]['historical_execution_evidence_sha256'], [cf['historical_direct_paths'], cf['historical_position_paths'], cf['frozen_readiness']['path_evidence'], cf['frozen_readiness']['position_exit_evidence']], 'counterfactual: tuple(historical direct paths, historical position paths, frozen readiness path_evidence, position_exit_evidence)')
for round in range(32):
    next_hashes = dict(hashes)
    for original_hash, value in derived.items():
        next_hashes[original_hash] = hashlib.sha256(encode(convert(value, hashes))).hexdigest()
    for entry in manifest['files']:
        name = entry['file']
        if name in documents:
            value = convert(documents[name], hashes)
            data = encode(value)
            reencoded[name] = data
            next_hashes[entry['sha256']] = hashlib.sha256(data).hexdigest()
    if next_hashes == hashes:
        break
    hashes = next_hashes
else:
    raise ValueError('reference hash dependency graph did not converge')
# All inputs and hash dependencies have verified before the first output write.
out.mkdir(parents=True, exist_ok=True)
for name, data in {'.gitignore': b'*\n!*/\n!.gitignore\n!.lifecycle-root\n!README.md\n', '.lifecycle-root': b'MAIN encoding projection; see sibling encoding-projection.json.\n'}.items():
    target = out / name
    if not target.exists():
        target.write_bytes(data)
records = []
for entry in manifest['files']:
    name = entry['file']
    data = reencoded.get(name, original[name])
    target = out / name
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists() and target.read_bytes() != data:
        raise ValueError('existing derivative differs: ' + name)
    if not target.exists():
        with target.open('xb') as f:
            f.write(data)
    records.append({'file': name, 'source_sha256': entry['sha256'], 'main_sha256': hashlib.sha256(data).hexdigest(), 'source_bytes': entry['bytes'], 'main_bytes': len(data), 'transformation': 'MAIN integer encoding, multiplier bits, exit codes and derived hash links' if data != original[name] else 'exact raw bytes'})
record = json.dumps({'schema_version': 1, 'hash_rounds': round, 'fields': sorted(fields), 'derived_hashes': [{'source_sha256': h, 'main_sha256': hashes[h], 'preimages': sorted(set(preimages[h]))} for h in sorted(derived)], 'files': records}, indent=2) + '\n'
if args.record.exists():
    if not args.record.read_text() == record:
        raise ValueError('Existing projection record differs; never overwrite an accepted reference')
else:
    with args.record.open('x') as handle:
        handle.write(record)
print(f'Projected {len(documents)} analytical documents; preserved all {len(records)} original files; converged in {round} hash rounds.')
