#!/usr/bin/env python3
"""Real partial-capture checks and explicitly synthetic negative controls."""
import base64
import copy
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

import kamino_u3d_state as state

BINARY_ROOT = state.lut.REPO / 'docs/examples/phase-u3d2-binaries'
STATE_ROOT = state.lut.REPO / 'docs/examples/phase-u3d2-state'


class StateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.rows = state.targets(BINARY_ROOT)
        cls.row, cls.binaries, _ = cls.rows[0]
        cls.plan = state.state_plan(cls.row, cls.binaries)
        receipt = state.lut.read(STATE_ROOT / 'receipt.json')
        cls.successes = [r for r in receipt['requests'] if r['method'] == 'getAccountInfo' and not r['transport_error']]
        cls.response = state.lut.read(STATE_ROOT / cls.successes[0]['response_file'])['result']

    def test_real_partial_capture_reproduces_without_transport(self):
        with patch.object(state.transport.RawArchive, 'call', side_effect=AssertionError('network forbidden')):
            result = state.verify(STATE_ROOT, BINARY_ROOT)
        self.assertEqual([len(t['acquired']) for t in result['targets']], [2, 0, 0, 0])
        self.assertFalse(any(t['raw_state_capture_complete'] for t in result['targets']))
        self.assertFalse(result['production_replay_eligible'])

    def test_exact_t1_boundary_capture_and_scope_hashes(self):
        self.assertEqual(self.plan[0]['address'], '3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH')
        self.assertEqual([r['params'][1]['slot'] for r in self.successes], [448195165, 448195166])
        expected = ['c3fe3908e3c8778f0b99f6c83def59cc07c95ca7468e8f4585c5fe3d0caf6884',
                    '7b412449f556f471ca5623ec1731f03366675455a1509a349388e89e90d231bc']
        for item, request, digest in zip(self.plan, self.successes, expected):
            response = state.lut.read(STATE_ROOT / request['response_file'])['result']
            self.assertEqual(state.account_check(self.row, item, response)['data_sha256'], digest)

    def test_current_or_post_scope_state_cannot_seed_pre_state(self):
        for slot in (448195166, 999999999):
            response = copy.deepcopy(self.response)
            response['context']['slot'] = slot
            with self.subTest(slot=slot), self.assertRaisesRegex(ValueError, 'wrong_historical_context'):
                state.account_check(self.row, self.plan[0], response)

    def test_scope_owner_and_validator_balance_must_match(self):
        for key, value, reason in [('owner', state.inventory.SYSTEM, 'wrong_scope_state_owner'),
                                   ('lamports', self.response['value']['lamports'] + 1, 'historical_lamports')]:
            response = copy.deepcopy(self.response)
            response['value'][key] = value
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, reason):
                state.account_check(self.row, self.plan[0], response)

    def test_missing_or_truncated_oracle_state_is_rejected(self):
        response = copy.deepcopy(self.response)
        response['value'] = None
        with self.assertRaisesRegex(ValueError, 'historical_account_missing'):
            state.account_check(self.row, self.plan[0], response)
        response = copy.deepcopy(self.response)
        response['value']['data'][0] = base64.b64encode(b'partial').decode()
        response['value']['space'] = 28712
        with self.assertRaisesRegex(ValueError, 'partial_account_data'):
            state.account_check(self.row, self.plan[0], response)

    def test_same_slot_ambiguity_and_missing_coverage_rejected(self):
        for change in ('conflict', 'omit', 'slot'):
            binaries = copy.deepcopy(self.binaries)
            screen = binaries['slot_screening']
            if change == 'conflict':
                screen['conflicts'].append({'account': self.plan[0]['address'], 'position': 'before'})
            elif change == 'omit':
                screen['required_accounts'].pop()
            else:
                screen['slot'] += 1
            with self.subTest(change=change), self.assertRaisesRegex(ValueError, 'same_slot_screen'):
                state.state_plan(self.row, binaries)

    def test_unproven_binary_set_cannot_start_state_pipeline(self):
        for row, binaries, _ in self.rows[1:]:
            with self.assertRaisesRegex(ValueError, 'historical_binary_set_incomplete'):
                state.state_plan(row, binaries)
        binaries = copy.deepcopy(self.binaries)
        binaries['pre_slot'] += 1
        with self.assertRaisesRegex(ValueError, 'historical_binary_context_mismatch'):
            state.state_plan(self.row, binaries)

    def test_wrong_scope_or_klend_binary_bytes_rejected(self):
        spec = importlib.util.spec_from_file_location('u3d2_binary_check', state.lut.REPO / 'scripts/rebuild-kamino-u3d2.py')
        rebuild = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(rebuild)
        original = rebuild.dependencies.replay_resolver
        for program_id in (state.envelope.SCOPE, state.inventory.KLEND):
            def tamper(row, cache, output, used):
                result = original(row, cache, output, used)
                program = next(p for p in result['programs'] if p['program_id'] == program_id)
                path = output / program['binary_file']
                data = bytearray(path.read_bytes())
                data[-1] ^= 1
                path.write_bytes(data)
                return result
            with self.subTest(program_id=program_id), patch.object(rebuild.dependencies, 'replay_resolver', tamper):
                with self.assertRaisesRegex(ValueError, 'historical binary image differs'):
                    rebuild.verify_binaries(BINARY_ROOT)

    def test_every_planned_state_read_has_exact_historical_slot(self):
        self.assertEqual(len(self.plan), 32)
        self.assertEqual(len({r['address'] for r in self.plan}), 16)
        for item in self.plan:
            self.assertEqual(item['params'], [item['address'], {'encoding': 'base64', 'commitment': 'finalized', 'slot': item['slot']}])
            self.assertEqual(item['slot'], 448195165 if item['boundary'] == 'pre' else 448195166)
        self.assertNotIn(state.INSTRUCTIONS, {r['address'] for r in self.plan})

    def test_transport_stops_at_bounded_limit_and_retains_empty_bodies(self):
        class FailingArchive:
            identity = 'https://solana-mainnet.g.alchemy.com'
            calls = []
            def __init__(self, *_args):
                pass
            def call(self, method, params):
                self.calls.append((method, params))
                if method == 'getGenesisHash':
                    value = {'result': state.lut.GENESIS}
                    return state.lut.canonical(value), value, None
                return b'', None, 'invalid_json_response'
        with tempfile.TemporaryDirectory() as tmp, patch.dict(state.os.environ, {'SOLANA_ARCHIVE_RPC_URL': 'https://example.invalid'}), patch.object(state.transport, 'RawArchive', FailingArchive):
            root = Path(tmp) / 'attempt'
            result = state.capture(root, BINARY_ROOT)
            receipt = state.lut.read(root / 'receipt.json')
            self.assertEqual(len(FailingArchive.calls), 3)
            self.assertEqual([r['attempt'] for r in receipt['requests']], [1, 1, 2])
            self.assertEqual([len(t['acquired']) for t in result['targets']], [0, 0, 0, 0])
            for request in receipt['requests'][1:]:
                self.assertEqual((root / request['response_file']).read_bytes(), b'')
            with self.assertRaisesRegex(ValueError, 'immutable directory'):
                state.capture(root, BINARY_ROOT)

    def test_contradictory_context_is_not_retried(self):
        outer = self
        class WrongContextArchive:
            identity = 'https://solana-mainnet.g.alchemy.com'
            calls = []
            def __init__(self, *_args):
                pass
            def call(self, method, params):
                self.calls.append((method, params))
                value = {'result': state.lut.GENESIS if method == 'getGenesisHash' else copy.deepcopy(outer.response)}
                if method != 'getGenesisHash':
                    value['result']['context']['slot'] += 1
                return state.lut.canonical(value), value, None
        with tempfile.TemporaryDirectory() as tmp, patch.dict(state.os.environ, {'SOLANA_ARCHIVE_RPC_URL': 'https://example.invalid'}), patch.object(state.transport, 'RawArchive', WrongContextArchive):
            result = state.capture(Path(tmp) / 'attempt', BINARY_ROOT)
            self.assertEqual(len(WrongContextArchive.calls), 2)
            self.assertEqual(result['targets'][0]['failure']['reason'], 'wrong_historical_context')

    def test_raw_hash_mismatch_cannot_be_ignored(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'capture'
            shutil.copytree(STATE_ROOT, root)
            ref = self.successes[0]['response_file']
            with (root / ref).open('ab') as f:
                f.write(b' ')
            with self.assertRaisesRegex(ValueError, 'raw response hash or length mismatch'):
                state.verify(root, BINARY_ROOT)

    def test_uncaptured_context_has_no_fallback(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / 'capture'
            shutil.copytree(STATE_ROOT, root)
            receipt = state.lut.read(root / 'receipt.json')
            key = receipt['requests'][-1]['request_id']
            dropped = [r for r in receipt['requests'] if r['request_id'] == key]
            receipt['requests'] = [r for r in receipt['requests'] if r['request_id'] != key]
            for request in dropped:
                (root / request['response_file']).unlink()
                del receipt['raw_artifact_hashes'][request['response_file']]
            state.write(root / 'receipt.json', receipt)
            with self.assertRaisesRegex(RuntimeError, 'uncaptured request; no current-state fallback'):
                state.verify(root, BINARY_ROOT)

    def test_false_complete_or_execution_flag_is_rejected(self):
        for flag in ('raw_state_capture_complete', 'complete_historical_state_proven', 'runtime_executed'):
            with self.subTest(flag=flag), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp) / 'capture'
                shutil.copytree(STATE_ROOT, root)
                result = state.lut.read(root / 'result.json')
                result['targets'][0][flag] = True
                state.write(root / 'result.json', result)
                with self.assertRaisesRegex(ValueError, 'derived state result differs'):
                    state.verify(root, BINARY_ROOT)

    def test_synthetic_ata_bytes_must_match_wallet_mint_and_initialized_state(self):
        tx = self.row['transaction']
        ata = tx['instructions'][1]['accounts']
        item = next(i for i in self.plan if i['address'] == ata[1]['address'] and i['boundary'] == 'pre')
        index = next(i for i, k in enumerate(tx['account_keys']) if k['address'] == item['address'])
        token = next(t for t in tx['pre_token_balances'] if t['account_index'] == index)
        data = bytearray(165)
        data[:32] = state.lut.baseline.b58decode(ata[3]['address'])
        data[32:64] = state.lut.baseline.b58decode(ata[2]['address'])
        data[64:72] = token['amount'].to_bytes(8, 'little')
        data[108] = 1
        response = {'context': {'slot': item['slot']}, 'value': {'owner': ata[5]['address'], 'lamports': tx['pre_balances'][index],
                    'rentEpoch': 0, 'executable': False, 'data': [base64.b64encode(data).decode(), 'base64']}}
        state.account_check(self.row, item, response)
        for offset in (0, 32, 108):
            changed = bytearray(data)
            changed[offset] ^= 1
            response['value']['data'][0] = base64.b64encode(changed).decode()
            with self.subTest(offset=offset), self.assertRaises(ValueError):
                state.account_check(self.row, item, response)


if __name__ == '__main__':
    unittest.main()
