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

test('candidate judge requires reviewed gold and matching calibration before dispatch and produces bound scores', async ({}, info) => {
  test.setTimeout(120000);
  const { cpSync, existsSync } = await import('node:fs');
  const base = info.outputPath('candidate');
  mkdirSync(base, { recursive: true });
  const invoke = (...args) => spawnSync(process.execPath, [resolve('benchmarks/runner.mjs'), ...args], { encoding: 'utf8', timeout: 90000 });
  const suite = join(base, 'suite');
  cpSync('benchmarks/suites/v0.0', suite, { recursive: true });
  for (const file of ['cases.jsonl', 'gold.jsonl']) writeFileSync(join(suite, file), readFileSync(join(suite, file), 'utf8').split('\n')[0] + '\n');
  const catalog = join(suite, 'catalog.json');
  const source = join(base, 'source');
  expect(invoke('run', '--catalog', catalog, '--output', source).status).toBe(0);
  const reference = JSON.parse(readFileSync('benchmarks/suites/v0.0/calibration/samples.json'));
  reference.status = 'reviewed'; reference.reviewer = 'synthetic contract fixture, not real human review';
  for (const sample of reference.samples) sample.expectation = { status: 'reviewed', score: 3, verdict: 'supports', evidence: 'synthetic', hard_failure: false, reason: 'Synthetic contract fixture' };
  const refs = join(base, 'reference.json');
  writeFileSync(refs, JSON.stringify(reference));
  writeFileSync(join(base, 'adapter.mjs'), `#!/usr/bin/env node
import assert from 'node:assert/strict';import {writeFileSync} from 'node:fs';
const chunks=[];for await(const c of process.stdin)chunks.push(c);const r=JSON.parse(Buffer.concat(chunks));
assert.equal(JSON.stringify(r).includes('expectation'),false);
assert.equal(JSON.stringify(r).includes('attempt_id'),false);
const check={score:3,verdict:'supports',evidence:'Synthetic judge contract; not semantic evidence.',hard_failure:false};
writeFileSync(process.env.CODEXIA_USAGE_FILE,JSON.stringify({input_tokens:100,output_tokens:20}));
process.stdout.write(JSON.stringify(r.input.dimensions ? {checks:r.input.dimensions.map(dimension=>({dimension,...check}))}:check));
`, { mode: 0o755 });
  const config = { evidence: 'simulation', adapter: './adapter.mjs', model: 'synthetic-judge', max_invocations: 20, max_request_bytes: 65536, stop_after_input_tokens: 10000, stop_after_output_tokens: 10000, stop_after_estimated_usd: 1, pricing: { source: 'synthetic', as_of: '2026-10-08', input_per_million: 1, cached_input_per_million: 0, output_per_million: 1 } };
  const provider = join(base, 'provider.json');writeFileSync(provider, JSON.stringify(config));
  const calibrated = join(base, 'calibrated');
  const calibrationRun = invoke('judge-calibration', '--calibration', refs, '--provider-config', provider, '--output', calibrated);
  expect(calibrationRun.status, calibrationRun.stderr).toBe(0);
  expect(JSON.parse(readFileSync(join(calibrated, 'report.json'))).calibrated).toBe(true);
  const args = ['judge', '--catalog', catalog, '--left', source, '--provider-config', provider, '--calibration', refs, '--calibration-run', calibrated];
  const draft = join(base, 'draft');
  const denied = invoke(...args, '--output', draft);
  expect(denied.status).toBe(1);expect(denied.stderr).toContain('gold');
  expect(existsSync(join(draft, 'budget/ledger.json'))).toBe(false);
  const gold = JSON.parse(readFileSync(join(suite, 'gold.jsonl')));gold.status = 'reviewed';gold.reviewer = 'synthetic contract fixture';
  writeFileSync(join(suite, 'gold.jsonl'), JSON.stringify(gold)+'\n');
  config.model = 'changed';writeFileSync(provider, JSON.stringify(config));
  const stale = invoke(...args, '--output', join(base, 'stale'));
  expect(stale.status).toBe(1);expect(stale.stderr).toContain('judge identity');
  config.model = 'synthetic-judge';writeFileSync(provider, JSON.stringify(config));
  reference.status = 'draft';writeFileSync(refs, JSON.stringify(reference));
  const changedReference = invoke(...args, '--output', join(base, 'changed-reference'));
  expect(changedReference.status).toBe(1);expect(changedReference.stderr).toContain('stale calibration');
  expect(existsSync(join(base, 'changed-reference/budget/ledger.json'))).toBe(false);
  reference.status = 'reviewed';writeFileSync(refs, JSON.stringify(reference));
  const output = join(base, 'judged');
  const judged = invoke(...args, '--output', output);
  expect(judged.status, judged.stderr).toBe(0);
  const result = JSON.parse(readFileSync(join(output, 'report.json')));
  expect(result.budget.invocations).toBe(1);
  expect(result.budget.known_estimated_usd).toBeCloseTo(0.00012, 9);
  const scored = invoke('score', '--catalog', catalog, '--output', source, '--reviews', join(output, 'reviews.json'));
  expect(scored.status, scored.stderr).toBe(0);
  expect(JSON.parse(readFileSync(join(source, 'report.json'))).semantic_evaluated).toBe(1);
  const reviews = JSON.parse(readFileSync(join(output, 'reviews.json')));
  reviews.reviews[0].artifact_hash = 'stale';writeFileSync(join(output, 'reviews.json'), JSON.stringify(reviews));
  expect(invoke('score', '--catalog', catalog, '--output', source, '--reviews', join(output, 'reviews.json')).status).toBe(1);
});
