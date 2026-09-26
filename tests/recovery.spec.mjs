import {test,expect} from '@playwright/test';
import {spawn,spawnSync} from 'node:child_process';
import {once} from 'node:events';
import {mkdirSync,readFileSync,writeFileSync,existsSync,unlinkSync,readdirSync} from 'node:fs';
import {resolve,join} from 'node:path';

test.setTimeout(90_000);
test('API isolates partial packages and resumes validated chapters after failure and process death',async({request},info)=>{
  const root=info.outputPath('library');mkdirSync(root,{recursive:true});
  const binary=resolve('target/debug/codexia');const key='codexia-recovery-test-key';const keyFile=join(root,'key');writeFileSync(keyFile,key,{mode:0o600});
  const failure=join(root,'fail');writeFileSync(failure,'fail');const events=join(root,'calls.jsonl');const pause=join(root,'pause');const marker=join(root,'paused');
  const analyzer=join(root,'analyzer.mjs');writeFileSync(analyzer,`#!/usr/bin/env node
import {spawnSync} from 'node:child_process';import {existsSync,appendFileSync,writeFileSync} from 'node:fs';
const chunks=[];for await(const c of process.stdin)chunks.push(c);const input=Buffer.concat(chunks);const r=JSON.parse(input);const id=r.context?.chapter?.chapter_id;
appendFileSync(${JSON.stringify(events)},JSON.stringify({task:r.task,id,title:r.context?.book_title})+'\\n');
if(r.task==='chapter_analysis'&&id==='chapter_004'&&r.context.book_title.includes('Souls')){
 if(existsSync(${JSON.stringify(failure)})){process.stderr.write('intentional failed chapter');process.exit(1);}
 if(existsSync(${JSON.stringify(pause)})){writeFileSync(${JSON.stringify(marker)},'paused');await new Promise(resolve=>setTimeout(resolve,60000));}
}
const out=spawnSync(${JSON.stringify(resolve('tests/fixtures/analyzer.mjs'))},[],{input});process.stdout.write(out.stdout);process.stderr.write(out.stderr);process.exit(out.status??1);
`,{mode:0o755});
  const args=['api',root,'--api-key-file',keyFile,'--bind','127.0.0.1:18792','--analyzer-command',analyzer,'--rate-limit-per-minute','1000'];let server;let log='';const states=[];
  async function start(){log='';server=spawn(binary,args,{detached:true,stdio:['ignore','ignore','pipe']});server.stderr.on('data',x=>log+=x);await expect.poll(async()=>{if(server.exitCode!==null)throw Error(log);if(!log.includes('Codexia:'))return 0;try{return(await request.get('http://127.0.0.1:18792/health')).status()}catch{return 0}},{timeout:10000}).toBe(200);}
  async function stop(signal='SIGTERM'){if(server&&server.exitCode===null&&server.signalCode===null){const exited=once(server,'exit');process.kill(-server.pid,signal);await exited;}}
  async function api(path,method='GET',data){const r=await request.fetch('http://127.0.0.1:18792'+path,{method,headers:{'X-API-Key':key},data});return {status:r.status(),body:await r.json()};}
  const wait=async(id,state)=>{await expect.poll(async()=>{const r=await api(`/v1/books/${id}/status`);states.push(r.body);return r.body.state},{timeout:20000}).toBe(state);};
  try{
    await start();
    const good=(await api('/v1/books','POST',readFileSync('private/golden-books/source/alices-adventures-in-wonderland-pg11.epub'))).body;await wait(good.book_id,'ready');
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
  }finally{await stop();writeFileSync(info.outputPath('recovery-states.json'),JSON.stringify(states,null,2));}
});
