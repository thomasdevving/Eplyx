#!/usr/bin/env python3
"""Measure shared schema-2 CAS against per-observation evidence copies."""
import argparse
import json
from collections import defaultdict
from pathlib import Path

DIRS = {
    'account_content': 'accounts/content',
    'account_observation': 'accounts/observations',
    'account_chunk': 'accounts/chunks',
    'chunked_account_observation': 'accounts/chunked-observations',
    'response_template': 'accounts/responses',
    'transaction': 'transactions',
    'program_binary': 'programs',
    'runtime': 'runtime',
    'validator': 'validator',
}


def references(value):
    if isinstance(value, dict):
        if value.get('kind') in DIRS and isinstance(value.get('sha256'), str):
            yield value['kind'], value['sha256']
            return
        for child in value.values():
            yield from references(child)
    elif isinstance(value, list):
        for child in value:
            yield from references(child)


def reachable(record, evidence):
    seen = set()
    pending = list(references(record))
    while pending:
        key = pending.pop()
        if key in seen:
            continue
        seen.add(key)
        path = evidence / DIRS[key[0]] / key[1]
        raw = path.read_bytes()
        try:
            pending.extend(references(json.loads(raw)))
        except (ValueError, UnicodeDecodeError):
            pass
    return seen


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('corpus', type=Path)
    args = parser.parse_args()
    corpus = args.corpus
    evidence = corpus / 'evidence'
    records = sorted((corpus / 'records').glob('*.json'))
    by_record = {p.stem: reachable(json.loads(p.read_bytes()), evidence) for p in records}
    union = set().union(*by_record.values())
    sizes = {key: (evidence / DIRS[key[0]] / key[1]).stat().st_size for key in union}
    per_record = {name: sum(sizes[key] for key in keys) for name, keys in by_record.items()}
    categories = defaultdict(lambda: {'objects': 0, 'bytes': 0})
    for key, size in sizes.items():
        category = DIRS[key[0]]
        categories[category]['objects'] += 1
        categories[category]['bytes'] += size
    physical = sum(p.stat().st_size for p in evidence.rglob('*') if p.is_file())
    manifest_bytes = sum(p.stat().st_size for p in records)
    print(json.dumps({
        'records': len(records), 'record_manifest_bytes': manifest_bytes,
        'independent_evidence_bytes': sum(per_record.values()),
        'per_record_evidence_bytes': per_record,
        'shared_referenced_evidence_bytes': sum(sizes.values()),
        'shared_physical_evidence_bytes': physical,
        'saved_by_shared_references_bytes': sum(per_record.values()) - sum(sizes.values()),
        'shared_categories': dict(sorted(categories.items())),
        'bundle_bytes': sum(p.stat().st_size for p in (corpus.parent / (corpus.name + '-bundle')).rglob('*') if p.is_file())
        if (corpus.parent / (corpus.name + '-bundle')).exists() else None,
    }, indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
