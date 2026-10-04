import { test, expect } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { cpSync, readFileSync, writeFileSync } from 'node:fs';
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
