import {test,expect} from '@playwright/test';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {mkdirSync,readFileSync,writeFileSync,existsSync} from 'node:fs';
import {join,resolve} from 'node:path';
import {compileSections} from './fixtures/sections.mjs';

test('question retrieval stays inside confirmed reading ranges and rejects unoffered citations',async({request,page},info)=>{
 const root=info.outputPath('retrieval');mkdirSync(root,{recursive:true});const compilerCalls=join(root,'compiler-requests.jsonl');const compiler=join(root,'compiler.mjs');writeFileSync(compiler,`#!/usr/bin/env node
import {appendFileSync} from 'node:fs';import {spawnSync} from 'node:child_process';const chunks=[];for await(const c of process.stdin)chunks.push(c);const input=Buffer.concat(chunks);appendFileSync(${JSON.stringify(compilerCalls)},input+'\\n');const out=spawnSync(${JSON.stringify(resolve('tests/fixtures/analyzer.mjs'))},[],{input});process.stdout.write(out.stdout);process.exit(out.status??1);
`,{mode:0o755});const pkg=compileSections(root,compiler);const ir=JSON.parse(readFileSync(join(pkg,'book_ir.json')));const captured=join(root,'contexts.jsonl');
 const unrelated=ir.blocks.find(b=>b.text.includes('continues across'));const agent=join(root,'agent.mjs');
 writeFileSync(agent,`#!/usr/bin/env node
import {appendFileSync} from 'node:fs';const chunks=[];for await(const c of process.stdin)chunks.push(c);const r=JSON.parse(Buffer.concat(chunks));appendFileSync(${JSON.stringify(captured)},JSON.stringify(r)+'\\n');
const blocks=(r.input.question||'').startsWith('escape')?[${JSON.stringify(unrelated)}]:r.context.nearby_blocks;
const refs=blocks.map(b=>({block_id:b.block_id,start_char:0,end_char:[...b.text].length,text_fingerprint:b.text_fingerprint}));if((r.input.question||'').startsWith('offset'))refs[0].end_char-=1;
process.stdout.write(JSON.stringify({cards:[{card_type:'answer',card_id:'answer',title:'Source evidence',content:{answer:blocks.map(b=>b.text).join('\\n')},source_refs:refs,confidence_basis_points:9000,grounding:'grounded',spoiler_status:r.spoiler_boundary.mode==='full_book'?'full_book_allowed':'within_boundary',follow_up_actions:[]}]}));
`,{mode:0o755});
 const binary=resolve('target/debug/codexia');const server=spawn(binary,['serve',pkg,'--state-dir',join(root,'state'),'--bind','127.0.0.1:18798','--agent-command',agent],{stdio:'ignore'});const base='http://127.0.0.1:18798';
 const contexts=()=>existsSync(captured)?readFileSync(captured,'utf8').trim().split('\n').map(JSON.parse):[];
 try{
  await expect.poll(async()=>{try{return(await request.get(base+'/v1/bootstrap')).status()}catch{return 0}}).toBe(200);
  const boot=await(await request.get(base+'/v1/bootstrap')).json();const book=base+'/v1/books/'+boot.book.book_id;
  const loc=id=>({chapter_id:id,block_id:null,char_offset:null,epub_cfi:null});
  let session=await(await request.post(base+'/v1/reader-sessions',{data:{book_id:boot.book.book_id,current_location:loc('chapter_003'),spoiler_mode:'read_range'}})).json();
  const content=async id=>await(await request.get(book+'/chapters/'+id+'/content')).json();
  const confirm=async(id,sessionId=session.session_id,block=null,offset=null)=>{const c=await content(id);const b=block||c.blocks.at(-1);return await(await request.patch(base+'/v1/reader-sessions/'+sessionId,{data:{read_until:{...loc(id),block_id:b.block_id,char_offset:offset??[...b.text].length}}})).json();};
  const ask=async(question,s=session)=>request.post(book+'/ask',{data:{question,reader_state:s,spoiler_mode:'read_range'}});
  const first=(await content('chapter_002')).blocks.find(b=>b.text.includes('bronze'));session=await confirm('chapter_002',session.session_id,first,18);
  const partialReply=await ask('bronze compass');expect(partialReply.status()).toBe(200);expect(JSON.stringify(contexts().at(-1).context)).not.toContain('Mira');
  session=await confirm('chapter_002');session=await confirm('chapter_003');
  const response=await ask('Who owns the bronze compass and follows the northern river?');expect(response.status()).toBe(200);const answer=await response.json();expect(answer.cards[0].content.answer).toContain('Mira');expect(answer.cards[0].content.answer).toContain('Jon');
  const offered=contexts().at(-1).context;expect(offered.nearby_blocks.some(b=>b.chapter_id==='chapter_002')).toBe(true);expect(offered.nearby_blocks.some(b=>b.chapter_id==='chapter_003')).toBe(true);expect(JSON.stringify(offered)).not.toContain('NIGHTJAR');expect(JSON.stringify(offered)).not.toContain('Preface material');
  const current=(await content('chapter_003')).blocks.find(b=>b.text.includes('northern'));
  const connected=await request.post(book+'/explain',{data:{selected_text:current.text,source_ref:{block_id:current.block_id,start_char:0,end_char:[...current.text].length,text_fingerprint:current.text_fingerprint},reader_state:session,spoiler_mode:'read_range',intent:'connect'}});expect(connected.status()).toBe(200);expect(contexts().at(-1).context.nearby_blocks.some(b=>b.chapter_id==='chapter_002')).toBe(true);
  const chinese=await ask('青铜罗盘');expect(chinese.status()).toBe(200);expect((await chinese.json()).cards[0].content.answer).toContain('青铜罗盘属于米拉');
  const before=contexts().length;
  for(const question of ['Where is plutonium stored?','What is the NIGHTJAR password?']){const r=await ask(question);expect(r.status()).toBe(200);expect((await r.json()).cards[0].content.insufficient_evidence).toBe(true);}
  expect(contexts()).toHaveLength(before);
  expect((await ask('escape bronze compass')).status()).toBe(502);
  expect((await ask('offset bronze compass')).status()).toBe(502);
  await page.goto(base+'/#read/chapter_003');await page.getByRole('button',{name:'Chat',exact:true}).click();await page.getByRole('textbox',{name:'向本书提问'}).fill('bronze compass');await page.getByRole('button',{name:'发送',exact:true}).click();
  await page.locator('[data-source-block]').first().click();await expect(page).toHaveURL(/#read\/chapter_002/);await expect(page.locator('article')).toContainText('bronze compass');
  const prompts=readFileSync(compilerCalls,'utf8').trim().split('\n').map(JSON.parse).filter(r=>r.task==='chapter_analysis');
  for(const prompt of prompts){const prior=prompt.context.prior_chapters;expect(prior.every(c=>Array.isArray(c.source_excerpts)&&!('condensed_summary' in c))).toBe(true);expect(prior.flatMap(c=>c.source_excerpts).length).toBeLessThanOrEqual(8);}
  writeFileSync(info.outputPath('retrieval-evidence.json'),JSON.stringify({answer,contexts:contexts()},null,2));
 }finally{const ended=once(server,'exit');server.kill();await ended;}
});
