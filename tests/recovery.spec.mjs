import {test,expect} from '@playwright/test';
import {spawn,spawnSync} from 'node:child_process';
import {once} from 'node:events';
import {mkdirSync,readFileSync,writeFileSync,existsSync,unlinkSync,readdirSync} from 'node:fs';
import {resolve,join} from 'node:path';

test.setTimeout(90_000);
test('API isolates partial packages and resumes validated chapters after failure and process death',async({request},info)=>{
  const root=info.outputPath('library');mkdirSync(root,{recursive:true});
  const binary=resolve('target/debug/codexia');const key='codexia-recovery-test-key';const keyFile=join(root,'key');writeFileSync(keyFile,key,{mode:0o600});
  const failure=join(root,'fail');writeFileSync(failure,'fail');const upgradeFailure=join(root,'upgrade-fail');const events=join(root,'calls.jsonl');const pause=join(root,'pause');const marker=join(root,'paused');
  const analyzer=join(root,'analyzer.mjs');writeFileSync(analyzer,`#!/usr/bin/env node
import {spawnSync} from 'node:child_process';import {existsSync,appendFileSync,writeFileSync} from 'node:fs';
const chunks=[];for await(const c of process.stdin)chunks.push(c);const input=Buffer.concat(chunks);const r=JSON.parse(input);const id=r.context?.chapter?.chapter_id;
appendFileSync(${JSON.stringify(events)},JSON.stringify({task:r.task,id,title:r.context?.book_title,profile:r.analysis_profile})+'\\n');
if(r.analysis_profile==='deep'&&existsSync(${JSON.stringify(upgradeFailure)})){process.stderr.write('intentional upgrade failure');process.exit(1);}
if(r.task==='chapter_analysis'&&id==='chapter_004'&&r.context.book_title.includes('Souls')){
 if(existsSync(${JSON.stringify(failure)})){process.stderr.write('intentional failed chapter');process.exit(1);}
 if(existsSync(${JSON.stringify(pause)})){writeFileSync(${JSON.stringify(marker)},'paused');await new Promise(resolve=>setTimeout(resolve,60000));}
}
const out=spawnSync(${JSON.stringify(resolve('tests/fixtures/analyzer.mjs'))},[],{input});process.stdout.write(out.stdout);process.stderr.write(out.stderr);process.exit(out.status??1);
`,{mode:0o755});
  const args=['api',root,'--api-key-file',keyFile,'--bind','127.0.0.1:18792','--analyzer-command',analyzer,'--rate-limit-per-minute','1000'];let revision='v1';let server;let log='';const states=[];
  async function start(){log='';server=spawn(binary,args,{detached:true,env:{...process.env,CODEXIA_ANALYZER_REVISION:revision},stdio:['ignore','ignore','pipe']});server.stderr.on('data',x=>log+=x);await expect.poll(async()=>{if(server.exitCode!==null)throw Error(log);if(!log.includes('Codexia:'))return 0;try{return(await request.get('http://127.0.0.1:18792/health')).status()}catch{return 0}},{timeout:10000}).toBe(200);}
  async function stop(signal='SIGTERM'){if(server&&server.exitCode===null&&server.signalCode===null){const exited=once(server,'exit');process.kill(-server.pid,signal);await exited;}}
  async function api(path,method='GET',data,headers={}){const r=await request.fetch('http://127.0.0.1:18792'+path,{method,headers:{'X-API-Key':key,...headers},data});return {status:r.status(),body:await r.json()};}
  const wait=async(id,state)=>{await expect.poll(async()=>{const r=await api(`/v1/books/${id}/status`);states.push(r.body);return r.body.state},{timeout:20000}).toBe(state);};
  try{
    await start();
    const good=(await api('/v1/books','POST',readFileSync('private/golden-books/source/alices-adventures-in-wonderland-pg11.epub'))).body;await wait(good.book_id,'ready');
    const alice=readFileSync('private/golden-books/source/alices-adventures-in-wonderland-pg11.epub');
    const manifestPath=`/v1/books/${good.book_id}/files/manifest.json`;
    await api('/v1/books','POST',alice,{'X-Codexia-Profile':'basic'});await wait(good.book_id,'ready');
    expect((await api(manifestPath)).body.profile).toBe('basic');
    const before=readFileSync(events,'utf8').split('\n').length;
    await Promise.all([api('/v1/books','POST',alice,{'X-Codexia-Profile':'basic'}),api('/v1/books','POST',alice,{'X-Codexia-Profile':'basic'})]);
    expect(readFileSync(events,'utf8').split('\n').length).toBe(before);
    writeFileSync(upgradeFailure,'fail');await api('/v1/books','POST',alice,{'X-Codexia-Profile':'deep'});await wait(good.book_id,'failed');
    expect((await api(manifestPath)).body.profile).toBe('basic');
    await stop();await start();expect((await api(manifestPath)).body.profile).toBe('basic');
    expect((await api(`/v1/books/${good.book_id}/status`)).body.state).toBe('failed');
    unlinkSync(upgradeFailure);await api(`/v1/books/${good.book_id}/retry`,'POST',{});await wait(good.book_id,'ready');
    expect((await api(manifestPath)).body.profile).toBe('deep');
    const previousKey=(await api(`/v1/books/${good.book_id}/status`)).body.analysis_key;
    await stop();revision='v2';await start();
    const accepts=await Promise.all([api('/v1/books','POST',alice,{'X-Codexia-Profile':'deep'}),api('/v1/books','POST',alice,{'X-Codexia-Profile':'deep'})]);
    expect(accepts.map(x=>x.status)).toEqual([202,202]);await wait(good.book_id,'ready');
    expect((await api(`/v1/books/${good.book_id}/status`)).body.analysis_key).not.toBe(previousKey);
    const broken=(await api('/v1/books','POST',readFileSync('private/golden-books/source/the-souls-of-black-folk-pg408.epub'))).body;await wait(broken.book_id,'failed');
    const dir=join(root,'books',broken.book_id,'package');
    expect(readdirSync(join(dir,'chapters')).filter(name=>name.endsWith('.analysis.json')).length).toBeGreaterThan(0);
    const failedStatus=JSON.parse(readFileSync(join(dir,'compile_status.json'),'utf8'));expect(failedStatus.complete).toBe(false);expect(failedStatus.error).toContain('intentional');expect(failedStatus.completed_artifacts.length).toBeGreaterThan(0);
    await stop();await start();
    expect((await api(`/v1/books/${good.book_id}/status`)).body.state).toBe('ready');expect((await api(`/v1/books/${broken.book_id}/status`)).body.state).toBe('failed');
    expect((await api(`/v1/books/${broken.book_id}/package`)).status).toBe(404);
    unlinkSync(failure);writeFileSync(pause,'pause');
    expect((await api(`/v1/books/${broken.book_id}/retry`,'POST',{})).status).toBe(202);
    await expect.poll(()=>existsSync(marker)).toBe(true);
    expect((await api(`/v1/books/${broken.book_id}/retry`,'POST',{})).status).toBe(409);
    const overlap=spawnSync(binary,['compile',resolve('private/golden-books/source/the-souls-of-black-folk-pg408.epub'),'--out',dir,'--analyzer-command',analyzer],{encoding:'utf8',timeout:5000});
    expect(overlap.status).toBe(1);expect(overlap.stderr).toContain('another compilation owns');
    await stop('SIGKILL');unlinkSync(pause);
    writeFileSync(join(dir,'chapters','chapter_003.analysis.json'),'{corrupt');
    await start();
    expect((await api(`/v1/books/${good.book_id}/status`)).body.state).toBe('ready');expect((await api(`/v1/books/${broken.book_id}/status`)).body.state).toBe('failed');
    expect((await api(`/v1/books/${broken.book_id}/retry`,'POST',{})).status).toBe(202);await wait(broken.book_id,'ready');
    const done=JSON.parse(readFileSync(join(dir,'compile_status.json'),'utf8'));expect(done.attempt).toBe(3);expect(done.complete).toBe(true);
    const calls=readFileSync(events,'utf8').trim().split('\n').map(JSON.parse).filter(c=>c.title?.includes('Souls')&&c.task==='chapter_analysis');
    expect(calls.filter(c=>c.id==='chapter_002')).toHaveLength(1);expect(calls.filter(c=>c.id==='chapter_004')).toHaveLength(3);expect(calls.filter(c=>c.id==='chapter_003')).toHaveLength(2);
    expect((await api(`/v1/books/${broken.book_id}/retry`,'POST',{})).status).toBe(409);
    await stop();writeFileSync(join(dir,'book_map.json'),'{broken');await start();
    expect((await api(`/v1/books/${good.book_id}/status`)).body.state).toBe('ready');expect((await api(`/v1/books/${broken.book_id}/status`)).body.state).toBe('failed');
    expect((await api(`/v1/books/${broken.book_id}/retry`,'POST',{})).status).toBe(202);await wait(broken.book_id,'ready');
    const cli=join(root,'cli-package');const cliArgs=['compile',resolve('private/golden-books/source/alices-adventures-in-wonderland-pg11.epub'),'--out',cli,'--analyzer-command',analyzer];
    const original=spawnSync(binary,cliArgs,{encoding:'utf8'});expect(original.status,original.stderr).toBe(0);
    const originalManifest=readFileSync(join(cli,'manifest.json'),'utf8');
    writeFileSync(upgradeFailure,'fail');const replacement=spawnSync(binary,[...cliArgs,'--profile','deep','--force'],{encoding:'utf8'});
    expect(replacement.status).toBe(1);expect(readFileSync(join(cli,'manifest.json'),'utf8')).toBe(originalManifest);
    expect(spawnSync(binary,['validate',cli]).status).toBe(0);
    unlinkSync(upgradeFailure);const promoted=spawnSync(binary,[...cliArgs,'--profile','deep'],{encoding:'utf8'});expect(promoted.status,promoted.stderr).toBe(0);
    expect(JSON.parse(readFileSync(join(cli,'manifest.json'),'utf8')).profile).toBe('deep');
  }finally{await stop();writeFileSync(info.outputPath('recovery-states.json'),JSON.stringify(states,null,2));}
});
