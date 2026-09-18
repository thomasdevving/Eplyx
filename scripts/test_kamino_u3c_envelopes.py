#!/usr/bin/env python3
"""Production response integrity and explicit negative controls, offline only."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
import kamino_u3b_lut as lut
import kamino_u3c_dependencies as deps
import kamino_u3c_envelope as envelope


class Integrity(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix='eplyx-u3c-tests-')
        cls.root = Path(cls.temp.name)
        deps.unpack(deps.EVIDENCE, cls.root)
        cls.receipt = lut.read(cls.root / 'receipt.json')
        cls.bytes = (cls.root / 'receipt.json').read_bytes()

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def tearDown(self):
        (self.root / 'receipt.json').write_bytes(self.bytes)

    def changed(self, edit):
        value = copy.deepcopy(self.receipt)
        edit(value)
        (self.root / 'receipt.json').write_bytes(lut.canonical(value))

    def test_actual_capture_complete_membership_and_failure_counts(self):
        r, primary, cache = deps.validate_capture(self.root)
        self.assertEqual((len(primary), len(cache)), (4, 103))
        self.assertEqual(sum(x['status'] == 'success' for x in r['requests']), 100)
        self.assertEqual(sum(x['status'] == 'failure' for x in r['requests']), 3)
        for x in r['requests']:
            if x['status'] == 'failure':
                self.assertEqual(x['failure_reason'], 'invalid_json_response')
                self.assertIsNone(x['response_file'])
                self.assertIsNone(x['response_sha256'])  # actual empty bodies

    def test_checkpoint_cannot_be_final_sample(self):
        self.changed(lambda r: r.update(complete=False))
        with self.assertRaisesRegex(ValueError, 'incomplete'):
            deps.validate_capture(self.root)

    def test_missing_target_cannot_be_backfilled(self):
        self.changed(lambda r: r['target_results'].pop())
        with self.assertRaisesRegex(ValueError, 'membership'):
            deps.validate_capture(self.root)

    def test_duplicate_request_cannot_hide_denominator(self):
        self.changed(lambda r: r['requests'].append(r['requests'][0]))
        with self.assertRaisesRegex(ValueError, 'duplicate'):
            deps.validate_capture(self.root)

    def test_current_state_query_cannot_be_evidence(self):
        def edit(r):
            x = next(x for x in r['requests'] if x['method'] == 'getAccountInfo')
            del x['params'][1]['slot']
            x['request_id'] = lut.sha(lut.canonical([x['method'], x['params']]))
        self.changed(edit)
        with self.assertRaisesRegex(ValueError, 'current or intermediate'):
            deps.validate_capture(self.root)

    def test_raw_binary_tampering_rejected(self):
        ref = next(n for n in self.receipt['raw_artifact_hashes'] if n.endswith('.so'))
        p = self.root / ref
        body = p.read_bytes()
        try:
            p.write_bytes(body + b'negative control')
            with self.assertRaisesRegex(ValueError, 'hash differs'):
                deps.validate_capture(self.root)
        finally:
            p.write_bytes(body)

    def test_raw_rpc_tampering_rejected(self):
        ref = next(n for n in self.receipt['raw_artifact_hashes'] if n.endswith('.body'))
        p = self.root / ref
        body = p.read_bytes()
        try:
            p.write_bytes(b'{}')
            with self.assertRaisesRegex(ValueError, 'hash differs'):
                deps.validate_capture(self.root)
        finally:
            p.write_bytes(body)

    def test_unknown_extra_artifact_rejected(self):
        p = self.root / 'unlisted.json'
        try:
            p.write_bytes(b'{}')
            with self.assertRaisesRegex(ValueError, 'membership'):
                deps.validate_capture(self.root)
        finally:
            p.unlink()

    def test_reverse_offline_resolver_preserves_all_canonical_bytes(self):
        normal, _ = deps.derive()
        reverse, _ = deps.derive(reverse=True)
        self.assertEqual(normal, reverse)
        for name, body in normal.items():
            self.assertEqual((deps.EVIDENCE / name).read_bytes(), body)
        rows = json.loads(normal['stage-table.json'])
        self.assertEqual(rows['primary_metrics']['C4_binary_capture_complete'], 1)
        self.assertTrue(all(r[f'C{i}'] == 'not_attempted' for r in rows['rows'] for i in range(5, 11)))

    def test_reverse_envelope_preserves_original_eight_proofs(self):
        self.assertEqual(envelope.derive(), envelope.derive(reverse=True))


if __name__ == '__main__':
    unittest.main()
