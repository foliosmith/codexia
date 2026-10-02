import {test,expect} from '@playwright/test';
import {spawn,spawnSync,execFileSync} from 'node:child_process';
import {once} from 'node:events';
import {mkdirSync,readFileSync,writeFileSync,readdirSync,existsSync} from 'node:fs';
import {join,resolve} from 'node:path';
import net from 'node:net';

test('local runtime bounds requests and model work, cancels children and writes redacted diagnostics',async({request},info)=>{
 test.setTimeout(40000);
 const root=info.outputPath('runtime');mkdirSync(root,{recursive:true});const pkg=join(root,'package');const state=join(root,'state');const binary=resolve('target/debug/codexia');
 execFileSync(binary,['compile',resolve('private/golden-books/source/alices-adventures-in-wonderland-pg11.epub'),'--out',pkg,'--analyzer-command',resolve('tests/fixtures/analyzer.mjs')],{stdio:'pipe'});
 const pidFile=join(root,'provider.pid');const agent=join(root,'agent.mjs');writeFileSync(agent,`#!/usr/bin/env node
import {writeFileSync} from 'node:fs';const chunks=[];for await(const c of process.stdin)chunks.push(c);const r=JSON.parse(Buffer.concat(chunks));writeFileSync(${JSON.stringify(pidFile)},String(process.pid));
if(r.input.question.startsWith('quick')){if(process.env.CODEXIA_USAGE_FILE)writeFileSync(process.env.CODEXIA_USAGE_FILE,JSON.stringify({input_tokens:10,output_tokens:20}));process.stdout.write(JSON.stringify({cards:[{card_type:'answer',card_id:'one',title:'Answer',content:{answer:'Available context.'},source_refs:[],confidence_basis_points:5000,grounding:'inferred',spoiler_status:'full_book_allowed',follow_up_actions:[]}]}));}else{process.stderr.write('DO_NOT_LOG_PROVIDER_SECRET');await new Promise(resolve=>setTimeout(resolve,30000));}
`,{mode:0o755});
 let server;let log='';async function start(extra={}){server=spawn(binary,['serve',pkg,'--state-dir',state,'--bind','127.0.0.1:18793','--agent-command',agent],{stdio:['ignore','ignore','pipe'],env:{...process.env,CODEXIA_READER_TIMEOUT_MS:'1200',CODEXIA_MAX_ANALYZERS:'1',CODEXIA_INPUT_USD_PER_MILLION:'10',CODEXIA_OUTPUT_USD_PER_MILLION:'50',...extra}});server.stderr.on('data',x=>log+=x);await expect.poll(async()=>{if(server.exitCode!==null)throw Error(log);try{return(await request.get('http://127.0.0.1:18793/v1/bootstrap')).status()}catch{return 0}}).toBe(200);}
 async function stop(){if(server?.exitCode===null){const done=once(server,'exit');server.kill();await done;}}
 try{
  await start();const boot=await(await request.get('http://127.0.0.1:18793/v1/bootstrap')).json();const chapter=boot.chapters[0].chapter_id;const location={chapter_id:chapter,block_id:null,char_offset:null,epub_cfi:null};const reader_state={session_id:null,current_location:location,read_until:location,completed_chapter_ids:[],progress_basis_points:0};const book=`http://127.0.0.1:18793/v1/books/${boot.book.book_id}`;
  const payload=(id,question)=>({request_id:id,question,reader_state,spoiler_mode:'full_book'});
  expect((await request.post(book+'/ask',{headers:{Origin:'https://untrusted.example'},data:payload('cross-site','quick rabbit')})).status()).toBe(403);
  const first=request.post(book+'/ask',{data:payload('cancel-me','DO_NOT_LOG_QUESTION rabbit')});await expect.poll(()=>existsSync(pidFile)).toBe(true);
  const busy=await request.post(book+'/ask',{data:payload('busy','quick rabbit')});expect(busy.status()).toBe(503);
  const cancelled=await request.post(book+'/actions/cancel-me/cancel',{data:{}});expect(cancelled.status()).toBe(202);
  expect((await first).status()).toBe(409);
  const pid=Number(readFileSync(pidFile,'utf8'));await expect.poll(()=>{try{process.kill(pid,0);return false}catch{return true}}).toBe(true);
  const timed=await request.post(book+'/ask',{data:payload('timeout-me','DO_NOT_LOG_QUESTION rabbit')});expect(timed.status()).toBe(504);
  const quick=await request.post(book+'/ask',{data:payload('quick-one','quick rabbit')});expect(quick.status()).toBe(200);
  const response=await new Promise((resolve,reject)=>{const socket=net.connect(18793,'127.0.0.1');let data='';socket.on('connect',()=>socket.write('POST /v1/books HTTP/1.1\r\nHost: localhost\r\nContent-Length: 9000000\r\n\r\n'));socket.on('data',x=>data+=x);socket.on('end',()=>resolve(data));socket.on('error',reject);});expect(response).toContain('413');expect(response).toContain('request_too_large');
  const files=readdirSync(join(state,'diagnostics')).filter(f=>f.endsWith('.json'));const events=files.map(f=>JSON.parse(readFileSync(join(state,'diagnostics',f),'utf8')));
  expect(events.some(e=>e.status==='cancelled')).toBe(true);expect(events.some(e=>e.status==='timeout')).toBe(true);
  const measured=events.find(e=>e.request_id==='quick-one');expect(measured.usage.input_tokens).toBe(10);expect(measured.estimated_usd).toBeCloseTo(0.0011);
  expect(JSON.stringify(events)+log).not.toContain('DO_NOT_LOG');
  await stop();await start({CODEXIA_MAX_CONTEXT_BYTES:'512'});const rejected=await request.post(book+'/ask',{data:payload('too-long','quick rabbit')});expect(rejected.status()).toBe(413);
  writeFileSync(info.outputPath('runtime-evidence.json'),JSON.stringify({events,oversize:response},null,2));
 }finally{await stop();}
});


