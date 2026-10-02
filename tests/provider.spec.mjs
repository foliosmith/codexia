import { test, expect } from "@playwright/test";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync, readdirSync } from "node:fs";
import { resolve, join } from "node:path";

test("invalid chapter citations stop before paid book synthesis", async ({}, info) => {
  const root = info.outputPath("provider");
  const bin = join(root, "bin");
  mkdirSync(bin, { recursive: true });
  const calls = join(root, "calls.txt");
  writeFileSync(join(bin, "codex"), `#!/usr/bin/env node
import {writeFileSync,appendFileSync} from 'node:fs';
const chunks=[];for await(const chunk of process.stdin)chunks.push(chunk);
const request=JSON.parse(Buffer.concat(chunks).toString().trim().split('\\n').at(-1));
appendFileSync(${JSON.stringify(calls)},request.task+'\\n');
if(request.task!=='chapter_analysis'){process.stderr.write('unexpected synthesis');process.exit(1);}
const block=request.context.chapter.blocks[0];
const response={summary:{one_sentence:'Fixture',short:'Fixture',deep:'Fixture',role_in_book:'Fixture'},key_ideas:[],concepts:[{concept_id:'bad',name:'Bad citation',aliases:[],definition_in_this_book:'Fault injection',source_refs:[{block_id:block.block_id,start_char:0,end_char:1,text_fingerprint:'0'.repeat(64)}]}],claims:[],argument_flow:[],difficult_passages:[],entities:[]};
const path=process.argv[process.argv.indexOf('--output-last-message')+1];
writeFileSync(path,JSON.stringify(response));
console.log(JSON.stringify({type:'turn.completed',usage:{input_tokens:1,cached_input_tokens:0,output_tokens:1}}));
`, { mode: 0o755 });
  const output = join(root, "package");
  const result = spawnSync(resolve("target/debug/codexia"), ["compile", resolve("private/golden-books/source/alices-adventures-in-wonderland-pg11.epub"), "--out", output, "--analyze-through", "2", "--analyzer-command", resolve("scripts/online-analyzer.mjs")], {
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, CODEXIA_ANALYZER_MODEL: "fault-injection", CODEXIA_ONLINE_RUN_DIR: root }, encoding: "utf8", timeout: 10_000,
  });
  expect(result.error).toBeUndefined();
  expect(result.status).toBe(1);
  const operations=readFileSync(calls, "utf8").trim().split("\n");
  expect(operations.length).toBeGreaterThan(0);expect([...new Set(operations)]).toEqual(["chapter_analysis"]);
  const events = readdirSync(join(root, "provider")).map((id) => JSON.parse(readFileSync(join(root, "provider", id, "event.json"), "utf8")));
  expect(events.every(event=>event.status==="invalid_output")).toBe(true);
  expect(JSON.parse(readFileSync(join(output, "compile_status.json"), "utf8")).ready_stages).toEqual(["parse", "normalize"]);
  writeFileSync(info.outputPath("provider-failure.log"), result.stderr);
});
