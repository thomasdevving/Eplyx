#!/usr/bin/env python3
"""Separate bounded historical envelope dependency capture. No current fallback."""
import argparse,importlib.util,json,os,subprocess,time
from pathlib import Path
import kamino_u3b_lut as lut
import kamino_u3c_envelope as envelope
spec=importlib.util.spec_from_file_location('raw_archive',Path(__file__).with_name('capture-kamino-u3b-luts.py'));collector=importlib.util.module_from_spec(spec);spec.loader.exec_module(collector)
def write(path,value):path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(lut.canonical(value))
def capture(root):
    lut.require(not root.exists(),'one directory per attempt; refuses overwrite')
    targets,_,_=envelope.frozen_inputs()
    evidence=envelope.derive()
    rows=[json.loads(body)for name,body in evidence.items()if name.startswith('transactions/')]
    primary=[r for r in rows if envelope.SCOPE in r['before_rejection']]
    lut.require(len(primary)==4 and all(r['envelope']['envelope_admissible']for r in primary),'primary envelope admission required before acquisition')
    endpoint=os.environ.get('SOLANA_ARCHIVE_RPC_URL');lut.require(endpoint,'configure the previously qualified historical archive explicitly')
    rpc=collector.RawArchive(endpoint,os.environ.get('SOLANA_ARCHIVE_RPC_ORIGIN',''))
    qualified=lut.read(envelope.U3B/'acquisition.json')['provider']
    lut.require(rpc.identity==qualified['scheme_host'],'select the previously qualified provider; a different archive requires separate qualification')
    root.mkdir(parents=True);receipt={'kind':'experimental_u3c_dependency_capture','complete':False,'sample_fingerprint':lut.FINGERPRINT,'provider':rpc.identity,'genesis':lut.GENESIS,'archive_validation_sha256':qualified['validation_artifact_sha256'],'requests':[],'raw_artifact_hashes':{},'target_results':[],'max_attempts_per_request_context':1,'fallback':False};cache={};start=time.perf_counter()
    write(root/'receipt.json',receipt)
    for row in primary:
        signature=row['envelope']['signature'];folder=root/signature;folder.mkdir()
        process=subprocess.Popen([str(lut.REPO/'target/debug/examples/acquire_envelope_dependencies')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        process.stdin.write(json.dumps({'transaction':row['transaction'],'output':str(folder.resolve())})+'\n');process.stdin.flush()
        while True:
            line=process.stdout.readline()
            if not line:break
            message=json.loads(line)
            if message['kind']=='capture_result':
                write(folder/'result.json',message['result']);receipt['target_results'].append({'signature':signature,'result_file':f'{signature}/result.json'});continue
            lut.require(message['kind']=='rpc_request','unexpected resolver output')
            method,params=message['method'],message['params'];key=lut.sha(lut.canonical([method,params]))
            if key not in cache:
                tick=time.perf_counter();body,value,error=rpc.call(method,params);seconds=time.perf_counter()-tick
                ref=f'rpc/{key}.body';withheld=False
                try:collector.safe_body(body)
                except ValueError:withheld=True;error='sensitive_response_withheld';value=None
                if not withheld:
                    (root/'rpc').mkdir(exist_ok=True);(root/ref).write_bytes(body);receipt['raw_artifact_hashes'][ref]=lut.sha(body)
                else:ref=None
                cache[key]={'error':error}if error else {'result':value['result']}
                receipt['requests'].append({'request_id':key,'method':method,'params':params,'attempted':True,'status':'failure'if error else 'success','failure_reason':error,'response_file':ref,'response_sha256':lut.sha(body),'response_bytes':len(body),'response_withheld':withheld,'transport_seconds':seconds})
                write(root/'receipt.json',receipt)
            process.stdin.write(json.dumps(cache[key])+'\n');process.stdin.flush()
        code=process.wait()
        if code:
            write(folder/'result.json',{'signature':signature,'failure':{'code':'dependency_binary_unavailable','detail':'resolver stopped; inspect explicit safe request membership'},'runtime_executed':False})
            receipt['target_results'].append({'signature':signature,'result_file':f'{signature}/result.json'})
    receipt['complete']=True
    for path in root.rglob('*'):
        if path.is_file() and path.name!='receipt.json':receipt['raw_artifact_hashes'][str(path.relative_to(root))]=lut.sha(path.read_bytes())
    write(root/'receipt.json',receipt)
    write(root/'timing.json',{'requests':len(receipt['requests']),'account_requests':sum(r['method']=='getAccountInfo'for r in receipt['requests']),'block_requests':sum(r['method']=='getBlock'for r in receipt['requests']),'successful_requests':sum(r['status']=='success'for r in receipt['requests']),'failed_requests':sum(r['status']=='failure'for r in receipt['requests']),'transport_seconds':sum(r['transport_seconds']for r in receipt['requests']),'full_capture_seconds':time.perf_counter()-start})
    print(lut.canonical({'provider':rpc.identity,'target_results':[lut.read(root/r['result_file']) for r in receipt['target_results']],'network_requests':len(receipt['requests'])}).decode())
def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('--output',type=Path,required=True);args=parser.parse_args();capture(args.output)
if __name__=='__main__':
    try:main()
    except Exception:
        print('U3C capture stopped; inspect safe retained membership. Endpoint redacted.',file=__import__('sys').stderr);raise SystemExit(1)
