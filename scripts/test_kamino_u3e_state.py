#!/usr/bin/env python3
"""Historical captured bytes plus labeled synthetic boundary negative controls."""
import base64
import copy
import unittest
from unittest.mock import patch

import kamino_u3e_state as s


class StateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.row, cls.binaries, cls.derived = s.inputs()
        cls.plan = cls.derived['planned_boundary_requests']
        root = s.ROOT / 'phase-u3e-mapping-probe'
        attempt = s.lut.read(root / 'receipt.json')['attempts'][-1]
        cls.mapping = s.lut.read(root / attempt['body_file'])['result']
        cls.item = next(i for i in cls.plan if i['address'] == cls.row['transaction']['instructions'][0]['accounts'][1]['address'] and i['boundary'] == 'pre')

    def test_inventory_includes_exact_message_programdata_and_aliases(self):
        self.assertEqual(len(self.derived['account_rows']), 27)
        self.assertEqual(len(self.plan), 32)
        self.assertEqual(len(self.derived['message_resolution_inputs']), 1)
        self.assertEqual(self.derived['scope_ordered_remaining_accounts'], [s.inventory.SCOPE] * 4)
        self.assertEqual(sum(a['observed_invocation'] for a in self.derived['account_rows'] if a['message_index'] is not None), 6)

    def test_real_mapping_owner_discriminator_and_hash(self):
        facts = s.validate(self.row, self.derived, self.item, self.mapping)
        self.assertEqual(facts['data_bytes'], 29704)
        self.assertEqual(facts['anchor_type'], 'OracleMappings')
        self.assertFalse(facts['execution_seed_admitted'])

    def test_missing_required_account_rejected(self):
        value = copy.deepcopy(self.mapping)
        value['value'] = None
        with self.assertRaises(ValueError):
            s.validate(self.row, self.derived, self.item, value)

    def test_wrong_historical_owner_rejected(self):
        value = copy.deepcopy(self.mapping)
        value['value']['owner'] = s.inventory.SYSTEM
        with self.assertRaises(ValueError):
            s.validate(self.row, self.derived, self.item, value)

    def test_wrong_known_pre_hash_rejected(self):
        value = copy.deepcopy(self.mapping)
        data = bytearray(base64.b64decode(value['value']['data'][0]))
        data[-1] ^= 1
        value['value']['data'][0] = base64.b64encode(data).decode()
        with self.assertRaisesRegex(ValueError, 'prior exact'):
            s.validate(self.row, self.derived, self.item, value)

    def test_post_state_cannot_seed_pre_boundary(self):
        value = copy.deepcopy(self.mapping)
        value['context']['slot'] = s.inventory.SLOT
        with self.assertRaisesRegex(ValueError, 'context'):
            s.validate(self.row, self.derived, self.item, value)

    def test_current_oracle_context_rejected(self):
        value = copy.deepcopy(self.mapping)
        value['context']['slot'] = 500000000
        with self.assertRaisesRegex(ValueError, 'context'):
            s.validate(self.row, self.derived, self.item, value)

    def test_same_slot_conflict_rejected(self):
        binaries = copy.deepcopy(self.binaries)
        binaries['slot_screening']['conflicts'] = [{'synthetic_interference': True}]
        with self.assertRaisesRegex(ValueError, 'same_slot'):
            s.state.state_plan(self.row, binaries)

    def test_mapping_omission_rejected_before_requests(self):
        derived = copy.deepcopy(self.derived)
        derived['planned_boundary_requests'] = [i for i in self.plan if i['address'] != self.item['address']]
        # Synthetic successful callback permits the mutation to reach a named assertion.
        with self.assertRaisesRegex(ValueError, 'complete ordered boundary'):
            s.evaluate(self.row, derived, lambda *args: (None, {}, None))

    def test_ata_metadata_never_substitutes_missing_bytes(self):
        address = self.row['transaction']['instructions'][1]['accounts'][1]['address']
        item = next(i for i in self.plan if i['address'] == address and i['boundary'] == 'pre')
        with self.assertRaises(ValueError):
            s.validate(self.row, self.derived, item, {'context': {'slot': item['slot']}, 'value': None})

    def test_zero_lamport_authority_absence_is_explicit(self):
        permitted = [i for i in self.plan if s.permits_absence(self.row, i)]
        self.assertEqual(len(permitted), 2)
        for item in permitted:
            self.assertFalse(s.validate(self.row, self.derived, item, {'context': {'slot': item['slot']}, 'value': None})['present'])
        self.assertFalse(s.permits_absence(self.row, self.item))

    def test_failure_stops_later_acquisition_and_runtime_claims(self):
        seen = []
        def call(method, params, check, allow):
            seen.append(method)
            return (s.lut.GENESIS, {}, None) if method == 'getGenesisHash' else (None, None, 'timeout')
        result = s.evaluate(self.row, self.derived, call)
        self.assertEqual(len(seen), 2)
        self.assertFalse(result['historical_state_complete'])
        self.assertFalse(result['runtime_executed'])


if __name__ == '__main__':
    unittest.main()
