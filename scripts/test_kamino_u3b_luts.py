#!/usr/bin/env python3
"""Offline acquisition-boundary assertions; all mutable fixtures are synthetic."""
import copy
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
import kamino_u3b_lut as lut
spec=importlib.util.spec_from_file_location('capture_lut',Path(__file__).with_name('capture-kamino-u3b-luts.py'))
collector=importlib.util.module_from_spec(spec);spec.loader.exec_module(collector)

class LutAcquisitionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.targets=lut.frozen_targets()

    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='eplyx-u3b-tests-')
        self.root=Path(self.temp.name)/'evidence'
        collector.capture(self.root,self.targets,unavailable_reason='synthetic_configuration_unavailable')

    def tearDown(self):
        self.temp.cleanup()

    def mutate(self,fn):
        p=self.root/'acquisition.json';v=lut.read(p);fn(v);p.write_bytes(lut.canonical(v))

    def test_exact_frozen_membership(self):
        self.assertEqual(len(self.targets),8)
        self.assertEqual(len(lut.requested_contexts(self.targets)),16)
        self.assertEqual(len({k for k,s in lut.requested_contexts(self.targets)}),10)
        self.assertEqual([len(t['address_table_lookups'])for t in self.targets],[1,5,5,1,1,1,1,1])
        self.assertEqual(sum(not t['on_chain_success']for t in self.targets),1)

    def test_unavailable_preserves_every_context_and_never_calls_network(self):
        class ForbiddenRpc:
            def call(self,*args):raise AssertionError('network called')
        capture=lut.validate_capture(self.root,self.targets)
        self.assertEqual(len(capture['requests']),16)
        self.assertTrue(all(r['status']=='failure' and r['failure_reason']=='synthetic_configuration_unavailable'for r in capture['requests']))
        outputs=lut.derive(self.root)
        report=json.loads(outputs['stage-table.json'])
        self.assertEqual(report['after_unproven'],8)
        self.assertEqual(report['after_exact_historical_proof'],0)
        self.assertTrue(all(r['stages'][0]['status']=='failed' and all(s['status']=='not_attempted'for s in r['stages'][1:])for r in report['rows']))

    def test_missing_or_duplicate_context_is_rejected(self):
        for mode in ['missing','duplicate']:
            capture=lut.read(self.root/'acquisition.json')
            changed=copy.deepcopy(capture)
            if mode=='missing':changed['requests'].pop()
            else:changed['requests'].append(changed['requests'][0])
            (self.root/'acquisition.json').write_bytes(lut.canonical(changed))
            with self.assertRaisesRegex(ValueError,'membership'):lut.validate_capture(self.root,self.targets)
            (self.root/'acquisition.json').write_bytes(lut.canonical(capture))

    def test_checkpoint_and_pending_fail_closed(self):
        self.mutate(lambda v:v.update(complete=False))
        with self.assertRaisesRegex(ValueError,'invalid U3B'):lut.validate_capture(self.root,self.targets)
        self.mutate(lambda v:v.update(complete=True))
        self.mutate(lambda v:v['requests'][0].update(status='pending'))
        with self.assertRaisesRegex(ValueError,'unresolved'):lut.validate_capture(self.root,self.targets)

    def test_success_requires_validated_provider(self):
        self.mutate(lambda v:v['requests'][0].update(status='success',failure_reason=None))
        with self.assertRaisesRegex(ValueError,'without validated provider'):lut.validate_capture(self.root,self.targets)

    def test_current_rpc_query_without_exact_slot_is_rejected(self):
        self.mutate(lambda v:v['requests'][0]['params'][1].pop('slot'))
        with self.assertRaisesRegex(ValueError,'parameters'):lut.validate_capture(self.root,self.targets)

    def test_s_minus_one_is_not_execution_slot(self):
        self.mutate(lambda v:v['requests'][0].update(requested_slot=v['requests'][0]['execution_slot']-1))
        with self.assertRaisesRegex(ValueError,'boundary'):lut.validate_capture(self.root,self.targets)

    def test_failure_cannot_reference_unhashed_body(self):
        self.mutate(lambda v:v['requests'][0].update(response_file='rpc/untracked.body',response_sha256='0'*64))
        with self.assertRaisesRegex(ValueError,'unbound'):lut.validate_capture(self.root,self.targets)

    def test_attempt_directory_never_overwritten(self):
        with self.assertRaisesRegex(ValueError,'already exists'):collector.capture(self.root,self.targets,unavailable_reason='again')

    def test_credentials_are_never_provider_identity(self):
        rpc=collector.RawArchive('https://rpc.example/private-secret?apikey=synthetic')
        self.assertEqual(rpc.identity,'https://rpc.example')
        with self.assertRaises(ValueError):collector.RawArchive('https://user:pass@rpc.example')
        with self.assertRaises(ValueError):collector.safe_body(b'{"api_key":"synthetic-secret"}')
        with self.assertRaises(ValueError):collector.safe_body(b'error at https://rpc.example/v2/synthetic-secret')

    def test_raw_non_json_failure_is_preserved(self):
        class BrokenRpc:
            identity='https://synthetic.invalid'
            def call(self,method,params):return b'upstream unavailable',None,'invalid_json_response'
        root=Path(self.temp.name)/'failed-attempt';capture=collector.capture(root,self.targets,BrokenRpc())
        self.assertEqual((root/'rpc/genesis.body').read_bytes(),b'upstream unavailable')
        self.assertEqual(capture['raw_artifact_hashes']['rpc/genesis.body'],lut.sha(b'upstream unavailable'))
        self.assertTrue(all(r['failure_reason']=='invalid_json_response'for r in capture['requests']))
        lut.validate_capture(root,self.targets)

    def test_genesis_mismatch_stops_before_table_requests(self):
        class OtherNetwork:
            identity='https://synthetic.invalid'
            def __init__(self):self.calls=[]
            def call(self,method,params):
                self.calls.append(method);value={'result':'other-network'}
                return lut.canonical(value),value,None
        rpc=OtherNetwork();root=Path(self.temp.name)/'wrong-network';capture=collector.capture(root,self.targets,rpc)
        self.assertEqual(rpc.calls,['getGenesisHash'])
        self.assertTrue(all(r['failure_reason']=='archive_genesis_mismatch'for r in capture['requests']))

    def test_relocated_and_reversed_filesystem_and_workers_are_deterministic(self):
        a=lut.derive(self.root)
        relocated=Path(self.temp.name)/'different-name';relocated.mkdir()
        for path in reversed(sorted(self.root.rglob('*'))):
            if path.is_file():
                destination=relocated/path.relative_to(self.root);destination.parent.mkdir(parents=True,exist_ok=True);shutil.copyfile(path,destination)
        b=lut.derive(relocated,reverse=True,workers=4)
        self.assertEqual(a,b)

    def test_any_frozen_input_change_stops(self):
        relocated=Path(self.temp.name)/'sample';shutil.copytree(lut.SAMPLE,relocated)
        p=relocated/'before-rejections.json';p.write_bytes(p.read_bytes()+b'\n')
        with self.assertRaisesRegex(ValueError,'input differs'):lut.frozen_targets(relocated)

    def validated_archive_fixture(self, table_errors=False):
        fixtures=lut.read(lut.REPO/'engine/tests/fixtures/synthetic-lut-bytes.json')
        self.assertTrue(fixtures['synthetic'])
        tables=fixtures['tables']
        key=self.targets[0]['address_table_lookups'][0]['accountKey']
        # Entire provider and validation witness are synthetic and confined to
        # this test's temp directory. Exercise the production capability schema
        # without writing any successful evidence into production artifacts.
        witness={'kind':'independent_historical_lut_validation_fixture','synthetic':False,'semantics_reference':'synthetic end-of-slot provider model for API assertions','independent_fixture_reference':'officially serialized synthetic byte fixture; TEST ONLY','observations':[{'pubkey':key,'requested_slot':i+1,'expected_account_sha256':tables[i]['data_sha256']}for i in range(2)]}
        class ValidatedMockArchive:
            identity='https://synthetic.invalid'
            def __init__(self):self.calls=[]
            def call(self,method,params):
                self.calls.append((method,params))
                if method=='getGenesisHash':
                    value={'jsonrpc':'2.0','id':1,'result':lut.GENESIS}
                else:
                    slot=params[1]['slot']
                    if table_errors and slot>2:
                        value={'jsonrpc':'2.0','id':1,'result':None,'error':{'code':-32001}}
                        return lut.canonical(value),value,'rpc_error_-32001'
                    table=tables[slot-1]if slot<=2 else tables[2]
                    value={'jsonrpc':'2.0','id':1,'result':{'context':{'slot':slot},'value':{'owner':'AddressLookupTab1e1111111111111111111111111','lamports':10000000,'executable':False,'rentEpoch':0,'data':[table['data_base64'],'base64']}}}
                return lut.canonical(value),value,None
        return ValidatedMockArchive(),witness

    def test_validated_archive_acquisition_and_offline_raw_hash_linkage(self):
        rpc,fixture=self.validated_archive_fixture()
        root=Path(self.temp.name)/'synthetic-valid-provider'
        capture=collector.capture(root,self.targets,rpc,fixture)
        self.assertEqual(sum(r['status']=='success'for r in capture['requests']),16)
        self.assertEqual(len(rpc.calls),19)
        lut.validate_capture(root,self.targets)
        for r in capture['requests']:
            self.assertEqual(r['params'][1]['slot'],r['execution_slot'])
            self.assertEqual(r['response_sha256'],lut.sha((root/r['response_file']).read_bytes()))
        # Arbitrary synthetic addresses MUST disagree with the real frozen RPC.
        outputs=lut.derive(root)
        report=json.loads(outputs['stage-table.json'])
        self.assertEqual(report['after_exact_historical_proof'],0)
        self.assertTrue(all(r['failure']['stage']==4 for r in report['rows']))
        raw=root/capture['requests'][0]['response_file'];raw.write_bytes(raw.read_bytes()+b'\n')
        with self.assertRaisesRegex(ValueError,'raw artifact hash'):lut.validate_capture(root,self.targets)

    def test_prior_provider_contract_does_not_require_divergent_fixture(self):
        rpc,_=self.validated_archive_fixture()
        root=Path(self.temp.name)/'qualified-provider-no-optional-fixture'
        capture=collector.capture(root,self.targets,rpc)
        self.assertEqual(len(rpc.calls),17)
        self.assertTrue(all(r['attempted'] and r['status']=='success' and r['decoded_account_sha256'] for r in capture['requests']))
        self.assertEqual(capture['slot_hashes_requests'],[],'active LUTs must not overfetch SlotHashes')
        contract=lut.read(root/capture['archive_validation_file'])
        self.assertTrue(contract['qualification_reused'])
        self.assertEqual(contract['optional_divergent_fixture'],'not_supplied')
        lut.validate_capture(root,self.targets)
        report=json.loads(lut.derive(root)['stage-table.json'])
        self.assertTrue(all(r['failure']['category']=='rpc_loaded_address_mismatch' for r in report['rows']),'provider trust must never substitute for the LUT cross-check')

    def test_decoded_account_hash_and_attempt_are_revalidated(self):
        rpc,_=self.validated_archive_fixture()
        root=Path(self.temp.name)/'account-hash-test';collector.capture(root,self.targets,rpc)
        original=lut.read(root/'acquisition.json')
        for field,value,expected in [('decoded_account_sha256','0'*64,'account hash'),('attempted',False,'not attempted')]:
            changed=copy.deepcopy(original);changed['requests'][0][field]=value
            (root/'acquisition.json').write_bytes(lut.canonical(changed))
            with self.assertRaisesRegex(ValueError,expected):lut.validate_capture(root,self.targets)

    def test_transaction_context_membership_cannot_be_changed(self):
        self.mutate(lambda v:v['requests'][0].update(transaction_signatures=[]))
        with self.assertRaisesRegex(ValueError,'transaction membership'):lut.validate_capture(self.root,self.targets)

    def test_missing_optional_fixture_is_not_an_acquisition_failure(self):
        capture=lut.validate_capture(self.root,self.targets)
        self.assertTrue(all(r['attempted'] is False for r in capture['requests']))
        self.assertNotIn('independent_historical_lut_validation_fixture_unavailable',str(capture))

    def test_failure_categories_preserve_the_actual_stage(self):
        for detail,expected in [('wrong LUT owner/executable/closed state','wrong_owner'),('official LUT decode: InvalidAccountData','malformed_table'),('historical LUT metadata is from a future slot','slot_context_mismatch'),('historical SlotHashes evidence required','slot_hashes_missing'),('official LUT visibility: LookupTableAccountNotFound','deactivated'),('writable lookup: InvalidLookupIndex','lookup_index_invalid'),('independent loaded addresses differ from frozen RPC','rpc_loaded_address_mismatch'),('official runtime rejects AccountLoadedTwice','duplicate_account'),('compiled instruction identity/privileges/data differ','compiled_index_mismatch')]:
            self.assertEqual(lut.failure_category({'detail':detail,'stage':2},[]),expected)

    def test_warmup_visibility_has_its_own_failure_category(self):
        entries=[{'status':'success','execution_slot':100,'metadata':{'last_extended_slot':100,'last_extended_slot_start_index':2},'address_count':4}]
        lookups=[{'writableIndexes':[3],'readonlyIndexes':[]}]
        self.assertEqual(lut.failure_category({'stage':3,'detail':'writable lookup: InvalidLookupIndex'},entries,lookups),'warmup_visibility_failure')

    def test_wrong_returned_slot_is_retained_as_a_specific_failure(self):
        delegate,_=self.validated_archive_fixture()
        class WrongSlot:
            identity=delegate.identity
            def call(self,method,params):
                body,value,error=delegate.call(method,params)
                if method=='getAccountInfo':
                    value['result']['context']['slot']+=1
                    body=lut.canonical(value)
                return body,value,error
        root=Path(self.temp.name)/'wrong-context';capture=collector.capture(root,self.targets,WrongSlot())
        self.assertTrue(all(r['attempted'] and r['status']=='failure' and r['failure_category']=='slot_context_mismatch' and r['response_file'] for r in capture['requests']))
        report=json.loads(lut.derive(root)['stage-table.json'])
        self.assertTrue(all(r['failure']['category']=='slot_context_mismatch' for r in report['rows']))

    def test_provider_table_errors_preserve_all_membership_and_raw_bodies(self):
        rpc,fixture=self.validated_archive_fixture(table_errors=True)
        root=Path(self.temp.name)/'synthetic-provider-table-errors'
        capture=collector.capture(root,self.targets,rpc,fixture)
        self.assertEqual(len(capture['requests']),16)
        self.assertTrue(all(r['status']=='failure' and r['failure_reason']=='rpc_error_-32001'for r in capture['requests']))
        self.assertTrue(all(lut.read(root/r['response_file'])['error']['code']==-32001 for r in capture['requests']))
        lut.validate_capture(root,self.targets)
        self.assertEqual(len(rpc.calls),19)

    def test_same_historical_byte_hash_cannot_validate_archive_boundary(self):
        provider={'scheme_host':'https://synthetic.invalid','visibility':'finalized_end_of_execution_slot'}
        validation={'kind':'historical_lut_archive_validation','passed':True,'synthetic':False,'provider':provider['scheme_host'],'genesis_hash':lut.GENESIS,'visibility':provider['visibility'],'semantics_reference':'synthetic rule','independent_fixture_reference':'synthetic independent bytes','observations':[{'pubkey':'table','requested_slot':10,'expected_account_sha256':'0'*64},{'pubkey':'table','requested_slot':11,'expected_account_sha256':'0'*64}]}
        fixture={'kind':'independent_historical_lut_validation_fixture','synthetic':False,'semantics_reference':validation['semantics_reference'],'independent_fixture_reference':validation['independent_fixture_reference'],'observations':validation['observations']}
        p=self.root/'synthetic-boundary-test.json';p.write_bytes(lut.canonical(fixture))
        validation.update(independent_fixture_file=p.name,independent_fixture_sha256=lut.sha(p.read_bytes()))
        with self.assertRaisesRegex(ValueError,'divergent'):lut.validate_archive_validation(validation,provider,self.root,{p.name:lut.sha(p.read_bytes())})

if __name__=='__main__':unittest.main(verbosity=2)
