#!/usr/bin/env node
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, readFileSync, writeFileSync, readdirSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { loadSuite, root, hash, read } from './data.mjs';
import { privatePath, save, treeHash, acquireLock } from './storage.mjs';
import { compile, execute, collectCalls } from './adapters/a0.mjs';
import { report } from './scoring/score.mjs';
import { template, validateReviews, preparePackets } from './scoring/review.mjs';
import { calibration } from './scoring/calibration.mjs';
import { judgeCalibration } from './scoring/judge.mjs';
import { compare } from './compare.mjs';
import { auditCorpus } from './corpus.mjs';
import { providerConfig, usageRecord } from './budget.mjs';

process.umask(0o077);
const controller = new AbortController();
process.on('SIGINT', () => controller.abort());
process.on('SIGTERM', () => controller.abort());

function summarize(directory, manifest, suite, reviewFile) {
  const attempts = join(directory, 'attempts');
  const trials = existsSync(attempts) ? readdirSync(attempts).sort().filter(id => existsSync(join(attempts, id, 'trial.json'))).map(id => read(join(attempts, id, 'trial.json'))) : [];
  writeFileSync(join(directory, 'trials.jsonl'), trials.map(t => JSON.stringify(t) + '\n').join(''));
  const reviewData = reviewFile ? read(reviewFile) : null;
  const reviews = reviewData ? validateReviews(reviewData, trials, suite) : new Map();
  const result = report(directory, manifest, suite, trials, reviews);
  if (reviewData) {
    const assessment = join(directory, 'assessments', hash(JSON.stringify(reviewData)));
    save(join(assessment, 'reviews.json'), reviewData);
    save(join(assessment, 'report.json'), result);
  }
  writeFileSync(join(directory, 'scores.jsonl'), result.scores.map(s => JSON.stringify(s) + '\n').join(''));
  writeFileSync(join(directory, 'report.md'), `# Codexia benchmark\n\nEvidence: ${result.evidence}; candidate: ${result.candidate}.\n\nQuality: **${result.quality}**. ${result.attempts} attempts, ${result.semantic_evaluated} semantically evaluated.\n\n| Case / attempt | Execution | Structural | Semantic | Evidence |\n|---|---|---|---|---|\n${trials.map((t, i) => `| ${t.attempt_id} | ${t.status} | ${result.scores[i].structural.passed} | ${result.scores[i].semantic.state} | [artifact](${t.artifact}) |`).join('\n')}\n\n${result.limitations.map(s => `- ${s}`).join('\n')}\n`);
  return result;
}

