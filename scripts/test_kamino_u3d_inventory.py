#!/usr/bin/env python3
"""Inventory regression tests; none is a production replay test."""
import copy
import unittest

import kamino_u3d_inventory as inventory


class InventoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.inputs = inventory.frozen_inputs()
        cls.headers = inventory.captured_headers()

    def derive(self, inputs=None):
        return inventory.derive(*(inputs or self.inputs), self.headers)

    def test_every_message_and_programdata_account_has_a_row(self):
        result = self.derive()
        self.assertEqual([r['address'] for r in result['account_rows'][:23]],
                         [r['address'] for r in self.inputs[0]['transaction']['account_keys']])
        self.assertEqual(len(result['account_rows']), 27)
        self.assertEqual(len(result['message_resolution_inputs']), 1)

    def test_unknown_state_is_not_promoted_by_metadata_or_screen(self):
        result = self.derive()
        missing = [r for r in result['account_rows'] if r['historical_bytes_still_required']]
        self.assertEqual(len(missing), 16)
        self.assertTrue(all(r['owner'] is None and not r['historically_acquired'] for r in missing))
        ata = next(r for r in missing if 'associated-token-account' in [x['role'] for x in r['roles']])
        self.assertIn('token_account', ata['classes'])
        self.assertIsNotNone(ata['token_metadata'])
        self.assertFalse(result['all_historical_inputs_complete'])
        self.assertFalse(result['runtime_dependency_closure_proven'])
        self.assertFalse(result['production_replay_eligible'])

    def test_scope_aliases_are_ordered_and_do_not_invent_four_oracles(self):
        result = self.derive()
        self.assertEqual(result['scope_ordered_remaining_accounts'], [inventory.SCOPE] * 4)
        scope = next(r for r in result['account_rows'] if r['address'] == inventory.SCOPE)
        self.assertEqual([r['account_position'] for r in scope['roles']], [4, 5, 6, 7])
        self.assertEqual(sum('oracle_state' in r['classes'] for r in result['account_rows']), 3)

    def test_instruction_order_and_ata_membership_cannot_change(self):
        for mutation in ('reorder_scope', 'drop_ata'):
            inputs = copy.deepcopy(self.inputs)
            instructions = inputs[0]['transaction']['instructions']
            if mutation == 'reorder_scope':
                instructions[0], instructions[2] = instructions[2], instructions[0]
            else:
                instructions.pop(1)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError):
                self.derive(inputs)

    def test_native_account_order_is_preserved(self):
        inputs = copy.deepcopy(self.inputs)
        keys = inputs[0]['transaction']['account_keys']
        keys[0], keys[1] = keys[1], keys[0]
        with self.assertRaisesRegex(ValueError, 'native message key order'):
            self.derive(inputs)

    def test_screen_must_cover_every_key_and_reject_any_conflict(self):
        for mutation in ('omit_account', 'add_conflict', 'wrong_slot'):
            inputs = copy.deepcopy(self.inputs)
            screen = inputs[1]['slot_screening']
            if mutation == 'omit_account':
                screen['required_accounts'].pop()
            elif mutation == 'add_conflict':
                screen['conflicts'].append({'account': inventory.SCOPE, 'position': 'after'})
            else:
                screen['slot'] += 1
            with self.subTest(mutation=mutation), self.assertRaisesRegex(ValueError, 'same-slot screen'):
                self.derive(inputs)

    def test_failed_original_is_rejected(self):
        inputs = copy.deepcopy(self.inputs)
        inputs[0]['transaction']['success'] = False
        with self.assertRaisesRegex(ValueError, 'successful T1'):
            self.derive(inputs)

    def test_boundary_distinguishes_lut_sysvar_and_state(self):
        result = self.derive()
        for row in result['account_rows']:
            if row['address'] == inventory.INSTRUCTIONS:
                self.assertTrue(row['runtime_provided'])
                self.assertIsNone(row['requested_pre_slot'])
            else:
                self.assertEqual(row['requested_pre_slot'], inventory.SLOT - 1)
        self.assertEqual(result['message_resolution_inputs'][0]['requested_slot'], inventory.SLOT)


if __name__ == '__main__':
    unittest.main()
