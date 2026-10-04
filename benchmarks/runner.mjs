#!/usr/bin/env node
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, readFileSync, writeFileSync, readdirSync, openSync, closeSync, unlinkSync } from 'node:fs';
import { join, dirname, resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { loadSuite, root, hash, read } from './data.mjs';
import { privatePath, save, treeHash } from './storage.mjs';
import { compile, execute } from './adapters/a0.mjs';
import { report } from './scoring/score.mjs';
import { template, validateReviews } from './scoring/review.mjs';

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
    'agent-command': { type: 'string' }, repeat: { type: 'string', default: '1' }, reviews: { type: 'string' },
  }, allowPositionals: true });
  const command = positionals[0];
  assert.ok(positionals.length === 1 && ['validate', 'run', 'resume', 'report', 'score', 'review-template', 'export'].includes(command), 'usage: node benchmarks/runner.mjs <validate|run|resume|report|score|review-template|export> [--catalog <file>] [--output <private-directory>] [--repeat <1..10>] [--agent-command <offline-replay-executable>] [--reviews <file>]');
  assert.ok(!values.reviews || ['score', 'report', 'export'].includes(command), '--reviews requires score/report/export');
  if (command === 'score') assert.ok(values.reviews, 'score requires --reviews');
  const suite = loadSuite(values.catalog);
  if (command === 'validate') console.log(JSON.stringify({ valid: true, cases: suite.cases.length, books: suite.books.size, fingerprint: suite.fingerprint }));
  else {
    assert.equal(suite.catalog.split, 'dev', 'held-out execution requires verified isolation; unsupported');
    const repeat = Number(values.repeat);
    assert.ok(Number.isSafeInteger(repeat) && repeat > 0 && repeat <= 10, 'repeat must be 1..10');
    assert.ok(values.output || command === 'run', 'resume/report require --output');
    const directory = privatePath(values.output || join(root, 'private/benchmarks/runs', randomUUID()));
    const binary = resolve(values.binary);
    const agent = resolve(values['agent-command'] || join(root, 'benchmarks/adapters/extract.mjs'));
    const identity = { suite_hash: suite.fingerprint, binary_hash: hash(readFileSync(binary)), agent_hash: hash(readFileSync(agent)), benchmark_hash: treeHash(join(root, 'benchmarks')), registration_analyzer_hash: hash(readFileSync(join(root, 'tests/fixtures/analyzer.mjs'))) };
    let manifest;
    if (command === 'run') {
      assert.ok(!existsSync(directory), 'use a fresh output directory');
      mkdirSync(dirname(directory), { recursive: true });
      mkdirSync(directory);
      manifest = { protocol: '0.0', ...identity, git_sha: execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim(), git_diff_hash: hash(execFileSync('git', ['diff', 'HEAD', '--', 'src', 'benchmarks', 'tests/fixtures/analyzer.mjs'], { cwd: root })), evidence: values['agent-command'] ? 'replay' : 'offline', candidate: 'A0', mode: 'fixed-package-reader', compiler: 'offline-registration', node_version: process.version, repeat, started_at: new Date().toISOString(), status: 'running' };
      save(join(directory, 'manifest.json'), manifest);
    } else {
      manifest = read(join(directory, 'manifest.json'));
      for (const [key, value] of Object.entries(identity)) assert.equal(manifest[key], value, `${key} changed; use a new run`);
      assert.equal(manifest.repeat, repeat, 'repeat changed');
    }
    const lock = join(directory, '.lock');
    let fd;
    try {
      fd = openSync(lock, 'wx', 0o600);
      writeFileSync(fd, JSON.stringify({ pid: process.pid }));
      if (['run', 'resume'].includes(command)) {
        for (const book of suite.books.values()) {
          if (controller.signal.aborted) break;
          const compilation = join(directory, 'packages', book.id);
          const pkg = join(compilation, 'package');
          if (!existsSync(join(compilation, 'compile.json'))) compile(book, compilation, binary);
          assert.equal(treeHash(pkg), read(join(compilation, 'compile.json')).package_hash, 'package changed');
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
              if (interrupted) artifact = { status: 'cancelled', error: 'Interrupted attempt retained; start a new run for a fresh attempt.', steps: [], calls: [] };
              else if (item.applicability === 'not_applicable') artifact = { status: 'not_applicable', steps: [], calls: [] };
              else {
                try { artifact = await execute({ item, book, pkg, directory: attempt, binary, agent, signal: controller.signal }); }
                catch (error) { artifact = { status: controller.signal.aborted ? 'cancelled' : 'provider_error', error: error.message, steps: [], calls: [] }; }
              }
              save(join(attempt, 'artifact.json'), artifact);
              const trial = { protocol: '0.0', attempt_id: id, case_id: item.id, status: artifact.status, duration_ms: Math.round(performance.now() - started), artifact: `attempts/${id}/artifact.json`, artifact_hash: hash(readFileSync(join(attempt, 'artifact.json'))), call_count: artifact.calls.length, known_usage_calls: artifact.calls.filter(c => c.usage !== null).length };
              save(join(attempt, 'trial.json'), trial);
              summarize(directory, manifest, suite);
            }
          }
        }
        manifest.status = controller.signal.aborted ? 'cancelled' : 'completed';
        save(join(directory, 'manifest.json'), manifest);
      }
      const result = summarize(directory, manifest, suite, values.reviews);
      if (command === 'review-template') {
        const target = join(directory, 'review-template.json');
        assert.ok(!existsSync(target), 'review template already exists; do not overwrite review work');
        const trials = readFileSync(join(directory, 'trials.jsonl'), 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse);
        save(target, template(trials, suite));
      }
      if (command === 'export') {
        save(join(directory, 'export-summary.json'), { protocol: result.protocol, evidence: result.evidence, quality: result.quality, attempts: result.attempts, independent_cases: result.independent_cases, independent_books: result.independent_books, execution: result.execution, structural_passes: result.structural_passes, hard_failures: result.hard_failures, semantic_evaluated: result.semantic_evaluated, task_successes: result.task_successes, usage_coverage: result.usage_coverage });
      }
      console.log(JSON.stringify({ output: directory, attempts: result.attempts, quality: result.quality, evidence: result.evidence }));
      if (result.quality === 'fail' || controller.signal.aborted) process.exitCode = 1;
    } catch (error) {
      if (fd !== undefined && ['run', 'resume'].includes(command)) {
        manifest.status = 'failed';
        save(join(directory, 'manifest.json'), manifest);
        summarize(directory, manifest, suite);
      }
      throw error;
    } finally {
      if (fd !== undefined) { closeSync(fd); unlinkSync(lock); }
    }
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
