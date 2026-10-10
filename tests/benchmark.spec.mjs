import { test, expect } from '@playwright/test';
import { spawnSync } from 'node:child_process';
import { cpSync, readFileSync, writeFileSync, existsSync, mkdirSync, symlinkSync, unlinkSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { hostname } from 'node:os';
import { resolve, join, relative } from 'node:path';

const runner = resolve('benchmarks/runner.mjs');
const invoke = (...args) => spawnSync(process.execPath, [runner, ...args], { encoding: 'utf8', timeout: 120000 });

test('configured compilation shares the run ledger and cold attempt invocation budget', async ({}, info) => {
  test.setTimeout(120000);
  for (const mode of ['complete', 'global-limit', 'case-limit']) {
    const suite = info.outputPath(mode);
    cpSync('benchmarks/suites/v0.0', suite, { recursive: true });
    const item = JSON.parse(readFileSync(join(suite, 'cases.jsonl'), 'utf8').split('\n')[0]);
    item.budget.max_calls = mode === 'case-limit' ? 3 : 5;
    writeFileSync(join(suite, 'cases.jsonl'), JSON.stringify(item) + '\n');
    writeFileSync(join(suite, 'gold.jsonl'), readFileSync(join(suite, 'gold.jsonl'), 'utf8').split('\n')[0] + '\n');
    const helper = relative(suite, resolve('tests/fixtures/analyzer.mjs'));
    writeFileSync(join(suite, 'provider.mjs'), `#!/usr/bin/env node
import {writeFileSync} from 'node:fs';import {spawnSync} from 'node:child_process';import {resolve,dirname} from 'node:path';import {fileURLToPath} from 'node:url';
const chunks=[];for await(const c of process.stdin)chunks.push(c);
writeFileSync(process.env.CODEXIA_USAGE_FILE,JSON.stringify({input_tokens:100,output_tokens:20}));
const result=spawnSync(process.execPath,[resolve(dirname(fileURLToPath(import.meta.url)),${JSON.stringify(helper)})],{input:Buffer.concat(chunks)});process.stdout.write(result.stdout);process.exit(result.status??1);
`, { mode: 0o755 });
    const config = { evidence: 'simulation', adapter: './provider.mjs', model: 'synthetic-compile-provider', max_invocations: mode === 'global-limit' ? 2 : 5, max_request_bytes: 524288, stop_after_input_tokens: 50000, stop_after_output_tokens: 50000, stop_after_estimated_usd: 1, pricing: { source: 'Synthetic regression rates', as_of: '2026-10-08', input_per_million: 1, cached_input_per_million: 0, output_per_million: 1 } };
    const configFile = join(suite, 'provider.json');
    writeFileSync(configFile, JSON.stringify(config));
    const run = join(suite, 'run');
    const validation = invoke('validate', '--catalog', join(suite, 'catalog.json'), '--provider-config', configFile);
    expect(validation.status, validation.stderr).toBe(0);
    expect(JSON.parse(validation.stdout).provider_config_hash).toHaveLength(64);
    const result = invoke('run', '--catalog', join(suite, 'catalog.json'), '--provider-config', configFile, '--compile-with-provider', '--mode', 'cold-compile-reader', '--output', run);
    expect(result.status, result.stderr).toBe(mode === 'complete' ? 0 : 1);
    const report = JSON.parse(readFileSync(join(run, 'report.json')));
    const compileCalls = mode === 'complete' ? 4 : mode === 'global-limit' ? 2 : 3;
    expect(report.provider_budget.phases.compile.invocations).toBe(compileCalls);
    expect(report.provider_budget.phases.answer.invocations).toBe(mode === 'complete' ? 1 : 0);
    expect(report.costs.compile_usd).toBeCloseTo(compileCalls * 0.00012, 9);
    expect(report.costs.answer_usd).toBeCloseTo(mode === 'complete' ? 0.00012 : 0, 9);
    const trial = JSON.parse(readFileSync(join(run, 'trials.jsonl'), 'utf8'));
    expect(trial.call_count).toBe(mode === 'complete' ? 5 : compileCalls);
    if (mode !== 'complete') expect(trial.status).toBe('budget_exceeded');
    expect(JSON.parse(readFileSync(join(run, 'manifest.json'))).compiler).toBe('configured-provider');
    if (mode === 'complete') {
      unlinkSync(join(run, 'attempts/citation-1/trial.json'));
      unlinkSync(join(run, 'attempts/citation-1/artifact.json'));
      expect(invoke('resume', '--catalog', join(suite, 'catalog.json'), '--provider-config', configFile, '--compile-with-provider', '--mode', 'cold-compile-reader', '--output', run).status).toBe(1);
      const recovered = JSON.parse(readFileSync(join(run, 'trials.jsonl'), 'utf8'));
      expect(recovered.status).toBe('cancelled');
      expect(recovered.call_count).toBe(5);
      expect(JSON.parse(readFileSync(join(run, 'budget/ledger.json'))).calls).toHaveLength(5);
    }
  }
});

test('provider budget caps invocations, accounts cache usage and stops on unknown cost', async ({}, info) => {
  test.setTimeout(120000);
  for (const mode of ['count', 'cost', 'unknown']) {
    const suite = info.outputPath(mode);
    cpSync('benchmarks/suites/v0.0', suite, { recursive: true });
    for (const name of ['cases', 'gold']) {
      const file = join(suite, `${name}.jsonl`);
      writeFileSync(file, readFileSync(file, 'utf8').split('\n').slice(0, 2).join('\n') + '\n');
    }
    const helper = relative(suite, resolve('benchmarks/adapters/extract.mjs'));
    const adapter = join(suite, 'provider.mjs');
    writeFileSync(adapter, `#!/usr/bin/env node
import {appendFileSync,writeFileSync} from 'node:fs';
import {spawnSync} from 'node:child_process';
import {dirname,resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
const chunks=[];for await(const chunk of process.stdin)chunks.push(chunk);
appendFileSync(new URL('./invocations.txt',import.meta.url),'called\\n');
${mode === 'unknown' ? '' : 'writeFileSync(process.env.CODEXIA_USAGE_FILE,JSON.stringify({input_tokens:1000,cached_input_tokens:400,output_tokens:20}));'}
const result=spawnSync(process.execPath,[resolve(dirname(fileURLToPath(import.meta.url)),${JSON.stringify(helper)})],{input:Buffer.concat(chunks)});
process.stdout.write(result.stdout);process.exit(result.status??1);
`, { mode: 0o755 });
    const config = { evidence: 'simulation', adapter: './provider.mjs', model: 'synthetic-provider', max_invocations: mode === 'count' ? 1 : 5, max_request_bytes: 524288, stop_after_input_tokens: 50000, stop_after_output_tokens: 50000, stop_after_estimated_usd: mode === 'cost' ? 0.001 : 1, pricing: { source: 'Synthetic regression rates, not market pricing', as_of: '2026-10-08', input_per_million: 2, cached_input_per_million: 0.5, output_per_million: 10 } };
    const configFile = join(suite, 'provider.json');
    writeFileSync(configFile, JSON.stringify(config));
    const run = join(suite, 'run');
    const args = ['--catalog', join(suite, 'catalog.json'), '--provider-config', configFile, '--output', run];
    const result = invoke('run', ...args);
    expect(result.status, result.stderr).toBe(1);
    expect(readFileSync(join(suite, 'invocations.txt'), 'utf8').trim().split('\n')).toHaveLength(1);
    const report = JSON.parse(readFileSync(join(run, 'report.json')));
    expect(report.evidence).toBe('simulation');
    expect(report.execution.budget_exceeded).toBe(1);
    expect(report.provider_budget.invocations).toBe(1);
    expect(report.provider_budget.hard_spend_cap).toBe(false);
    if (mode === 'unknown') {
      expect(report.costs.answer_usd).toBeNull();
      expect(report.provider_budget.unknown_usage_calls).toBe(1);
    } else expect(report.costs.answer_usd).toBeCloseTo(0.0016, 9);
    if (mode === 'cost') expect(report.provider_budget.estimated_overshoot_usd).toBeCloseTo(0.0006, 9);
    const denial = JSON.parse(readFileSync(join(run, 'attempts/unsupported-claim-1/budget-exceeded.json')));
    expect(denial.reason).toBe({ count: 'invocation_limit', cost: 'estimated_cost_threshold', unknown: 'unaccounted_invocation' }[mode]);
    expect(invoke('resume', ...args).status).toBe(1);
    expect(readFileSync(join(suite, 'invocations.txt'), 'utf8').trim().split('\n')).toHaveLength(1);
    config.max_invocations++;
    writeFileSync(configFile, JSON.stringify(config));
    expect(invoke('resume', ...args).stderr).toContain('provider_config_hash changed');
    if (mode === 'count') {
      config.api_key = 'DO_NOT_PERSIST';
      writeFileSync(configFile, JSON.stringify(config));
      const rejectedPath = join(suite, 'invalid-config-run');
      const rejected = invoke('run', '--catalog', join(suite, 'catalog.json'), '--provider-config', configFile, '--output', rejectedPath);
      expect(rejected.status).toBe(1);
      expect(rejected.stderr).not.toContain('DO_NOT_PERSIST');
      expect(existsSync(rejectedPath)).toBe(false);
    }
  }
});

test('benchmark separates cold compilation from shared-package Reader timing', async ({}, info) => {
  test.setTimeout(120000);
  const suite = info.outputPath('suite');
  cpSync('benchmarks/suites/v0.0', suite, { recursive: true });
  for (const name of ['cases', 'gold']) {
    const file = join(suite, `${name}.jsonl`);
    writeFileSync(file, readFileSync(file, 'utf8').split('\n')[0] + '\n');
  }
  const catalog = join(suite, 'catalog.json');
  for (const mode of ['fixed-package-reader', 'cold-compile-reader']) {
    const run = join(suite, mode);
    const args = ['--catalog', catalog, '--output', run, '--mode', mode, '--repeat', '2'];
    const result = invoke('run', ...args);
    expect(result.status, result.stderr).toBe(0);
    const report = JSON.parse(readFileSync(join(run, 'report.json')));
    expect(report.compilations).toHaveLength(mode === 'fixed-package-reader' ? 1 : 2);
    expect(report.reader_timings).toHaveLength(2);
    expect(report.repeated_cases[0]).toEqual({ case_id: 'citation', planned_attempts: 2, recorded_attempts: 2, successes: 0, all_passed: null });
    for (const timing of report.reader_timings) {
      expect(timing.first_result_ms).toBeGreaterThanOrEqual(timing.reader_ms);
      expect(timing.first_result_ms).toBeGreaterThanOrEqual(timing.compile_ms);
    }
    for (const compilation of report.compilations) {
      expect(compilation.source_bytes).toBeGreaterThan(0);
      expect(compilation.package_bytes).toBeGreaterThan(compilation.analysis_bytes);
      expect(compilation.analysis_bytes).toBeGreaterThan(0);
    }
    const manifest = JSON.parse(readFileSync(join(run, 'manifest.json')));
    expect(Date.parse(manifest.retention.content_expires_at) - Date.parse(manifest.started_at)).toBe(7 * 86400000);
    const original = readFileSync(join(run, 'trials.jsonl'), 'utf8');
    expect(invoke('resume', ...args).status).toBe(0);
    expect(readFileSync(join(run, 'trials.jsonl'), 'utf8')).toBe(original);
    if (mode === 'cold-compile-reader') {
      expect(invoke('resume', '--catalog', catalog, '--output', run, '--repeat', '2').status).toBe(1);
      for (const id of ['citation-1', 'citation-2']) expect(JSON.parse(readFileSync(join(run, 'attempts', id, 'compilation/package/compile_status.json'))).attempt).toBe(1);
    }
  }
  const compared = invoke('compare', '--left', join(suite, 'fixed-package-reader'), '--right', join(suite, 'cold-compile-reader'), '--output', info.outputPath('bad-compare'));
  expect(compared.status).toBe(1);
  expect(compared.stderr).toContain('incomparable mode');
});

test('benchmark corpus audit rejects renamed copies across development and holdout', async ({}, info) => {
  const sealed = info.outputPath('sealed');
  cpSync('benchmarks/suites/v0.0', sealed, { recursive: true });
  const catalogFile = join(sealed, 'catalog.json');
  const catalog = JSON.parse(readFileSync(catalogFile));
  catalog.split = 'holdout';
  writeFileSync(catalogFile, JSON.stringify(catalog));
  const output = info.outputPath('conflict');
  const result = invoke('audit-corpus', '--holdout', catalogFile, '--output', output);
  expect(result.status).toBe(1);
  const audit = JSON.parse(readFileSync(join(output, 'corpus-audit.json')));
  expect(audit.conflicts.map(c => c.reason)).toContain('same_source_family');
  expect(audit.sealed_execution_ready).toBe(false);
  const bookFile = join(sealed, 'fixtures/river-study.json');
  const book = JSON.parse(readFileSync(bookFile));
  book.family = 'renamed-family';
  writeFileSync(bookFile, JSON.stringify(book));
  catalog.books[0].sha256 = createHash('sha256').update(readFileSync(bookFile)).digest('hex');
  writeFileSync(catalogFile, JSON.stringify(catalog));
  const renamed = info.outputPath('renamed');
  expect(invoke('audit-corpus', '--holdout', catalogFile, '--output', renamed).status).toBe(1);
  expect(JSON.parse(readFileSync(join(renamed, 'corpus-audit.json'))).conflicts.map(c => c.reason)).toEqual(['identical_source']);
  expect(invoke('run', '--catalog', catalogFile, '--output', info.outputPath('must-not-run')).status).toBe(1);
});

test('cold benchmark deadline includes compilation and retains its failure', async ({}, info) => {
  const suite = info.outputPath('suite');
  cpSync('benchmarks/suites/v0.0', suite, { recursive: true });
  const item = JSON.parse(readFileSync(join(suite, 'cases.jsonl'), 'utf8').split('\n')[0]);
  item.budget.timeout_ms = 800;
  writeFileSync(join(suite, 'cases.jsonl'), JSON.stringify(item) + '\n');
  writeFileSync(join(suite, 'gold.jsonl'), readFileSync(join(suite, 'gold.jsonl'), 'utf8').split('\n')[0] + '\n');
  const binary = join(suite, 'slow-compiler.mjs');
  writeFileSync(binary, '#!/usr/bin/env node\nimport {writeFileSync} from "node:fs";writeFileSync(process.argv[process.argv.indexOf("--out")+1]+".started","started");setTimeout(()=>process.exit(1),1800);\n', { mode: 0o755 });
  const run = join(suite, 'run');
  const result = invoke('run', '--catalog', join(suite, 'catalog.json'), '--mode', 'cold-compile-reader', '--binary', binary, '--output', run);
  expect(result.status).toBe(1);
  expect(existsSync(join(run, 'attempts/citation-1/compilation/package.started'))).toBe(true);
  const trial = JSON.parse(readFileSync(join(run, 'trials.jsonl'), 'utf8'));
  expect(trial.status).toBe('timed_out');
  expect(trial.call_count).toBe(0);
  const artifact = JSON.parse(readFileSync(join(run, trial.artifact)));
  expect(artifact.compilation.status).toBe('failed');
  expect(artifact.timing.reader_ms).toBeNull();
  const report = JSON.parse(readFileSync(join(run, 'report.json')));
  expect(report.compilations).toHaveLength(1);
  expect(report.reader_timings[0].first_result_ms).toBeNull();
});

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
  expect(report.package_evaluations['river-study'].structural_valid).toBe(true);
  expect(report.package_evaluations['river-study'].beta_ready).toBe(false);
  expect(report.retrieval.find(r => r.attempt_id === 'citation-1').complete_evidence_set).toBe(true);
  expect(report.retrieval.find(r => r.attempt_id === 'synonym-1').complete_evidence_set).toBe(false);
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

test('benchmark binds reviews to artifacts and exports only aggregate fields', async ({}, info) => {
  test.setTimeout(120000);
  const suite = info.outputPath('suite');
  cpSync('benchmarks/suites/v0.0', suite, { recursive: true });
  const caseFile = join(suite, 'cases.jsonl');
  writeFileSync(caseFile, readFileSync(caseFile, 'utf8').split('\n')[0] + '\n');
  const goldFile = join(suite, 'gold.jsonl');
  const gold = JSON.parse(readFileSync(goldFile, 'utf8').split('\n')[0]);
  gold.status = 'reviewed';
  gold.reviewer = 'Synthetic E2E oracle; not human corpus review';
  writeFileSync(goldFile, JSON.stringify(gold) + '\n');
  const run = join(suite, 'run');
  const args = ['--catalog', join(suite, 'catalog.json'), '--output', run];
  expect(invoke('run', ...args).status).toBe(0);
  expect(invoke('review-template', ...args).status).toBe(0);
  const reviewFile = join(run, 'review-template.json');
  const reviews = JSON.parse(readFileSync(reviewFile));
  const review = reviews.reviews[0];
  review.reviewer = 'Synthetic E2E oracle';
  review.judge_version = 'synthetic-contract-1';
  for (const check of review.checks) Object.assign(check, { score: 3, verdict: 'supports', evidence: 'Controlled source extraction; contract test only.', hard_failure: false });
  writeFileSync(reviewFile, JSON.stringify(reviews));
  expect(invoke('score', ...args, '--reviews', reviewFile).status).toBe(0);
  const accepted = JSON.parse(readFileSync(join(run, 'report.json')));
  expect(accepted.semantic_evaluated).toBe(1);
  expect(accepted.task_successes).toBe(1);
  expect(accepted.quality).toBe('inconclusive');
  const before = readFileSync(join(run, 'report.json'), 'utf8');
  const stale = structuredClone(reviews);
  stale.reviews[0].artifact_hash = '0'.repeat(64);
  const staleFile = join(run, 'stale.json');
  writeFileSync(staleFile, JSON.stringify(stale));
  const rejection = invoke('score', ...args, '--reviews', staleFile);
  expect(rejection.status).toBe(1);
  expect(rejection.stderr).toContain('stale artifact');
  expect(readFileSync(join(run, 'report.json'), 'utf8')).toBe(before);
  Object.assign(review.checks[0], { score: 0, verdict: 'refutes', evidence: 'Private reviewer note: DO_NOT_EXPORT_THIS', hard_failure: true });
  writeFileSync(reviewFile, JSON.stringify(reviews));
  expect(invoke('score', ...args, '--reviews', reviewFile).status).toBe(1);
  expect(JSON.parse(readFileSync(join(run, 'report.json'))).scores[0].hard_failures).toContain('semantic:citation_support');
  expect(invoke('export', ...args, '--reviews', reviewFile).status).toBe(1);
  const exported = readFileSync(join(run, 'export-summary.json'), 'utf8');
  expect(exported).not.toContain('DO_NOT_EXPORT_THIS');
  expect(exported).not.toContain('river-study');
  expect(exported).not.toContain('attempts/');
  expect(exported).not.toContain('reviewer');
  expect(JSON.parse(exported).hard_failures).toBe(1);
  gold.required_points.push('A revised annotation requires a fresh review, not a new provider call.');
  writeFileSync(goldFile, JSON.stringify(gold) + '\n');
  const oldGold = invoke('score', ...args, '--reviews', reviewFile);
  expect(oldGold.status).toBe(1);
  expect(oldGold.stderr).toContain('stale gold');
  review.gold_hash = createHash('sha256').update(JSON.stringify(gold)).digest('hex');
  writeFileSync(reviewFile, JSON.stringify(reviews));
  expect(invoke('score', ...args, '--reviews', reviewFile).stderr).not.toContain('changed; use a new run');
  expect(invoke('resume', ...args).status).toBe(1);
});

test('benchmark prepares blinded review packets containing only allowed source ranges', async ({}, info) => {
  const suite = info.outputPath('suite');
  cpSync('benchmarks/suites/v0.0', suite, { recursive: true });
  for (const name of ['cases', 'gold']) {
    const file = join(suite, `${name}.jsonl`);
    const rows = readFileSync(file, 'utf8').trim().split('\n').map(JSON.parse);
    writeFileSync(file, JSON.stringify(rows.find(row => (row.id || row.case_id) === 'partial-read')) + '\n');
  }
  const run = join(suite, 'run');
  const args = ['--catalog', join(suite, 'catalog.json'), '--output', run];
  expect(invoke('run', ...args).status).toBe(0);
  const prepared = invoke('prepare-review', ...args);
  expect(prepared.status, prepared.stderr).toBe(0);
  const directory = JSON.parse(prepared.stdout).review_packets;
  const binding = JSON.parse(readFileSync(join(directory, 'bindings.json')));
  const packet = JSON.parse(readFileSync(join(directory, `${binding[0].packet_id}.json`)));
  expect(JSON.stringify(packet.steps[0].evidence)).not.toContain('NIGHTJAR');
  expect(JSON.stringify(packet.steps[0].evidence)).toContain('The sealed result is pending');
  expect(packet.gold.forbidden_conclusions).toContain('NIGHTJAR');
  expect(packet.candidate).toBeUndefined();
  expect(packet.attempt_id).toBeUndefined();
  expect(packet.steps[0].answer.cards[0].card_id).toBeUndefined();
  expect(binding[0].artifact_hash).toHaveLength(64);
});

test('benchmark calibration retains disagreement and missing samples without claiming validation', async ({}, info) => {
  const prepared = info.outputPath('prepared');
  expect(invoke('prepare-calibration', '--output', prepared).status).toBe(0);
  const inputs = JSON.parse(readFileSync(join(prepared, 'inputs.json')));
  expect(inputs.samples).toHaveLength(20);
  expect(inputs.samples[0].expectation).toBeUndefined();
  const judgments = JSON.parse(readFileSync(join(prepared, 'judgments-template.json')));
  judgments.judge_version = 'synthetic-input-contract';
  judgments.judgments = ['c01', 'c02'].map(id => ({ id, score: 3, verdict: 'supports', evidence: 'Synthetic judgment for calibration accounting.', hard_failure: false }));
  const path = join(prepared, 'judgments.json');
  writeFileSync(path, JSON.stringify(judgments));
  const output = info.outputPath('scored');
  expect(invoke('calibrate', '--output', output, '--judgments', path).status).toBe(0);
  const report = JSON.parse(readFileSync(join(output, 'report.json')));
  expect(report.matches).toBe(1);
  expect(report.missing_samples).toBe(18);
  expect(report.calibrated).toBe(false);
  judgments.judgments.push(judgments.judgments[0]);
  writeFileSync(path, JSON.stringify(judgments));
  const bad = invoke('calibrate', '--output', info.outputPath('duplicate'), '--judgments', path);
  expect(bad.status).toBe(1);
  expect(bad.stderr).toContain('duplicate calibration');
});

test('benchmark calibration admission requires every reference reviewed and every judgment matching', async ({}, info) => {
  const reference = JSON.parse(readFileSync('benchmarks/suites/v0.0/calibration/samples.json'));
  const refFile = info.outputPath('reference.json');
  mkdirSync(info.outputPath(''), { recursive: true });
  reference.status = 'reviewed';
  reference.reviewer = 'Synthetic contract fixture, not an actual human corpus review';
  const judgmentsFile = info.outputPath('judgments.json');
  const judgments = { judge_version: 'synthetic-contract-1', judgments: reference.samples.map(s => ({ id: s.id, score: s.expectation.score, verdict: s.expectation.verdict, hard_failure: s.expectation.hard_failure, evidence: 'Synthetic prediction used only to test admission policy.' })) };
  function writeInputs() {
    writeFileSync(refFile, JSON.stringify(reference));
    judgments.calibration_hash = createHash('sha256').update(readFileSync(refFile)).digest('hex');
    writeFileSync(judgmentsFile, JSON.stringify(judgments));
  }
  writeInputs();
  const args = ['--calibration', refFile, '--judgments', judgmentsFile];
  const draft = info.outputPath('draft-expectations');
  expect(invoke('calibrate', ...args, '--output', draft).status).toBe(0);
  expect(JSON.parse(readFileSync(join(draft, 'report.json'))).calibrated).toBe(false);
  for (const sample of reference.samples) sample.expectation.status = 'reviewed';
  writeInputs();
  const accepted = info.outputPath('accepted');
  expect(invoke('calibrate', ...args, '--output', accepted).status).toBe(0);
  expect(JSON.parse(readFileSync(join(accepted, 'report.json'))).calibrated).toBe(true);
  judgments.judgments[1].hard_failure = false;
  writeInputs();
  const rejected = info.outputPath('missed-hard-failure');
  expect(invoke('calibrate', ...args, '--output', rejected).status).toBe(1);
  expect(JSON.parse(readFileSync(join(rejected, 'report.json'))).calibrated).toBe(false);
  delete reference.reviewer;
  writeInputs();
  expect(invoke('calibrate', ...args, '--output', info.outputPath('unattributed')).status).toBe(1);
});

test('benchmark A1 uses independent source packages and fixed reflection tasks in both modes', async ({}, info) => {
  test.setTimeout(180000);
  const suite = info.outputPath('suite');
  cpSync('benchmarks/suites/v0.0', suite, { recursive: true });
  const casesFile = join(suite, 'cases.jsonl');
  const cases = readFileSync(casesFile, 'utf8').trim().split('\n').map(JSON.parse).filter(row => ['citation', 'reflection'].includes(row.id));
  const reflection = cases.find(row => row.id === 'reflection');
  reflection.steps[0].question = 'Under what conditions is the copper sensor reliable?';
  reflection.steps.push({ ...reflection.steps[0], question: 'Does this retelling preserve the temperature limitation?' });
  for (const item of cases) item.budget.max_calls = 6;
  writeFileSync(casesFile, cases.map(row => JSON.stringify(row)+'\n').join(''));
  const goldFile = join(suite, 'gold.jsonl');
  writeFileSync(goldFile, readFileSync(goldFile, 'utf8').trim().split('\n').filter(line => ['citation', 'reflection'].includes(JSON.parse(line).case_id)).join('\n')+'\n');
  const catalog = join(suite, 'catalog.json');
  const helper = relative(suite, resolve('tests/fixtures/analyzer.mjs'));
  writeFileSync(join(suite, 'provider.mjs'), `#!/usr/bin/env node
import {writeFileSync} from 'node:fs';import {spawnSync} from 'node:child_process';import {resolve,dirname} from 'node:path';import {fileURLToPath} from 'node:url';
const chunks=[];for await(const c of process.stdin)chunks.push(c);
writeFileSync(process.env.CODEXIA_USAGE_FILE,JSON.stringify({input_tokens:100,output_tokens:20}));
const result=spawnSync(process.execPath,[resolve(dirname(fileURLToPath(import.meta.url)),${JSON.stringify(helper)})],{input:Buffer.concat(chunks)});process.stdout.write(result.stdout);process.exit(result.status??1);
`, { mode: 0o755 });
  const configFile = join(suite, 'provider.json');
  writeFileSync(configFile, JSON.stringify({ evidence: 'simulation', adapter: './provider.mjs', model: 'synthetic-a1-provider', max_invocations: 20, max_request_bytes: 524288, stop_after_input_tokens: 50000, stop_after_output_tokens: 50000, stop_after_estimated_usd: 1, pricing: { source: 'Synthetic regression rates', as_of: '2026-10-09', input_per_million: 1, cached_input_per_million: 0, output_per_million: 1 } }));
  for (const mode of ['fixed-package-reader', 'cold-compile-reader']) {
    const left = join(suite, mode, 'a0');
    const right = join(suite, mode, 'a1');
    for (const [candidate, run] of [['A0', left], ['A1', right]]) {
      const args = ['run', '--catalog', catalog, '--candidate', candidate, '--mode', mode, '--provider-config', configFile, '--output', run];
      if (candidate === 'A0') args.push('--compile-with-provider');
      const result = invoke(...args);
      expect(result.status, result.stderr).toBe(0);
      const artifact = JSON.parse(readFileSync(join(run, 'attempts/reflection-1/artifact.json')));
      const answers = artifact.calls.filter(call => call.request?.task === 'reflect_on_answer');
      expect(answers.map(call => call.request.input.question).sort()).toEqual(reflection.steps.map(step => step.question).sort());
      expect(answers.every(call => !('expected_points' in call.request.input))).toBe(true);
      expect(artifact.steps.map(step => step.question)).toEqual(reflection.steps.map(step => step.question));
      if (candidate === 'A1') {
        expect(artifact.calls.every(call => call.phase === 'answer')).toBe(true);
        for (const name of ['citation', 'reflection']) {
          const output = JSON.parse(readFileSync(join(run, 'attempts', name+'-1', 'artifact.json')));
          for (const call of output.calls) {
            expect(call.request.context.chapter_analysis).toBeNull();
            expect(call.request.context.related_concepts).toEqual([]);
            expect(call.request.context.argument_flow).toEqual([]);
            expect(call.request.context.nearby_blocks.length).toBeGreaterThan(0);
            expect(JSON.stringify(call.request)).not.toContain('Was this chapter registered?');
            expect(JSON.stringify(call.request)).not.toContain('Offline registration');
          }
        }
        const report = JSON.parse(readFileSync(join(run, 'report.json')));
        expect(report.structural_passes).toBe(2);
        expect(report.provider_budget.phases.compile.invocations).toBe(0);
        expect(report.provider_budget.phases.answer.invocations).toBe(3);
        expect(report.costs.compile_usd).toBe(0);
        expect(report.costs.answer_usd).toBeCloseTo(0.00036, 9);
        expect(JSON.parse(readFileSync(join(run, 'manifest.json'))).compiler).toBe('source-only');
        const pkg = mode === 'fixed-package-reader' ? join(run, 'packages/river-study/package') : join(run, 'attempts/reflection-1/compilation/package');
        const checkpoints = JSON.parse(readFileSync(join(pkg, 'checkpoints.json'))).checkpoints;
        expect(checkpoints.every(c => c.recall_questions.every(q => q.expected_points.length === 0))).toBe(true);
      }
    }
    const comparison = info.outputPath(mode+'-comparison');
    const result = invoke('compare', '--left', left, '--right', right, '--output', comparison);
    expect(result.status, result.stderr).toBe(0);
    const report = JSON.parse(readFileSync(join(comparison, 'comparison.json')));
    expect(report.paired_attempts).toBe(2);
    expect(report.counts.unassessed).toBe(2);
    expect(report.conclusion).toBe('inconclusive');
    expect(report.left.compiler).toBe('configured-provider');
    expect(report.right.compiler).toBe('source-only');
    expect(invoke('resume', '--catalog', catalog, '--mode', mode, '--candidate', 'A1', '--provider-config', configFile, '--output', right).status).toBe(0);
    expect(JSON.parse(readFileSync(join(right, 'budget/ledger.json'))).calls).toHaveLength(3);
    const manifestFile = join(right, 'manifest.json');
    const manifest = JSON.parse(readFileSync(manifestFile));
    delete manifest.reflection_protocol;
    writeFileSync(manifestFile, JSON.stringify(manifest));
    expect(invoke('compare', '--left', left, '--right', right, '--output', info.outputPath(mode+'-stale')).status).toBe(1);
    manifest.reflection_protocol = 'fixed-question-v1';
    writeFileSync(manifestFile, JSON.stringify(manifest));
    const reportFile = join(right, 'report.json');
    const changed = JSON.parse(readFileSync(reportFile));
    changed.scorer_hash = 'different';
    writeFileSync(reportFile, JSON.stringify(changed));
    const rejected = invoke('compare', '--left', left, '--right', right, '--output', info.outputPath(mode+'-bad-scorer'));
    expect(rejected.status).toBe(1);
    expect(rejected.stderr).toContain('incomparable scorer_hash');
  }
  const denied = join(suite, 'denied');
  expect(invoke('run', '--catalog', catalog, '--candidate', 'A1', '--provider-config', configFile, '--compile-with-provider', '--output', denied).status).toBe(1);
  expect(existsSync(denied)).toBe(false);
  delete reflection.steps[0].question;
  writeFileSync(casesFile, cases.map(row => JSON.stringify(row)+'\n').join(''));
  const missing = invoke('validate', '--catalog', catalog);
  expect(missing.status).toBe(1);
  expect(missing.stderr).toContain('reflection needs question');
});

test('benchmark resumes interrupted evidence without rerunning calls and verifies private EPUB identity', async ({}, info) => {
  test.setTimeout(120000);
  const suite = info.outputPath('suite');
  cpSync('benchmarks/suites/v0.0', suite, { recursive: true });
  for (const name of ['cases', 'gold']) {
    const path = join(suite, `${name}.jsonl`);
    writeFileSync(path, readFileSync(path, 'utf8').split('\n')[0] + '\n');
  }
  const catalogFile = join(suite, 'catalog.json');
  const first = join(suite, 'first');
  expect(invoke('run', '--catalog', catalogFile, '--output', first).status).toBe(0);
  const epub = join(first, 'packages/river-study/source.epub');
  const catalog = JSON.parse(readFileSync(catalogFile));
  catalog.books[0].epub = { path: 'first/packages/river-study/source.epub', sha256: createHash('sha256').update(readFileSync(epub)).digest('hex') };
  writeFileSync(catalogFile, JSON.stringify(catalog));
  const run = join(suite, 'real-epub');
  const args = ['--catalog', catalogFile, '--output', run];
  expect(invoke('run', ...args).status).toBe(0);
  const originalTrial = JSON.parse(readFileSync(join(run, 'trials.jsonl'), 'utf8'));
  const attempt = join(run, 'attempts', originalTrial.attempt_id);
  unlinkSync(join(attempt, 'trial.json'));
  unlinkSync(join(attempt, 'artifact.json'));
  expect(invoke('report', ...args).status).toBe(0);
  expect(JSON.parse(readFileSync(join(run, 'report.json'))).missing_attempts).toBe(1);
  expect(JSON.parse(readFileSync(join(run, 'report.json'))).independent_books).toBe(0);
  const lock = join(run, '.lock');
  writeFileSync(lock, JSON.stringify({ pid: process.pid, host: hostname() }));
  const live = invoke('resume', ...args);
  expect(live.status).toBe(1);
  expect(live.stderr).toContain('live process');
  const ended = spawnSync(process.execPath, ['-e', 'process.stdout.write(String(process.pid))'], { encoding: 'utf8' });
  writeFileSync(lock, JSON.stringify({ pid: Number(ended.stdout), host: hostname() }));
  expect(invoke('resume', ...args).status).toBe(1);
  const recovered = JSON.parse(readFileSync(join(run, 'trials.jsonl'), 'utf8'));
  expect(recovered.status).toBe('cancelled');
  expect(recovered.call_count).toBe(originalTrial.call_count);
  expect(JSON.parse(readFileSync(join(run, 'report.json'))).missing_attempts).toBe(0);
  writeFileSync(epub, Buffer.from('changed source'));
  const changed = invoke('validate', '--catalog', catalogFile);
  expect(changed.status).toBe(1);
  expect(changed.stderr).toContain('EPUB hash mismatch');
});
