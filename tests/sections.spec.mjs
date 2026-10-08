import {compileSections} from './fixtures/sections.mjs';
import {test,expect} from '@playwright/test';
import {execFileSync,spawn,spawnSync} from 'node:child_process';
import {once} from 'node:events';
import {mkdirSync,readFileSync,writeFileSync} from 'node:fs';
import {join,resolve} from 'node:path';

test('logical chapters retain physical provenance, span files and isolate skipped sections',async({request,page},info)=>{
 const root=info.outputPath('sections');mkdirSync(root,{recursive:true});
 const binary=resolve('target/debug/codexia');const pkg=compileSections(root);
 const ir=JSON.parse(readFileSync(join(pkg,'book_ir.json')));expect(ir.chapters).toHaveLength(3);expect(ir.logical_sections.map(s=>s.title)).toEqual(['Preface','Chapter I','Chapter II','Chapter III']);
 const two=ir.logical_sections[2];expect(two.ranges).toHaveLength(2);
 const all=ir.logical_sections.flatMap(s=>s.ranges.flatMap(r=>r.block_ids));expect(new Set(all).size).toBe(ir.blocks.length);expect(all.length).toBe(ir.blocks.length);
 expect(ir.blocks.every(b=>b.chapter_index===b.source_ref.spine_index)).toBe(true);
 const summary=JSON.parse(readFileSync(join(pkg,'chapters','chapter_003.analysis.json')));expect(summary.chapter_title).toBe('Chapter II');
 execFileSync(binary,['validate',pkg],{stdio:'pipe'});
 const fallback=JSON.parse(execFileSync(binary,['parse',join(root,'missing.epub')],{encoding:'utf8'}));expect(fallback.logical_sections).toEqual([]);
 const pride=JSON.parse(execFileSync(binary,['parse',resolve('private/golden-books/source/pride-and-prejudice-pg1342.epub')],{encoding:'utf8',maxBuffer:20*1024*1024}));expect(pride.logical_sections.filter(s=>/Chapter/i.test(s.title))).toHaveLength(61);
 const server=spawn(binary,['serve',pkg,'--state-dir',join(root,'state'),'--bind','127.0.0.1:18797'],{stdio:'ignore'});const base='http://127.0.0.1:18797';
 try{
  await expect.poll(async()=>{try{return(await request.get(base+'/v1/bootstrap')).status()}catch{return 0}}).toBe(200);
  const boot=await(await request.get(base+'/v1/bootstrap')).json();expect(boot.chapters.map(c=>c.title)).toEqual(['Preface','Chapter I','Chapter II','Chapter III']);
  const location={chapter_id:'chapter_003',block_id:null,char_offset:null,epub_cfi:null};const session=await(await request.post(base+'/v1/reader-sessions',{data:{book_id:boot.book.book_id,current_location:location,spoiler_mode:'read_range'}})).json();
  const content=await(await request.get(`${base}/v1/books/${boot.book.book_id}/chapters/chapter_003/content`)).json();const last=content.blocks.at(-1);const end={...location,block_id:last.block_id,char_offset:[...last.text].length};
  const updated=await(await request.patch(base+'/v1/reader-sessions/'+session.session_id,{data:{read_until:end}})).json();expect(updated.completed_chapter_ids).toEqual(['chapter_003']);expect(updated.progress_basis_points).toBe(Math.floor(content.blocks.reduce((n,b)=>n+[...b.text].length,0)*10000/ir.blocks.reduce((n,b)=>n+[...b.text].length,0)));
  await page.goto(base+'/#read/chapter_003');await expect(page.locator('body')).toContainText('Chapter II');
  writeFileSync(info.outputPath('sections-evidence.json'),JSON.stringify({sections:ir.logical_sections,pride:pride.logical_sections.map(s=>s.title),session:updated},null,2));
 }finally{const ended=once(server,'exit');server.kill();await ended;}
 const statePath=join(root,'state','reader_state.json');const legacy=JSON.parse(readFileSync(statePath));delete legacy.reading_layout;writeFileSync(statePath,JSON.stringify(legacy));
 const preservedState=readFileSync(statePath);
 const incompatible=spawnSync(binary,['serve',pkg,'--state-dir',join(root,'state')],{encoding:'utf8',timeout:3000});expect(incompatible.status).toBe(1);expect(incompatible.stderr).toContain('another chapter layout');expect(readFileSync(statePath)).toEqual(preservedState);
 ir.logical_sections[1].ranges[0].block_ids.pop();writeFileSync(join(pkg,'book_ir.json'),JSON.stringify(ir));expect(spawnSync(binary,['validate',pkg]).status).toBe(1);
});
