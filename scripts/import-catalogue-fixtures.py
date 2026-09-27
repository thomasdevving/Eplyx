#!/usr/bin/env python3
"""Import exact saved product-page captures; never contact a provider."""
import argparse,hashlib,json,pathlib
p=argparse.ArgumentParser();p.add_argument('--sta',required=True,type=pathlib.Path);args=p.parse_args()
root=pathlib.Path(__file__).resolve().parent.parent/'fixtures/catalogue'
for item in json.loads((root/'provenance.json').read_text())['files']:
    data=(args.sta/item['source']).read_bytes()
    if len(data)!=item['bytes'] or hashlib.sha256(data).hexdigest()!=item['sha256']:raise SystemExit('Catalogue source bytes do not match pinned reference')
    output=root/item['file']
    if output.exists():
        if output.read_bytes()!=data:raise SystemExit('Existing catalogue bytes differ; refusing overwrite')
    else:output.write_bytes(data)
print('Exact saved catalogue inputs verified. No network used.')