test('compiler rejects oversized context, times out adapters and rejects unsafe EPUB input',async({},info)=>{
 const root=info.outputPath('limits');mkdirSync(root,{recursive:true});const binary=resolve('target/debug/codexia');const epub=resolve('private/golden-books/source/alices-adventures-in-wonderland-pg11.epub');
 const run=(name,agent,env)=>spawnSync(binary,['compile',epub,'--out',join(root,name),'--analyzer-command',agent],{env:{...process.env,...env},encoding:'utf8',timeout:10000});
 const large=run('large',resolve('tests/fixtures/analyzer.mjs'),{CODEXIA_MAX_CONTEXT_BYTES:'512'});expect(large.status).toBe(1);expect(large.stderr).toContain('context_too_large');
 const slow=join(root,'slow.mjs');writeFileSync(slow,`#!/usr/bin/env node
for await(const _ of process.stdin){};await new Promise(r=>setTimeout(r,30000));`,{mode:0o755});
 const timeout=run('timeout',slow,{CODEXIA_ANALYZER_TIMEOUT_MS:'100'});expect(timeout.status).toBe(1);expect(timeout.stderr).toContain('timeout');
 const noisy=join(root,'noisy.mjs');writeFileSync(noisy,`#!/usr/bin/env node
for await(const _ of process.stdin){};process.stdout.write('X'.repeat(10000));`,{mode:0o755});
 const overflow=run('overflow',noisy,{CODEXIA_MAX_OUTPUT_BYTES:'1000'});expect(overflow.status).toBe(1);expect(overflow.stderr).toContain('output_too_large');
 const keyFile=join(root,'key');writeFileSync(keyFile,'runtime-test-secret',{mode:0o644});
 const permissions=spawnSync(binary,['api',join(root,'library'),'--api-key-file',keyFile],{encoding:'utf8',timeout:2000});expect(permissions.status).toBe(1);expect(permissions.stderr).toContain('owner-only');
 execFileSync('chmod',['600',keyFile]);writeFileSync(keyFile,'                    ');const emptyKey=spawnSync(binary,['api',join(root,'library'),'--api-key-file',keyFile],{encoding:'utf8',timeout:2000});expect(emptyKey.status).toBe(1);expect(emptyKey.stderr).toContain('API key');
 const broken=join(root,'broken.epub');writeFileSync(broken,'not a zip');expect(spawnSync(binary,['parse',broken]).status).toBe(1);
 execFileSync('python3',['-c',`import zipfile,sys
source,root=sys.argv[1:]
for name,entry,content in [('traversal','../../escaped','bad'),('ratio','huge.txt','A'*2000000)]:
 with zipfile.ZipFile(source) as src,zipfile.ZipFile(root+'/'+name+'.epub','w',compression=zipfile.ZIP_DEFLATED) as dst:
  for i in src.infolist():dst.writestr(i,src.read(i))
  dst.writestr(entry,content)`,epub,root]);
 for(const name of ['traversal','ratio']) {const result=spawnSync(binary,['parse',join(root,name+'.epub')],{encoding:'utf8',timeout:10000});expect(result.status,result.stderr).toBe(1);}
 const remote=spawnSync(binary,['serve',join(root,'large'),'--bind','0.0.0.0:18795'],{encoding:'utf8',timeout:2000});expect(remote.status).toBe(1);expect(remote.stderr).toContain('loopback');
 writeFileSync(info.outputPath('limits-evidence.json'),JSON.stringify({context:large.stderr,timeout:timeout.stderr,output:overflow.stderr},null,2));
});

