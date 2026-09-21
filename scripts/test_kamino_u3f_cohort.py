#!/usr/bin/env python3
"""Independent cohort boundaries and native execution evidence controls."""
import copy
import unittest
from unittest.mock import patch
import kamino_u3f_cohort as c
f,lut=c.f,c.lut


class CohortTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with patch.object(c.t.CurlArchive,'once',side_effect=AssertionError('offline only')):
            cls.payload,cls.post,_=c.prepare('T2')
        cls.runs=[f.read_capture(f.ROOT/f'phase-u3f-T2-attempt-1/{v}.json') for v in ('default','empty','different')]

    def test_T2_three_full_native_runs_exact_fidelity(self):
        for result in self.runs:
            self.assertTrue(result['evidence']['success'])
            self.assertEqual(result['evidence'],self.runs[0]['evidence'])
            self.assertEqual(result['evidence']['native_message']['instructions'][0][0],10)
            self.assertEqual(c.b.fidelity.compare(self.payload,result,self.post)['fidelity'],'matched')

    def test_T2_scope_changes_reserve_and_obligation(self):
        control=f.read_capture(f.ROOT/'phase-u3f-T2-causal-attempt-1/skip-scope.json')
        self.assertEqual(control['seeded_pre_accounts'],self.runs[0]['seeded_pre_accounts'])
        changed={k for k in self.payload['watch'] if control['evidence']['post_accounts'][k]!=self.runs[0]['evidence']['post_accounts'][k]}
        self.assertEqual(changed,{'3t4JZcueEzTbVP6kLxXrL3VpWx45jDer4eqysweBchNH','Adnj8BDHD9kdvzsSmRHB9ayS12kY1qfXzQCFxBcpq3t1','d4A2prbA2whesmvHaL88BH6Ewn5N4bTSU2Ze8P6Bc4Q'})
        with self.assertRaisesRegex(ValueError,'reduced envelope'):c.b.fidelity.compare(self.payload,control,self.post)

    def test_T3_no_retry_bound_reset_or_runtime_claim(self):
        receipt=lut.read(f.ROOT/'phase-u3f-cohort-state/receipt.json')
        attempts=[a for a in receipt['attempts'] if a['requested_slot']==448194462 and a['account']=='SysvarRent111111111111111111111111111111111']
        self.assertEqual([a['attempt'] for a in attempts],[1,2,3,4])
        self.assertEqual({a['failure_class'] for a in attempts},{'rate_limited'})
        with self.assertRaises(KeyError):c.prepare('T3')

    def test_T4_interference_rederived_from_raw_block(self):
        row,result=c.s.row_for('T4');cache=c.binary_cache()
        block=next(v for v in cache.values() if v.get('method')=='getBlock' and v['params'][0]==row['transaction']['slot'])
        with self.assertRaises(ValueError):
            f.runtime.proof.run_verifier({'mode':'screen','slot':row['transaction']['slot'],'signature':row['transaction']['signature'],'required_accounts':result['slot_screening']['required_accounts'],'block':{'params':block['params'],'result':block['result']}})
        forged=copy.deepcopy(result);forged['slot_screening']['conflicts']=[];forged['failure']=None
        with self.assertRaisesRegex(ValueError,'revalidation differs'):c.prove_binaries(row,forged,cache)

    def test_seven_existing_subjects_only(self):
        first=lut.read(f.VALIDATION/'T1-semantics.json');second=lut.read(f.VALIDATION/'T2-semantics.json')
        subjects={a['subject'] for r in (first,second) for a in r['subjects'] if a['domain']=='economic'}
        self.assertEqual(subjects,{'liquidity_deposited','reserve_liquidity_received','obligation_collateral_deposited','liquidity_borrowed','reserve_liquidity_drawn','origination_fee','debt_increased'})
        self.assertEqual(first['baseline_self_findings'],[]);self.assertEqual(second['baseline_self_findings'],[])
        self.assertEqual(second['target_outer_index'],7)
        self.assertTrue(any(s['name']=='debt_increased_scaled' and s['value']['type']=='text' and not s['economic'] for s in second['summary']))


if __name__=='__main__':unittest.main(verbosity=2)
