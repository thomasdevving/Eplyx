#!/usr/bin/env python3
"""Scan retained acquisition/report material without printing matched values."""
import json
import gzip
import re
import tarfile
from pathlib import Path
from urllib.parse import urlsplit

import kamino_u3b_lut as lut

OUTPUT = lut.REPO / 'docs/examples/phase-u3f-validation/security-scan.json'


def check(value, allow_public_demo=False):
    if isinstance(value, dict):
        for key, item in value.items():
            if key == 'source_url' and isinstance(item, str) and item.startswith('https://raw.githubusercontent.com/Kamino-Finance/scope/fe5352366a7215dda5c6f7b867a6bb5929d52c94/'):
                continue  # Public pinned source attribution, not an RPC endpoint.
            check(item, allow_public_demo)
    elif isinstance(value, list):
        for item in value:
            check(item, allow_public_demo)
    elif isinstance(value, str):
        for url in re.findall(r'https?://[^\s"<>]+', value):
            parsed = urlsplit(url)
            lut.require(not parsed.username and not parsed.password, 'credential URL found; value withheld')
            if 'alchemy.com' in (parsed.hostname or '') or 'helius' in (parsed.hostname or ''):
                public_demo = allow_public_demo and parsed.hostname == 'solana-mainnet.g.alchemy.com' and parsed.path.split('/') == ['', 'v2', 'docs-demo']
                lut.require(not parsed.query and (parsed.path in ('', '/') or public_demo), 'full provider endpoint found; value withheld')
        lut.require(not re.search(r'(?i)(?:Bearer\s+[A-Za-z0-9_.-]{16,}|(?:sk_live|sk-proj|ghp|github_pat)_[A-Za-z0-9_-]{12,})', value), 'credential-like material found; value withheld')


def main():
    files, archive_members = [], 0
    for root in sorted((lut.REPO / 'docs/examples').glob('phase-u3*')):
        for path in sorted(root.rglob('*')):
            if not path.is_file() or path == OUTPUT:
                continue
            content = path.read_bytes()
            legacy_public_demo = path == lut.REPO / 'docs/examples/phase-u3-before/census-source.py'
            if legacy_public_demo:
                lut.require(lut.sha(content) == '35719945da5cf9ffd37a6dbaafb1cef647d9844b7af8f4a761805c5aef7475c0', 'frozen public-configuration source changed')
            if path.name.endswith('.tar.gz'):
                with tarfile.open(path) as archive:
                    for member in archive:
                        if member.isfile():
                            check(archive.extractfile(member).read().decode('utf8', errors='replace'))
                            archive_members += 1
                files.append({'path': str(path.relative_to(lut.REPO)), 'sha256': lut.sha(content)})
                continue
            if path.name.endswith('.json.gz'):
                content = gzip.decompress(content)
            if path.suffix in ('.json', '.body', '.gz'):
                try:
                    value = json.loads(content)
                except (ValueError, UnicodeError):
                    value = content.decode('utf8', errors='replace')
                if path.name == 'receipt.json':
                    lut.baseline.hygiene(value)
                    for attempt in value.get('attempts', []):
                        lut.require('authorization' not in attempt['response_headers'] and 'set-cookie' not in attempt['response_headers'], 'unsafe retained header')
                check(value, legacy_public_demo)
            else:
                check(content.decode('utf8', errors='replace'), legacy_public_demo)
            files.append({'path': str(path.relative_to(lut.REPO)), 'sha256': lut.sha(content)})
    doc = lut.REPO / 'docs/phase-u3f-runtime-minimality.md'
    if doc.exists():
        check(doc.read_text())
        files.append({'path': str(doc.relative_to(lut.REPO)), 'sha256': lut.sha(doc.read_bytes())})
    for pattern in ('*u3f*.py', 'historical_transport.py', 'test_historical_transport.py'):
        for path in sorted((lut.REPO / 'scripts').glob(pattern)):
            check(path.read_text())
    OUTPUT.write_bytes(lut.canonical({'passed': True, 'files_scanned': len(files), 'archive_members_scanned': archive_members, 'files': files,
                                    'new_acquisition_provider_paths_queries_userinfo_retained': False,
                                    'frozen_public_demo_source_exception': 'phase-u3-before/census-source.py, exact frozen SHA-256 checked; public demo configuration is not a private credential and immutable source was preserved',
                                    'authorization_cookie_headers_retained': False,
                                    'qualification': 'retained evidence checked; synthetic sentinel credentials in negative-test source are not real credentials',
                                    'private_credentials_read_or_used': False}))
    print(json.dumps({'security_scan_passed': True, 'files_scanned': len(files)}))


if __name__ == '__main__':
    main()