test('API compile cancellation and deadlines preserve retryable failed state',async({request},info)=>{
 const root=info.outputPath('compile-cancel');mkdirSync(root,{recursive:true});const binary=resolve('target/debug/codexia');const key='runtime-compile-test-key';const keyFile=join(root,'key');writeFileSync(keyFile,key,{mode:0o600});
 const pidFile=join(root,'child.pid');const slow=join(root,'slow.mjs');writeFileSync(slow,`#!/usr/bin/env node\nimport {writeFileSync} from 'node:fs';for await(const _ of process.stdin){};writeFileSync(${JSON.stringify(pidFile)},String(process.pid));await new Promise(r=>setTimeout(r,30000));`,{mode:0o755});
 const server=spawn(binary,['api',root,'--api-key-file',keyFile,'--bind','127.0.0.1:18796','--analyzer-command',slow],{env:{...process.env,CODEXIA_COMPILE_TIMEOUT_MS:'3000'},stdio:'ignore'});
 const base='http://127.0.0.1:18796';const headers={'X-API-Key':key};
 try{
  await expect.poll(async()=>{try{return(await request.get(base+'/health')).status()}catch{return 0}}).toBe(200);
  const upload=await request.post(base+'/v1/books',{headers,data:readFileSync('private/golden-books/source/alices-adventures-in-wonderland-pg11.epub')});const {book_id}=await upload.json();
  await expect.poll(()=>existsSync(pidFile)).toBe(true);
  expect((await request.post(base+'/v1/books',{headers,data:readFileSync('private/golden-books/source/the-souls-of-black-folk-pg408.epub')})).status()).toBe(503);
  expect((await request.post(`${base}/v1/books/${book_id}/cancel`,{headers})).status()).toBe(202);
  const status=async()=>await(await request.get(`${base}/v1/books/${book_id}/status`,{headers})).json();
  await expect.poll(async()=>(await status()).state).toBe('failed');expect((await status()).error).toBe('cancelled');
  const pid=Number(readFileSync(pidFile,'utf8'));await expect.poll(()=>{try{process.kill(pid,0);return false}catch{return true}}).toBe(true);
  expect((await request.post(`${base}/v1/books/${book_id}/retry`,{headers})).status()).toBe(202);
  await expect.poll(async()=>(await status()).error,{timeout:10000}).toBe('timeout');
  writeFileSync(info.outputPath('compile-cancel-evidence.json'),JSON.stringify(await status(),null,2));
 }finally{const stopped=once(server,'exit');server.kill();await stopped;}
});
