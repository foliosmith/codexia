import { test, expect } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

test('automatic calibration keeps labels blind, accounts judge cost and stops without retry', async ({}, info) => {
  for (const mode of ['complete', 'limited', 'invalid', 'unknown']) {
    const base = info.outputPath(mode);
    mkdirSync(base, { recursive: true });
    const adapter = join(base, 'judge.mjs');
    writeFileSync(adapter, `#!/usr/bin/env node
import assert from 'node:assert/strict';import {writeFileSync} from 'node:fs';
const chunks=[];for await(const c of process.stdin)chunks.push(c);const request=JSON.parse(Buffer.concat(chunks));
assert.equal(request.task,'semantic_judge');assert.equal(JSON.stringify(request).includes('expectation'),false);
${mode === 'unknown' ? '' : 'writeFileSync(process.env.CODEXIA_USAGE_FILE,JSON.stringify({input_tokens:100,output_tokens:20}));'}
process.stdout.write(JSON.stringify({score:${mode === 'invalid' ? '9' : '2'},verdict:'insufficient',evidence:'Synthetic transport fixture; not a real assessment.',hard_failure:false}));
`, { mode: 0o755 });
    const provider = join(base, 'provider.json');
    writeFileSync(provider, JSON.stringify({ evidence: 'simulation', adapter: './judge.mjs', model: 'synthetic-judge', max_invocations: mode === 'limited' ? 2 : 20, max_request_bytes: 65536, stop_after_input_tokens: 10000, stop_after_output_tokens: 10000, stop_after_estimated_usd: 1, pricing: { source: 'synthetic', as_of: '2026-10-08', input_per_million: 1, cached_input_per_million: 0, output_per_million: 1 } }));
    const output = join(base, 'run');
    const args = [resolve('benchmarks/runner.mjs'), 'judge-calibration', '--provider-config', provider, '--output', output];
    const run = spawnSync(process.execPath, args, { encoding: 'utf8' });
    expect(run.status, run.stderr).toBe(mode === 'complete' ? 0 : 1);
    const report = JSON.parse(readFileSync(join(output, 'report.json')));
    expect(report.calibrated).toBe(false);
    expect(report.evidence).toBe('simulation');
    const count = mode === 'complete' ? 20 : mode === 'limited' ? 2 : 1;
    expect(report.budget.invocations).toBe(count);
    expect(report.budget.known_estimated_usd).toBeCloseTo((mode === 'unknown' ? 0 : count) * 0.00012, 9);
    const ledger = JSON.parse(readFileSync(join(output, 'budget/ledger.json')));
    expect(ledger.calls.every(call => call.phase === 'judge')).toBe(true);
    expect(report.status).toBe(mode === 'complete' ? 'completed' : mode === 'limited' ? 'budget_exceeded' : mode === 'unknown' ? 'unknown_usage' : 'invalid_output');
    expect(spawnSync(process.execPath, args, { encoding: 'utf8' }).status).toBe(1);
    expect(JSON.parse(readFileSync(join(output, 'budget/ledger.json')))).toEqual(ledger);
  }
});
