import { test, expect } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { cpSync, readFileSync, writeFileSync, existsSync, mkdirSync, symlinkSync } from 'node:fs';
import { resolve, join } from 'node:path';

const runner = resolve('benchmarks/runner.mjs');
const invoke = (...args) => spawnSync(process.execPath, [runner, ...args], { encoding: 'utf8', timeout: 120000 });

test('benchmark validates source identity and rejects ambiguous or missing contracts', async ({}, info) => {
  const valid = invoke('validate');
  expect(valid.status, valid.stderr).toBe(0);
  expect(JSON.parse(valid.stdout).cases).toBe(8);
  const root = info.outputPath('suite');
  cpSync('benchmarks/suites/v0.0', root, { recursive: true });
  const casesFile = join(root, 'cases.jsonl');
  const original = readFileSync(casesFile, 'utf8');
  const cases = original.trim().split('\n').map(JSON.parse);
  cases[0].steps[0].selection = 'missing-anchor';
  writeFileSync(casesFile, cases.map(JSON.stringify).join('\n') + '\n');
  const invalid = invoke('validate', '--catalog', join(root, 'catalog.json'));
  expect(invalid.status).toBe(1);
  expect(invalid.stderr).toContain('missing-anchor');
  writeFileSync(casesFile, original);
  const bookFile = join(root, 'fixtures/river-study.json');
  writeFileSync(bookFile, readFileSync(bookFile, 'utf8').replace('NIGHTJAR', 'CHANGED'));
  const changed = invoke('validate', '--catalog', join(root, 'catalog.json'));
  expect(changed.status).toBe(1);
  expect(changed.stderr).toContain('hash');
});

test('benchmark runs real Reader attempts privately without claiming offline semantic success', async ({}, info) => {
  test.setTimeout(120000);
  const run = info.outputPath('run');
  const result = invoke('run', '--output', run);
  expect(result.status, result.stderr).toBe(0);
  const report = JSON.parse(readFileSync(join(run, 'report.json')));
  expect(report.attempts).toBe(8);
  expect(report.execution.completed).toBe(8);
  expect(report.quality).toBe('inconclusive');
  expect(report.semantic_evaluated).toBe(0);
  expect(report.task_successes).toBe(0);
  expect(report.evidence).toBe('offline');
  const trials = readFileSync(join(run, 'trials.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
  const partial = trials.find(t => t.case_id === 'partial-read');
  const partialArtifact = JSON.parse(readFileSync(join(run, partial.artifact)));
  expect(JSON.stringify(partialArtifact.calls.map(call => call.request))).not.toContain('NIGHTJAR');
  const cross = trials.find(t => t.case_id === 'cross-chapter');
  const crossArtifact = JSON.parse(readFileSync(join(run, cross.artifact)));
  expect(JSON.stringify(crossArtifact.calls.map(call => call.request))).toContain('salt water');
  expect(new Set(trials.map(t => t.attempt_id)).size).toBe(8);
  const before = readFileSync(join(run, 'trials.jsonl'), 'utf8');
  expect(invoke('run', '--output', run).status).toBe(1);
  expect(invoke('resume', '--output', run).status).toBe(0);
  expect(readFileSync(join(run, 'trials.jsonl'), 'utf8')).toBe(before);
  const manifest = JSON.parse(readFileSync(join(run, 'manifest.json')));
  manifest.suite_hash = 'changed';
  writeFileSync(join(run, 'manifest.json'), JSON.stringify(manifest));
  expect(invoke('resume', '--output', run).status).toBe(1);
  const publicOutput = resolve('benchmarks/should-not-exist');
  expect(invoke('run', '--output', publicOutput).status).toBe(1);
  expect(existsSync(publicOutput)).toBe(false);
  const linked = info.outputPath('linked');
  mkdirSync(info.outputPath('outside'));
  symlinkSync(info.outputPath('outside'), linked);
  expect(invoke('run', '--output', join(linked, 'run')).status).toBe(1);
});

test('benchmark retains provider failures and enforces attempt budgets', async ({}, info) => {
  test.setTimeout(120000);
  for (const mode of ['empty', 'timeout', 'budget']) {
    const suite = info.outputPath(mode);
    cpSync('benchmarks/suites/v0.0', suite, { recursive: true });
    const item = JSON.parse(readFileSync(join(suite, 'cases.jsonl'), 'utf8').split('\n')[0]);
    item.budget.timeout_ms = mode === 'timeout' ? 1000 : 10000;
    item.budget.max_calls = mode === 'budget' ? 0 : 1;
    writeFileSync(join(suite, 'cases.jsonl'), JSON.stringify(item) + '\n');
    writeFileSync(join(suite, 'gold.jsonl'), readFileSync(join(suite, 'gold.jsonl'), 'utf8').split('\n')[0] + '\n');
    const agent = join(suite, 'agent.mjs');
    writeFileSync(agent, mode === 'timeout' ? '#!/usr/bin/env node\nsetTimeout(()=>{},10000);\n' : '#!/usr/bin/env node\nprocess.stdout.write(JSON.stringify({cards:[]}));\n', { mode: 0o755 });
    const run = join(suite, 'run');
    const result = invoke('run', '--catalog', join(suite, 'catalog.json'), '--output', run, '--agent-command', agent);
    expect(result.status, result.stderr).toBe(1);
    const report = JSON.parse(readFileSync(join(run, 'report.json')));
    expect(report.attempts).toBe(1);
    expect(report.quality).toBe('fail');
    expect(report.execution[mode === 'timeout' ? 'timed_out' : mode === 'budget' ? 'budget_exceeded' : 'provider_error']).toBe(1);
    expect(report.task_successes).toBe(0);
    expect(report.calls).toBe(mode === 'budget' ? 0 : 1);
  }
});
