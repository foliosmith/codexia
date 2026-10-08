import { test, expect } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { join, resolve } from 'node:path';

test('DeepSeek transport bounds requests, preserves billed failures and never prints credentials', async ({}, info) => {
  for (const mode of ['success', 'truncated', 'invalid', 'http', 'missing-key']) {
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
      encoding: 'utf8', input: JSON.stringify({ task: 'explain_passage', output_schema: { cards: [] }, context: { text: 'original fixture' } }),
      env: { ...process.env, DEEPSEEK_API_KEY: mode === 'missing-key' ? '' : 'test-secret', CODEXIA_ANALYZER_MODEL: 'deepseek-flash', CODEXIA_USAGE_FILE: usage },
    });
    expect(result.status, result.stderr).toBe(mode === 'success' ? 0 : 1);
    expect(result.stdout + result.stderr).not.toContain('test-secret');
    if (mode === 'missing-key') { expect(existsSync(sent)).toBe(false); continue; }
    const body = JSON.parse(readFileSync(sent));
    expect(body.max_tokens).toBe(1024);
    expect(body.thinking).toEqual({ type: 'disabled' });
    expect(body.response_format).toEqual({ type: 'json_object' });
    if (mode === 'http') expect(existsSync(usage)).toBe(false);
    else expect(JSON.parse(readFileSync(usage))).toEqual({ input_tokens: 100, cached_input_tokens: 25, output_tokens: 10 });
    if (mode === 'success') expect(JSON.parse(result.stdout)).toEqual({ cards: [] });
  }
});
