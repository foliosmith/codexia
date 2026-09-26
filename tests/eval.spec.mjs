import {test,expect} from '@playwright/test';
import {spawn,execFileSync} from 'node:child_process';
import {once} from 'node:events';
import {mkdirSync,writeFileSync,readFileSync} from 'node:fs';
import {resolve,join} from 'node:path';

test('Studio separates missing evidence, applicability and artifact-bound semantic review',async({request},info)=>{
  const root=info.outputPath('studio');mkdirSync(root,{recursive:true});
  const pkg=join(root,'package');const binary=resolve('target/debug/codexia');
  execFileSync(binary,['compile',resolve('private/golden-books/source/alices-adventures-in-wonderland-pg11.epub'),'--out',pkg,'--analyzer-command',resolve('tests/fixtures/analyzer.mjs')],{env:{...process.env,CODEXIA_TEST_GROUNDING_MODE:'inferred-refs'},stdio:'pipe'});
  let server=spawn(binary,['studio',pkg,'--state-dir',join(root,'state'),'--bind','127.0.0.1:18791'],{stdio:['ignore','ignore','pipe']});
  let log='';server.stderr.on('data',x=>log+=x);const reports=[];
  const url='http://127.0.0.1:18791/v1/studio';
  async function post(path,data){const r=await request.post(url+path,{data});expect(r.ok(),await r.text()).toBeTruthy();return r.json();}
  try{
    await expect.poll(async()=>{if(server.exitCode!==null)throw Error(log);try{return(await request.get(url+'/snapshot')).status()}catch{return 0}}).toBe(200);
    const empty=await post('/evals',{});reports.push(empty);
    expect(empty.metrics.claim_grounding.state).toBe('missing_required');
    expect(empty.metrics.claim_grounding.value_basis_points).toBeNull();
    expect(empty.semantic.state).toBe('not_evaluated');expect(empty.beta_ready).toBe(false);
    const policy={not_applicable:{claim_grounding:'This bounded fiction fixture does not produce argumentative claims.'}};
    const golden=await post('/golden-books',{label:'Reading semantics',annotations:{expected:'The fixture has contextual evidence, not an argumentative claim.',must_not_reveal:'Later chapter events'}});
    const base=await post('/evals',{...policy,golden_id:golden.golden_id});
    expect(base.metrics.claim_grounding.state).toBe('not_applicable');
    const review={analysis_fingerprint:base.analysis_fingerprint,annotations_fingerprint:base.annotations_fingerprint,reviewer:'deterministic regression oracle',checks:[
      {dimension:'citation_support',expected:'Interpretation is supported by the cited passage.',observed:'The synthetic fixture preserves that interpretation.',passed:true},
      {dimension:'key_point_coverage',expected:'The reference key point is represented.',observed:'The fixture includes the reference point.',passed:true},
      {dimension:'spoiler_boundary',expected:'No later event appears in the bounded answer.',observed:'The bounded fixture contains no later event.',passed:true},
    ]};
    const accepted=await post('/evals',{...policy,golden_id:golden.golden_id,semantic_review:review});reports.push(accepted);
    expect(accepted.beta_ready).toBe(true);
    for(const dimension of ['citation_support','key_point_coverage','spoiler_boundary']){
      const bad=structuredClone(review);const check=bad.checks.find(x=>x.dimension===dimension);check.passed=false;
      check.observed=dimension==='citation_support'?'The real cited text refutes the answer.':dimension==='key_point_coverage'?'The central annotated point is omitted.':'The answer cites an allowed passage but reveals the ending.';
      const rejected=await post('/evals',{...policy,golden_id:golden.golden_id,semantic_review:bad});reports.push(rejected);
      expect(rejected.structural_valid).toBe(true);expect(rejected.beta_ready).toBe(false);
    }
    const stale=await request.post(url+'/evals',{data:{...policy,golden_id:golden.golden_id,semantic_review:{...review,analysis_fingerprint:'old-output'}}});expect(stale.status()).toBe(400);
    const update=await request.put(url+`/golden-books/${golden.golden_id}/annotations`,{data:{annotations:{expected:'Changed reference'}}});expect(update.ok()).toBe(true);
    const changed=await request.post(url+'/evals',{data:{...policy,golden_id:golden.golden_id,semantic_review:review}});expect(changed.status()).toBe(400);
    const missing=await post('/evals',{...policy,golden_id:golden.golden_id});expect(missing.semantic.state).toBe('not_evaluated');
    let exited=once(server,'exit');server.kill();await exited;
    const stateFile=join(root,'state','studio_state.json');const state=JSON.parse(readFileSync(stateFile,'utf8'));
    state.eval_runs=[{...empty,metrics:Object.fromEntries(Object.keys(empty.metrics).map(key=>[key,10000]))}];writeFileSync(stateFile,JSON.stringify(state));
    server=spawn(binary,['studio',pkg,'--state-dir',join(root,'state'),'--bind','127.0.0.1:18791'],{stdio:['ignore','ignore','pipe']});
    server.stderr.on('data',x=>log+=x);
    await expect.poll(async()=>{try{return(await request.get(url+'/snapshot')).status()}catch{return 0}}).toBe(200);
    const history=await(await request.get(url+'/evals')).json();
    expect(history[0].metrics.claim_grounding.state).toBe('not_evaluated');
    expect(history[0].metrics.claim_grounding.value_basis_points).toBeNull();
  }finally{if(server.exitCode===null&&server.signalCode===null){const exited=once(server,'exit');server.kill();await exited;}writeFileSync(info.outputPath('evaluations.json'),JSON.stringify(reports,null,2));}
});
