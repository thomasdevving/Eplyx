// Proposal construction only. The retained configuration is supplied by Rust;
// no token fee, recipient output or analytical result is calculated here.
export const PARAMETER_OPERATION = 'token_2022_active_newer_transfer_fee_basis_points_v1';
export const TOKEN_2022_PROGRAM = 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb';
export const eligibilityEndpoint = (project, parent) => `/v1/projects/${encodeURIComponent(project)}/runs/${encodeURIComponent(parent)}/parameter-change/eligibility`;
export const submissionEndpoint = (project, parent) => `/v1/projects/${encodeURIComponent(project)}/runs/${encodeURIComponent(parent)}/parameter-changes`;
export function parseEligibility(value, project, parent) {
 const fail = () => { throw new Error('The retained parameter eligibility response could not be verified.'); };
 if(value?.schema_version!==1 || value.project_id!==project || value.run_id!==parent || typeof value.eligible!=='boolean') fail();
 if(!value.eligible) {
  if(typeof value.reason_code!=='string' || typeof value.reason!=='string') fail();
  return Object.freeze({eligible:false,reason_code:value.reason_code,reason:value.reason,project_id:project,run_id:parent});
 }
 if(value.operation!==PARAMETER_OPERATION || value.program_id!==TOKEN_2022_PROGRAM || !Number.isInteger(value.current_basis_points) || value.current_basis_points<0 || value.current_basis_points>10000) fail();
 for(const field of ['mint','capture_sha256','token_2022_elf_sha256','mint_data_sha256']) if(typeof value[field]!=='string' || !value[field]) fail();
 for(const field of ['schedule_epoch','captured_epoch','maximum_fee_raw']) if(typeof value[field]!=='string' || !/^(0|[1-9]\d*)$/.test(value[field])) fail();
 if(!value.transfer || !['source','destination'].every(k=>typeof value.transfer[k]==='string' && value.transfer[k]) || !/^(0|[1-9]\d*)$/.test(value.transfer.amount_raw??'') || !Number.isInteger(value.transfer.decimals)) fail();
 const transfer=Object.freeze({source:value.transfer.source,destination:value.transfer.destination,amount_raw:value.transfer.amount_raw,decimals:value.transfer.decimals});
 return Object.freeze({eligible:true,project_id:project,run_id:parent,operation:PARAMETER_OPERATION,program_id:TOKEN_2022_PROGRAM,mint:value.mint,current_basis_points:value.current_basis_points,schedule_epoch:value.schedule_epoch,captured_epoch:value.captured_epoch,maximum_fee_raw:value.maximum_fee_raw,mint_data_sha256:value.mint_data_sha256,capture_sha256:value.capture_sha256,token_2022_elf_sha256:value.token_2022_elf_sha256,transfer});
}
export function validateProposedBps(value) {
 const text=String(value??'').trim();
 if(!text) return 'Enter a proposed transfer fee in basis points.';
 if(!/^\d+$/.test(text) || Number(text)>10000) return 'Use an integer from 0 to 10000 basis points.';
 return null;
}
export function buildParameterSpec(eligibility, value) {
 if(!eligibility?.eligible) throw new Error('This retained transfer is not eligible for parameter analysis.');
 const error=validateProposedBps(value);if(error)throw new Error(error);
 return {schema_version:1,change:{kind:'protocol_parameter_change',target:{program_id:TOKEN_2022_PROGRAM,config_account:eligibility.mint},operation:{kind:PARAMETER_OPERATION,expected_current:{account_data_sha256:eligibility.mint_data_sha256,basis_points:eligibility.current_basis_points,schedule_epoch:eligibility.schedule_epoch,maximum_fee_raw:eligibility.maximum_fee_raw},proposed_basis_points:Number(String(value).trim())}}};
}
export function parameterSubmissionError(error) {
 const code=error.body?.status??error.body?.code;
 if(code==='current_state_mismatch' || String(error.message).includes('current_state_mismatch')) return 'current_state_mismatch: the declaration does not match the retained mint evidence. A newer state requires a new retained transfer run.';
 return error.message || 'The request could not be submitted. Your proposed basis points are retained.';
}
export async function submitParameter({eligibility,value,requestKey,call,navigate}) {
 const change_spec=buildParameterSpec(eligibility,value);
 const accepted=await call(submissionEndpoint(eligibility.project_id,eligibility.run_id),{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({request_key:requestKey,change_spec})});
 if(!/^run_[A-Za-z0-9_-]+$/.test(accepted.run_id??''))throw new Error('The server did not return an analytical run identity.');
 navigate(`/p/${encodeURIComponent(eligibility.project_id)}/runs/${encodeURIComponent(accepted.run_id)}`);
 return accepted;
}