try {
  const { values, positionals } = parseArgs({ options: {
    catalog: { type: 'string', default: fileURLToPath(new URL('suites/v0.0/catalog.json', import.meta.url)) },
    output: { type: 'string' }, binary: { type: 'string', default: join(root, 'target/debug/codexia') },
    'agent-command': { type: 'string' }, repeat: { type: 'string', default: '1' }, reviews: { type: 'string' }, judgments: { type: 'string' },
    candidate: { type: 'string', default: 'A0' }, left: { type: 'string' }, right: { type: 'string' },
    mode: { type: 'string', default: 'fixed-package-reader' },
    holdout: { type: 'string' },
    calibration: { type: 'string' },
    'provider-config': { type: 'string' },
    'compile-with-provider': { type: 'boolean', default: false },
  }, allowPositionals: true });
  const command = positionals[0];
  assert.ok(positionals.length === 1 && ['validate', 'run', 'resume', 'report', 'score', 'review-template', 'prepare-review', 'prepare-calibration', 'calibrate', 'judge-calibration', 'compare', 'audit-corpus', 'export'].includes(command), 'usage: node benchmarks/runner.mjs <validate|run|resume|report|score|review-template|prepare-review|prepare-calibration|calibrate|judge-calibration|compare|audit-corpus|export> [--catalog <file>] [--output <private-directory>] [--candidate A0|A1] [--mode fixed-package-reader|cold-compile-reader] [--repeat <1..10>] [--agent-command <offline-replay-executable>] [--reviews <file>] [--judgments <file>] [--left <run>] [--right <run>] [--holdout <catalog>] [--calibration <reference.json>] [--provider-config <private-config.json>] [--compile-with-provider]');
  assert.ok(['A0', 'A1'].includes(values.candidate), 'candidate must be A0 or A1');
  assert.ok(!(values['provider-config'] && values['agent-command']), 'use either provider-config or an offline/replay agent-command');
  assert.ok(!values['compile-with-provider'] || values['provider-config'], 'compile-with-provider requires provider-config');
  assert.ok(['fixed-package-reader', 'cold-compile-reader'].includes(values.mode), 'mode must be fixed-package-reader or cold-compile-reader');
  assert.ok(!values.reviews || ['score', 'report', 'export'].includes(command), '--reviews requires score/report/export');
  if (command === 'score') assert.ok(values.reviews, 'score requires --reviews');
  const suite = loadSuite(values.catalog);
  if (command === 'audit-corpus') {
    const audit = auditCorpus(values.catalog, values.holdout, values.output);
    console.log(JSON.stringify(audit));
    if (audit.conflicts.length) process.exitCode = 1;
  }
  else if (command === 'compare') console.log(JSON.stringify(compare(values.left, values.right, values.output)));
  else if (['prepare-calibration', 'calibrate'].includes(command)) {
    const result = calibration(command, values.output, values.judgments, values.calibration);
    console.log(JSON.stringify(result));
    if (result.state === 'evaluated' && !result.calibrated) process.exitCode = 1;
  }
  else if (command === 'judge-calibration') {
    const result = await judgeCalibration(values.output, values['provider-config'], values.calibration, controller.signal);
    console.log(JSON.stringify(result));
    if (result.status !== 'completed' || (result.calibration?.state === 'evaluated' && !result.calibrated)) process.exitCode = 1;
  }
  else if (command === 'validate') {
    const provider = values['provider-config'] ? providerConfig(values['provider-config']) : null;
    console.log(JSON.stringify({ valid: true, cases: suite.cases.length, books: suite.books.size, fingerprint: suite.fingerprint, provider_config_hash: provider ? hash(JSON.stringify(provider)) : null }));
  }
  else {
    assert.equal(suite.catalog.split, 'dev', 'held-out execution requires verified isolation; unsupported');
    const repeat = Number(values.repeat);
    assert.ok(Number.isSafeInteger(repeat) && repeat > 0 && repeat <= 10, 'repeat must be 1..10');
    assert.ok(values.output || command === 'run', 'resume/report require --output');
    const directory = privatePath(values.output || join(root, 'private/benchmarks/runs', randomUUID()));
    const binary = resolve(values.binary);
    const executing = ['run', 'resume'].includes(command);
    const provider = values['provider-config'] ? providerConfig(values['provider-config']) : null;
    const agent = resolve(provider?.adapter || values['agent-command'] || join(root, 'benchmarks/adapters/extract.mjs'));
    if (executing && !provider) assert.notEqual(agent, join(root, 'scripts/online-analyzer.mjs'), 'online adapter requires --provider-config');
    const identity = executing ? { suite_hash: suite.fingerprint, execution_hash: suite.executionHash, provider_config_hash: provider ? hash(JSON.stringify(provider)) : null, binary_hash: hash(readFileSync(binary)), agent_hash: hash(readFileSync(agent)), benchmark_hash: treeHash(join(root, 'benchmarks')), registration_analyzer_hash: hash(readFileSync(join(root, 'tests/fixtures/analyzer.mjs'))) } : null;
    let manifest;
    if (command === 'run') {
      assert.ok(!existsSync(directory), 'use a fresh output directory');
      mkdirSync(dirname(directory), { recursive: true });
      mkdirSync(directory);
      const started = Date.now();
      manifest = { protocol: '0.0', ...identity, git_sha: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(), git_diff_hash: hash(execFileSync('git', ['diff', 'HEAD', '--', 'src', 'benchmarks', 'tests/fixtures/analyzer.mjs'], { cwd: root })), evidence: provider?.evidence || (values['agent-command'] ? 'replay' : 'offline'), candidate: values.candidate, mode: values.mode, compiler: values['compile-with-provider'] ? 'configured-provider' : 'offline-registration', node_version: process.version, repeat, started_at: new Date(started).toISOString(), retention: { content_expires_at: new Date(started + 7 * 86400000).toISOString(), cleanup: 'manual; no automatic deletion' }, status: 'running' };
      save(join(directory, 'manifest.json'), manifest);
      if (provider) {
        save(join(directory, 'budget/config.json'), provider);
        save(join(directory, 'budget/ledger.json'), { calls: [] });
      }
    } else {
      manifest = read(join(directory, 'manifest.json'));
      if (executing) assert.equal(manifest.candidate, values.candidate, 'candidate changed');
      if (executing) assert.equal(manifest.mode, values.mode, 'mode changed');
      if (executing) assert.equal(manifest.compiler, values['compile-with-provider'] ? 'configured-provider' : 'offline-registration', 'compiler strategy changed');
      if (executing) for (const [key, value] of Object.entries(identity)) assert.equal(manifest[key], value, `${key} changed; use a new run`);
      else if (manifest.execution_hash) assert.equal(manifest.execution_hash, suite.executionHash, 'execution inputs changed; cannot rescore different questions or sources');
      else assert.equal(manifest.suite_hash, suite.fingerprint, 'legacy run needs its original suite; execution identity unavailable');
      assert.equal(manifest.repeat, repeat, 'repeat changed');
      if (provider) assert.equal(hash(JSON.stringify(read(join(directory, 'budget/config.json')))), manifest.provider_config_hash, 'stored provider config changed');
    }
    let release;
    try {
      release = acquireLock(directory, command === 'resume');
      if (['run', 'resume'].includes(command)) {
        for (const book of suite.books.values()) {
          if (controller.signal.aborted) break;
          const sharedCompilation = join(directory, 'packages', book.id);
          const compileBudget = values['compile-with-provider'] ? join(directory, 'budget') : '';
          if (manifest.mode === 'fixed-package-reader' && suite.cases.some(c => c.book_id === book.id && c.applicability !== 'not_applicable')) {
            if (!existsSync(join(sharedCompilation, 'compile.json'))) compile(book, sharedCompilation, binary, 120000, compileBudget);
            assert.equal(treeHash(join(sharedCompilation, 'package')), read(join(sharedCompilation, 'compile.json')).package_hash, 'package changed');
          }
          for (const item of suite.cases.filter(item => item.book_id === book.id)) {
            for (let n = 1; n <= repeat; n++) {
              if (controller.signal.aborted) break;
              const id = `${item.id}-${n}`;
              const attempt = join(directory, 'attempts', id);
              if (existsSync(join(attempt, 'trial.json'))) continue;
              const interrupted = existsSync(attempt);
              mkdirSync(attempt, { recursive: true });
              const started = performance.now();
              let artifact;
              if (interrupted) {
                if (existsSync(join(attempt, 'artifact.json'))) artifact = read(join(attempt, 'artifact.json'));
                else {
                  const compileCalls = manifest.mode === 'cold-compile-reader' ? collectCalls(join(attempt, 'compilation')).map(({ request, output, ...event }) => event) : [];
                  artifact = { status: 'cancelled', error: 'Interrupted attempt retained; start a new run for a fresh attempt.', steps: [], calls: [...compileCalls, ...collectCalls(attempt)] };
                }
              }
              else if (item.applicability === 'not_applicable') artifact = { status: 'not_applicable', steps: [], calls: [] };
              else {
                const compilation = manifest.mode === 'cold-compile-reader' ? join(attempt, 'compilation') : sharedCompilation;
                let compilationData;
                try {
                  if (manifest.mode === 'cold-compile-reader') compile(book, compilation, binary, item.budget.timeout_ms, compileBudget, item.budget.max_calls);
                  compilationData = read(join(compilation, 'compile.json'));
                  const remaining = manifest.mode === 'cold-compile-reader' ? Math.floor(item.budget.timeout_ms - (performance.now() - started)) : item.budget.timeout_ms;
                  if (remaining <= 0) throw Object.assign(new Error('attempt timeout'), { code: 'ETIMEDOUT' });
                  const compileCalls = manifest.mode === 'cold-compile-reader' ? collectCalls(compilation).length : 0;
                  const readerStarted = performance.now();
                  artifact = await execute({ item: { ...item, budget: { ...item.budget, timeout_ms: remaining, max_calls: Math.max(0, item.budget.max_calls - compileCalls) } }, book, pkg: join(compilation, 'package'), directory: attempt, binary, agent, signal: controller.signal, candidate: values.candidate, budgetDirectory: provider ? join(directory, 'budget') : '' });
                  artifact.timing = { reader_ms: Math.round(performance.now() - readerStarted) };
                }
                catch (error) {
                  artifact = { status: controller.signal.aborted ? 'cancelled' : error.code === 'ETIMEDOUT' ? 'timed_out' : 'provider_error', error: error.message, steps: [], calls: collectCalls(attempt), timing: { reader_ms: null } };
                  if (existsSync(join(compilation, 'budget-exceeded.json'))) artifact.status = 'budget_exceeded';
                }
                if (manifest.mode === 'cold-compile-reader') artifact.calls = [...collectCalls(compilation).map(({ request, output, ...event }) => event), ...artifact.calls];
                artifact.compilation = { ...(compilationData || { mode: manifest.compiler, status: 'failed', duration_ms: Math.round(performance.now() - started), source_bytes: null, package_bytes: null, analysis_bytes: null, usage: null, estimated_usd: null }), id: manifest.mode === 'cold-compile-reader' ? id : book.id };
              }
              save(join(attempt, 'artifact.json'), artifact);
              const trial = { protocol: '0.0', attempt_id: id, case_id: item.id, status: artifact.status, duration_ms: Math.round(performance.now() - started), artifact: `attempts/${id}/artifact.json`, artifact_hash: hash(readFileSync(join(attempt, 'artifact.json'))), call_count: artifact.calls.length, known_usage_calls: artifact.calls.filter(c => usageRecord(c.usage)).length };
              save(join(attempt, 'trial.json'), trial);
              summarize(directory, manifest, suite);
            }
          }
        }
        manifest.status = controller.signal.aborted ? 'cancelled' : 'completed';
        save(join(directory, 'manifest.json'), manifest);
      }
      const result = summarize(directory, manifest, suite, values.reviews);
      let reviewPackets;
      if (command === 'prepare-review') {
        const trials = readFileSync(join(directory, 'trials.jsonl'), 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse);
        reviewPackets = preparePackets(directory, trials, suite);
      }
      if (command === 'review-template') {
        const target = join(directory, 'review-template.json');
        assert.ok(!existsSync(target), 'review template already exists; do not overwrite review work');
        const trials = readFileSync(join(directory, 'trials.jsonl'), 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse);
        save(target, template(trials, suite));
      }
      if (command === 'export') {
        save(join(directory, 'export-summary.json'), { protocol: result.protocol, evidence: result.evidence, quality: result.quality, attempts: result.attempts, independent_cases: result.independent_cases, independent_books: result.independent_books, execution: result.execution, structural_passes: result.structural_passes, hard_failures: result.hard_failures, semantic_evaluated: result.semantic_evaluated, task_successes: result.task_successes, usage_coverage: result.usage_coverage });
      }
      console.log(JSON.stringify({ output: directory, attempts: result.attempts, quality: result.quality, evidence: result.evidence, review_packets: reviewPackets }));
      if (result.quality === 'fail' || controller.signal.aborted) process.exitCode = 1;
    } catch (error) {
      if (release && ['run', 'resume'].includes(command)) {
        manifest.status = 'failed';
        manifest.error = error.message;
        save(join(directory, 'manifest.json'), manifest);
        summarize(directory, manifest, suite);
      }
      throw error;
    } finally {
      release?.();
    }
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
