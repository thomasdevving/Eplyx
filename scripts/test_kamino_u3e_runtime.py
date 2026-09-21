#!/usr/bin/env python3
"""Captured runtime diagnostics and explicit negative controls; no VM execution."""
import base64
import copy
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import kamino_u3e_runtime as runtime
import test_historical_transport as controls

s = runtime.s


class RuntimeInputTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = s.ROOT / 'phase-u3e-runtime'
        cls.receipt = s.lut.read(cls.root / 'receipt.json')
        cls.clock = s.lut.read(cls.root / cls.receipt['attempts'][0]['body_file'])['result']
        cls.clock_item = runtime.plan()[0]

    def test_clock_all_five_fields_match_captured_context(self):
        fields = runtime.validate(self.clock_item, self.clock)['fields']
        self.assertEqual(fields, {'slot': 448195166, 'epoch': 1037, 'epoch_start_timestamp': 1789708003,
                                  'leader_schedule_epoch': 1038, 'unix_timestamp': 1789764316})

    def test_current_clock_or_owner_rejected(self):
        for fault in ('owner', 'slot', 'timestamp'):
            value = copy.deepcopy(self.clock)
            if fault == 'owner':
                value['value']['owner'] = s.inventory.SYSTEM
            else:
                data = bytearray(base64.b64decode(value['value']['data'][0]))
                data[0 if fault == 'slot' else 32] ^= 1
                value['value']['data'][0] = base64.b64encode(data).decode()
            with self.assertRaises(ValueError):
                runtime.validate(self.clock_item, value)

    def test_missing_slothashes_is_rpc_null_not_transport_empty(self):
        attempt = self.receipt['attempts'][-1]
        body = (self.root / attempt['body_file']).read_bytes()
        self.assertGreater(len(body), 0)
        failure, value = s.transport.classify(attempt['process_exit_code'], attempt['http_status'], body, attempt['response_headers'])
        self.assertIsNone(failure)
        self.assertIsNone(value['result']['value'])
        self.assertEqual(attempt['failure_class'], 'missing_account')

    def test_null_runtime_state_not_retried_or_declared_complete(self):
        item = runtime.plan()[-1]
        body = s.lut.canonical({'result': {'context': {'slot': s.inventory.SLOT}, 'value': None}})
        with tempfile.TemporaryDirectory() as tmp:
            client = s.transport.EvidenceClient(Path(tmp) / 'capture', s.transport.CurlArchive('https://archive.example'), s.lut.read(s.transport.POLICY_PATH), {}, lambda _: None)
            with patch.object(s.transport.subprocess, 'run', controls.wire(body)):
                _, facts, failure = client.call(item['method'], item['params'], lambda value: runtime.validate(item, value))
            self.assertEqual(len(client.receipt['attempts']), 1)
            self.assertIsNone(facts)
            self.assertEqual(failure, 'missing_account')
        result = s.lut.read(self.root / 'result.json')
        self.assertFalse(result['runtime_context_proven'])
        self.assertFalse(result['runtime_executed'])

    def test_epoch_schedule_consistent_with_clock(self):
        result = s.lut.read(self.root / 'result.json')
        fields = {i['name']: i['facts']['fields'] for i in result['acquired']}
        schedule, clock = fields['EpochSchedule'], fields['Clock']
        epoch = schedule['first_normal_epoch'] + (clock['slot'] - schedule['first_normal_slot']) // schedule['slots_per_epoch']
        self.assertEqual(epoch, clock['epoch'])


if __name__ == '__main__':
    unittest.main()
