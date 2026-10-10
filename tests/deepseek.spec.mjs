import { test, expect } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { cpSync, mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { join, resolve, relative } from 'node:path';
import { createHash } from 'node:crypto';

test('DeepSeek transport bounds requests, preserves billed failures and never prints credentials', async ({}, info) => {
  for (const mode of ['success', 'truncated', 'invalid', 'http', 'missing-key', 'oversized', 'unknown-task']) {
    const directory = info.outputPath(mode);
    mkdirSync(directory, { recursive: true });
    const preload = join(directory, 'fetch.mjs');
    const sent = join(directory, 'sent.json');
    const usage = join(directory, 'usage.json');
    writeFileSync(preload, `import assert from 'node:assert/strict';
import {writeFileSync} from 'node:fs';
let calls=0;
globalThis.fetch=async (url, options)=>{
 assert.equal(++calls,1); assert.equal(url,'https://api.deepseek.com/chat/completions');
 assert.equal(options.redirect,'error'); assert.equal(options.headers.authorization,'Bearer test-secret');
 writeFileSync(${JSON.stringify(sent)}, options.body);
 return new Response(JSON.stringify(${JSON.stringify({
      choices: [{ finish_reason: mode === 'truncated' ? 'length' : 'stop', message: { content: mode === 'invalid' ? 'not-json' : '{"cards":[]}' } }],
      usage: { prompt_tokens: 100, prompt_cache_hit_tokens: 25, completion_tokens: 10 },
      error: { message: 'test-secret provider error' },
    })}),{status:${mode === 'http' ? 429 : 200}});
};`);
    const result = spawnSync(process.execPath, ['--import', preload, resolve('benchmarks/adapters/deepseek.mjs')], {
      encoding: 'utf8', input: JSON.stringify({ task: mode === 'unknown-task' ? 'unsupported_task' : 'explain_passage', output_schema: { cards: [] }, context: { text: mode === 'oversized' ? 'x'.repeat(65536) : 'original fixture' } }),
      env: { ...process.env, DEEPSEEK_API_KEY: mode === 'missing-key' ? '' : 'test-secret', CODEXIA_ANALYZER_MODEL: 'deepseek-flash', CODEXIA_USAGE_FILE: usage },
    });
    expect(result.status, result.stderr).toBe(mode === 'success' ? 0 : 1);
    expect(result.stdout + result.stderr).not.toContain('test-secret');
    if (['missing-key', 'oversized', 'unknown-task'].includes(mode)) { expect(existsSync(sent)).toBe(false); continue; }
    const body = JSON.parse(readFileSync(sent));
    expect(body.max_tokens).toBe(1024);
    expect(body.thinking).toEqual({ type: 'disabled' });
    expect(body.response_format).toEqual({ type: 'json_object' });
    if (mode === 'http') expect(existsSync(usage)).toBe(false);
    else expect(JSON.parse(readFileSync(usage))).toEqual({ input_tokens: 100, cached_input_tokens: 25, output_tokens: 10 });
    if (mode === 'success') expect(JSON.parse(result.stdout)).toEqual({ cards: [] });
  }
});


test('DeepSeek compiles a source chapter and answers through Reader, retaining truncated synthesis cost', async ({}, info) => {
  test.setTimeout(120000);
  for (const mode of ['complete', 'truncated-synthesis']) {
    const directory = info.outputPath(mode);
    cpSync('benchmarks/suites/v0.0', directory, { recursive: true });
    const bookFile = join(directory, 'fixtures/river-study.json');
    const book = JSON.parse(readFileSync(bookFile));
    book.chapters = book.chapters.slice(0, 1);
    writeFileSync(bookFile, JSON.stringify(book));
    const catalogFile = join(directory, 'catalog.json');
    const catalog = JSON.parse(readFileSync(catalogFile));
    catalog.books[0].sha256 = createHash('sha256').update(readFileSync(bookFile)).digest('hex');
    writeFileSync(catalogFile, JSON.stringify(catalog));
    const casesFile = join(directory, 'cases.jsonl');
    const cases = readFileSync(casesFile, 'utf8').trim().split('\n').map(JSON.parse);
    const item = cases[0];
    item.steps.push(cases.find(row => row.id === 'reflection').steps[0]);
    item.budget = { timeout_ms: 30000, max_calls: 4 };
    writeFileSync(casesFile, JSON.stringify(item)+'\n');
    const goldFile = join(directory, 'gold.jsonl');
    writeFileSync(goldFile, readFileSync(goldFile, 'utf8').split('\n')[0]+'\n');
    const preload = join(directory, 'fetch.mjs');
    const helper = relative(directory, resolve('tests/fixtures/analyzer.mjs'));
    writeFileSync(preload, `import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';import {dirname,resolve} from 'node:path';import {fileURLToPath} from 'node:url';
const originalFetch=globalThis.fetch;
globalThis.fetch=async(url,options)=>{
 if(String(url).startsWith('http://127.0.0.1:'))return originalFetch(url,options);
 assert.equal(url,'https://api.deepseek.com/chat/completions');assert.equal(options.redirect,'error');
 const body=JSON.parse(options.body);const request=JSON.parse(body.messages[1].content);
 const compile=['chapter_analysis','book_synthesis'].includes(request.task);
 assert.equal(body.max_tokens,compile?8192:1024);
 const result=spawnSync(process.execPath,[resolve(dirname(fileURLToPath(import.meta.url)),${JSON.stringify(helper)})],{input:JSON.stringify(request),encoding:'utf8'});
 assert.equal(result.status,0,result.stderr);
 return new Response(JSON.stringify({choices:[{finish_reason:${JSON.stringify(mode)}==='truncated-synthesis'&&request.task==='book_synthesis'?'length':'stop',message:{content:result.stdout}}],usage:{prompt_tokens:100,prompt_cache_hit_tokens:25,completion_tokens:20}}));
};`);
    const configFile = join(directory, 'provider.json');
    writeFileSync(configFile, JSON.stringify({ evidence: 'simulation', adapter: relative(directory, resolve('benchmarks/adapters/deepseek.mjs')), model: 'synthetic-deepseek', max_invocations: 4, max_request_bytes: 65536, stop_after_input_tokens: 10000, stop_after_output_tokens: 20000, stop_after_estimated_usd: 1, pricing: { source: 'Synthetic regression rates', as_of: '2026-10-10', input_per_million: 1, cached_input_per_million: 0, output_per_million: 1 } }));
    const run = join(directory, 'run');
    const result = spawnSync(process.execPath, [resolve('benchmarks/runner.mjs'), 'run', '--catalog', catalogFile, '--provider-config', configFile, '--compile-with-provider', '--mode', 'cold-compile-reader', '--output', run], {
      encoding: 'utf8', timeout: 60000, env: { ...process.env, DEEPSEEK_API_KEY: 'test-secret', NODE_OPTIONS: '--import '+JSON.stringify(preload) },
    });
    expect(result.status, result.stderr).toBe(mode === 'complete' ? 0 : 1);
    expect(result.stdout+result.stderr).not.toContain('test-secret');
    const report = JSON.parse(readFileSync(join(run, 'report.json')));
    expect(report.provider_budget.phases.compile.invocations).toBe(2);
    expect(report.provider_budget.unknown_usage_calls).toBe(0);
    expect(report.costs.compile_usd).toBeCloseTo(0.00019, 9);
    const artifact = JSON.parse(readFileSync(join(run, 'attempts/citation-1/artifact.json')));
    const pkg = join(run, 'attempts/citation-1/compilation/package');
    if (mode === 'complete') {
      expect(report.structural_passes).toBe(1);
      expect(report.provider_budget.phases.answer.invocations).toBe(2);
      expect(report.costs.answer_usd).toBeCloseTo(0.00019, 9);
      expect(artifact.steps.map(step => step.task)).toEqual(['explain','reflect']);
      expect(JSON.parse(readFileSync(join(pkg, 'compile_status.json'))).complete).toBe(true);
    } else {
      expect(report.provider_budget.phases.answer.invocations).toBe(0);
      expect(artifact.status).toBe('provider_error');
      expect(artifact.steps).toEqual([]);
      expect(JSON.parse(readFileSync(join(pkg, 'compile_status.json'))).complete).toBe(false);
    }
  }
});
