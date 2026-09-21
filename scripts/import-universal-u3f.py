#!/usr/bin/env python3
"""Convert the two retained U3F observations into generic schema-2 evidence.

This is a local migration tool. The Rust importer and resolver perform the
product evidence checks; this script only locates frozen raw source envelopes.
"""
import argparse
import base64
import json
import sys
import tarfile
from pathlib import Path
import subprocess

sys.path.insert(0, str(Path(__file__).resolve().parent))
import kamino_u3f as f
import kamino_u3f_cohort as c

ROOT = f.lut.REPO
ARCHIVES = {}


def raw_source(source):
    if 'file' in source:
        return (ROOT / source['file']).read_bytes()
    name = source['archive']
    if name not in ARCHIVES:
        ARCHIVES[name] = tarfile.open(ROOT / name)
    return ARCHIVES[name].extractfile(source['member']).read()


def receipt_source(source):
    if 'archive' in source:
        name = source['archive']
        if name not in ARCHIVES:
            ARCHIVES[name] = tarfile.open(ROOT / name)
        return ARCHIVES[name].extractfile('receipt.json').read()
    path = ROOT / source['file']
    return (path.parent.parent / 'receipt.json').read_bytes()


def encoded(raw):
    return base64.b64encode(raw).decode()


def item_for(address, raw):
    return {'address': address, 'kind': 'full', 'raw_base64': encoded(raw)}


def sources_for(name, payload):
    slot = payload['frozen']['result']['slot']
    program_addresses = {seed['address'] for seed in payload['seeds'] if seed['kind'] == 'program'}
    program_entries = {address: [] for address in program_addresses}
    if name == 'T1':
        for seed in payload['seeds']:
            if seed['kind'] != 'program':
                continue
            archive_path = f.ROOT / seed['provenance']['archive']
            with tarfile.open(archive_path) as archive:
                receipt = archive.extractfile('receipt.json').read()
                for response in seed['provenance']['responses']:
                    raw = archive.extractfile(response['file']).read()
                    program_entries[seed['address']].append((response['offset'], raw, receipt))
    else:
        cache = c.binary_cache()
        for entry in cache.values():
            if entry.get('method') != 'getAccountInfo' or entry['params'][1]['slot'] != slot - 1:
                continue
            address = entry['params'][0]
            if address not in program_entries:
                continue
            offset = entry['params'][1].get('dataSlice', {}).get('offset', 0)
            source = entry['sources'][0]
            program_entries[address].append((offset, raw_source(source), receipt_source(source)))
    lut_raw = {entry['pubkey']: base64.b64decode(entry['raw_response_base64'])
               for entry in payload['frozen']['evidence']}
    seed_sources = []
    for seed in payload['seeds']:
        address = seed['address']
        if seed['kind'] == 'program':
            entries = sorted(program_entries[address])
            if not entries:
                raise ValueError(f'program seed has no raw chunks: {address}')
            receipts = list(dict.fromkeys(receipt for _, _, receipt in entries))
            seed_sources.append({'address': address, 'kind': 'chunked',
                                 'receipts_base64': [encoded(receipt) for receipt in receipts],
                                 'slices': [{'offset': offset, 'raw_base64': encoded(raw)}
                                            for offset, raw, _ in entries]})
        elif seed['kind'] == 'lut':
            seed_sources.append(item_for(address, lut_raw[address]))
        else:
            source = seed['provenance']
            seed_sources.append(item_for(address, raw_source(source)))
    if name == 'T1':
        root = f.ROOT / 'phase-u3e-state'
        receipt = f.lut.read(root / 'receipt.json')
        entries = [(attempt['account'], attempt['requested_slot'],
                    (root / attempt['body_file']).read_bytes())
                   for attempt in receipt['attempts']
                   if attempt['method'] == 'getAccountInfo' and not attempt['failure_class']]
    else:
        captured = c.checked_client(f.ROOT / 'phase-u3f-cohort-state')
        entries = [(entry['params'][0], entry['params'][1]['slot'], raw_source(entry['sources'][0]))
                   for entry in captured.values() if entry['method'] == 'getAccountInfo']
    by_boundary = {(address, requested): raw for address, requested, raw in entries}
    absent_sources = [item_for(address, by_boundary[address, slot - 1])
                      for address in payload['absent']]
    post_sources = [item_for(address, by_boundary[address, slot])
                    for address in payload['watch'] if (address, slot) in by_boundary]
    return seed_sources, absent_sources, post_sources


def main(output):
    provider = f.lut.read(f.envelope.U3B / 'acquisition.json')['provider']
    binary = ROOT / 'target/debug/examples/import_universal_observation'
    for name in ('T1', 'T2'):
        if name == 'T1':
            payload, _ = f.prepare()
        else:
            payload, _, _ = c.prepare('T2')
        signature = payload['frozen']['result']['transaction']['signatures'][0]
        row = f.lut.read(f.ROOT / f'phase-u3c-envelope/transactions/{signature}.json')
        envelope = row['envelope']
        seed_sources, absent_sources, post_sources = sources_for(name, payload)
        import_value = {'protocol': 'kamino-klend', 'provider': provider, 'payload': payload,
                        'seed_sources': seed_sources, 'absent_sources': absent_sources,
                        'post_sources': post_sources,
                        'instruction_roles': [ix['role'] for ix in envelope['instructions']],
                        'target_outer_index': envelope['targets'][0]['outer_index'],
                        'target_identity': envelope['targets'][0]['instruction_identity']}
        run = subprocess.run([str(binary), str(output)], cwd=ROOT,
                             input=f.lut.canonical(import_value), capture_output=True, timeout=600)
        if run.returncode:
            raise ValueError(f'{name} importer failed: {run.stderr.decode(errors="replace")[:4000]}')
        print(json.dumps({'name': name, 'record': json.loads(run.stdout),
                          'seed_sources': len(seed_sources), 'post_sources': len(post_sources)}), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    main(args.output.resolve())
