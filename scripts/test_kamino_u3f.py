#!/usr/bin/env python3
"""Executable native-v0 provenance and strict fidelity regressions (offline)."""
import base64
import copy
import importlib.util
import json
import subprocess
import unittest
from unittest.mock import patch
import kamino_u3f_fidelity as fidelity
f = fidelity.f
lut = f.lut
spec = importlib.util.spec_from_file_location('replay', f.lut.REPO / 'scripts/replay-kamino-u3f.py')
replay = importlib.util.module_from_spec(spec)
spec.loader.exec_module(replay)


class NativeEnvelopeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with patch.object(f.s.transport.CurlArchive, 'once', side_effect=AssertionError('network forbidden')):
            cls.payload, cls.manifest = f.prepare()
        cls.pre, cls.post = fidelity.references(cls.payload)
        cls.baseline = lut.read(f.ROOT / 'phase-u3f-materiality-attempt-1/empty.json')

    def execute(self, payload):
        payload['variant'] = 'empty'
        return subprocess.run([str(lut.REPO / 'target/debug/examples/execute_envelope_v0')], input=lut.canonical(payload), capture_output=True, timeout=180, cwd=lut.REPO)

    def rejected(self, change, expected):
        payload = copy.deepcopy(self.payload)
        change(payload)
        process = self.execute(payload)
        self.assertNotEqual(process.returncode, 0)
        self.assertIn(expected, process.stderr.decode())

    def test_original_executes_and_actual_prestate_is_historical(self):
        process = self.execute(copy.deepcopy(self.payload))
        self.assertEqual(process.returncode, 0, process.stderr.decode())
        result = json.loads(process.stdout)
        replay.verify_pre(self.payload, result)
        self.assertEqual(result['evidence'], self.baseline['evidence'])
        self.assertEqual(fidelity.compare(self.payload, result, self.post)['fidelity'], 'matched')

    def test_three_distinct_slothashes_have_identical_success(self):
        runs = [lut.read(f.ROOT / f'phase-u3f-materiality-attempt-1/{v}.json') for v in ('default','empty','different')]
        self.assertEqual(len({json.dumps(r['slot_hashes']) for r in runs}), 3)
        self.assertTrue(all(r['evidence'] == runs[0]['evidence'] and r['evidence']['success'] for r in runs))

    def test_missing_seed_rejected(self):
        self.rejected(lambda p: p['seeds'].remove(next(a for a in p['seeds'] if a['kind']=='ordinary')), 'required pre-state missing')

    def test_duplicate_seed_rejected(self):
        self.rejected(lambda p: p['seeds'].append(p['seeds'][0]), 'duplicate seed')

    def test_post_context_seed_rejected(self):
        self.rejected(lambda p: next(a for a in p['seeds'] if a['kind']=='ordinary').update(slot=f.s.inventory.SLOT), 'post state cannot seed')

    def test_current_clock_rejected(self):
        self.rejected(lambda p: next(a for a in p['seeds'] if a['kind']=='runtime').update(slot=f.s.inventory.SLOT+100), 'post state cannot seed')

    def test_post_scope_bytes_cannot_masquerade_as_pre(self):
        address = '3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH'
        def change(p):
            seed = next(a for a in p['seeds'] if a['address']==address)
            seed['account'] = self.post[address]
            seed['data_sha256'] = lut.sha(base64.b64decode(seed['account']['data'][0]))
        self.rejected(change, 'historical seed differs')

    def test_wrong_historical_elf_rejected(self):
        def change(p):
            seed = next(a for a in p['seeds'] if a['kind']=='program' and len(base64.b64decode(a['account']['data'][0]))>10000)
            data = bytearray(base64.b64decode(seed['account']['data'][0])); data[-1] ^= 1
            seed['account']['data'][0] = base64.b64encode(data).decode();seed['data_sha256'] = lut.sha(data)
        self.rejected(change, 'historical ELF hash differs')

    def test_wrong_lut_seed_rejected(self):
        def change(p):
            seed = next(a for a in p['seeds'] if a['kind']=='lut')
            data = bytearray(base64.b64decode(seed['account']['data'][0]));data[-1] ^= 1
            seed['account']['data'][0] = base64.b64encode(data).decode();seed['data_sha256'] = lut.sha(data)
        self.rejected(change, 'runtime LUT bytes differ')

    def test_success_is_not_fidelity(self):
        result = copy.deepcopy(self.baseline)
        account = next(a for a in result['evidence']['post_accounts'].values() if a and a['data'][0])
        data = bytearray(base64.b64decode(account['data'][0]));data[-1] ^= 1
        account['data'][0] = base64.b64encode(data).decode()
        self.assertEqual(fidelity.compare(self.payload,result,self.post)['fidelity'],'mismatched')

    def test_account_fields_all_strict(self):
        for field, value in [('owner','11111111111111111111111111111111'),('lamports',1),('executable',True),('rentEpoch',7)]:
            with self.subTest(field=field):
                result=copy.deepcopy(self.baseline)
                account=next(a for a in result['evidence']['post_accounts'].values() if a and a[field]!=value)
                account[field]=value
                self.assertEqual(fidelity.compare(self.payload,result,self.post)['fidelity'],'mismatched')

    def test_missing_watch_rejected(self):
        result=copy.deepcopy(self.baseline);result['evidence']['post_accounts'].pop(self.payload['watch'][0])
        with self.assertRaisesRegex(ValueError,'complete watched'):fidelity.compare(self.payload,result,self.post)

    def test_changed_native_message_rejected(self):
        result=copy.deepcopy(self.baseline);result['evidence']['native_message']['instructions'].pop()
        with self.assertRaisesRegex(ValueError,'native message'):fidelity.compare(self.payload,result,self.post)

    def test_outcome_metadata_strict(self):
        for field,value in [('fee',1),('logs',[]),('inner_instructions',[]),('return_data',{'program':'11111111111111111111111111111111','data_base64':'AQ=='})]:
            with self.subTest(field=field):
                result=copy.deepcopy(self.baseline);result['evidence'][field]=value
                self.assertEqual(fidelity.compare(self.payload,result,self.post)['fidelity'],'mismatched')

    def test_scope_control_has_no_fidelity_admission(self):
        control=lut.read(f.ROOT/'phase-u3f-causal-attempt-1/skip-scope.json')
        with self.assertRaisesRegex(ValueError,'reduced envelope'):fidelity.compare(self.payload,control,self.post)
        changed=[k for k,v in control['evidence']['post_accounts'].items() if v!=self.baseline['evidence']['post_accounts'][k]]
        self.assertEqual(changed,['3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH'])

    def test_no_semantics_when_fidelity_fails(self):
        with patch.object(f,'prepare',return_value=(self.payload,self.manifest)),patch.object(fidelity,'compare',return_value={'fidelity':'mismatched'}),patch.object(fidelity.subprocess,'run',side_effect=AssertionError('semantic evaluation before fidelity')):
            with self.assertRaisesRegex(ValueError,'no semantics'):fidelity.derive()

    def test_actual_prestate_audit_rejects_post_seed(self):
        actual = {'seeded_pre_accounts': {s['address']: s['account'] for s in self.payload['seeds']}}
        address='3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH'
        actual['seeded_pre_accounts'][address] = self.post[address]
        with self.assertRaisesRegex(ValueError,'actual VM pre-state'):replay.verify_pre(self.payload,actual)


if __name__=='__main__':unittest.main(verbosity=2)
