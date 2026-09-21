#!/usr/bin/env python3
"""Offline U3E revalidation; never starts an archive transport."""
import os
from pathlib import Path
import subprocess
import time
from unittest.mock import patch

import historical_transport as transport
import kamino_u3e as probe
import kamino_u3e_runtime as runtime

lut = transport.lut


def main():
    started = time.perf_counter()
    env = {k: v for k, v in os.environ.items() if not any(x in k.upper() for x in ('RPC', 'ARCHIVE', 'API_KEY', 'ALCHEMY', 'HELIUS'))}
    for command in (["bash", "scripts/rebuild-kamino-u3d-inventory.sh", "--verify"],
                    ["python3", "scripts/rebuild-kamino-u3d2.py"],
                    ["cargo", "build", "--offline", "-q", "-p", "eplyx-engine", "--example", "verify_envelope_state"]):
        subprocess.run(command, cwd=lut.REPO, env=env, check=True)
    freeze = lut.read(lut.REPO / 'docs/examples/phase-u3e-validation/preimplementation.json')
    for name, digest in freeze['prior_attempt_hashes'].items():
        lut.require(lut.sha((lut.REPO / name).read_bytes()) == digest, f'prior immutable attempt changed: {name}')
    with patch.object(transport.CurlArchive, 'once', side_effect=AssertionError('offline transport forbidden')):
        probe.probe(runtime.s.ROOT / 'phase-u3e-mapping-probe', verify=True)
        # Runtime derivation also revalidates the entire state capture, typed
        # layouts, relationship checks and the final screen from raw responses.
        result = runtime.run(runtime.s.ROOT / 'phase-u3e-runtime', verify=True)
    lut.require(result['failure']['reason'] == 'missing_account' and not result['runtime_executed'], 'terminal evidence differs')
    print(lut.canonical({'offline_revalidation_passed': True, 'provider_environment_removed': True,
                         'live_transport_forbidden': True, 'runtime_replayed': False,
                         'elapsed_seconds': time.perf_counter() - started}).decode())


if __name__ == '__main__':
    main()
