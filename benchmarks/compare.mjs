import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { hash, lines, read } from './data.mjs';
import { privatePath, save } from './storage.mjs';

function load(directory) {
  directory = privatePath(directory);
  const manifest = read(join(directory, 'manifest.json'));
  const report = read(join(directory, 'report.json'));
  assert.equal(manifest.status, 'completed', 'comparison requires finished runs');
  assert.equal(report.missing_attempts, 0, 'comparison requires all planned attempts, including failures');
  const trials = lines(join(directory, 'trials.jsonl'));
  assert.equal(trials.length, report.attempts, 'stale report attempt count');
  const scores = new Map();
  for (const trial of trials) {
    assert.equal(trial.artifact, `attempts/${trial.attempt_id}/artifact.json`, 'invalid comparison artifact path');
    const bytes = readFileSync(privatePath(join(directory, trial.artifact)));
    assert.equal(hash(bytes), trial.artifact_hash, 'comparison artifact changed');
    const score = report.scores.find(s => s.attempt_id === trial.attempt_id);
    assert.ok(score && !scores.has(trial.attempt_id), 'missing or duplicate comparison score');
    assert.equal(score.artifact_hash, trial.artifact_hash, 'stale comparison score');
    scores.set(trial.attempt_id, score);
  }
  return { manifest, report, scores, report_hash: hash(readFileSync(join(directory, 'report.json'))) };
}

export function compare(leftPath, rightPath, output) {
  assert.ok(leftPath && rightPath && output, 'compare needs --left, --right and --output');
  const directory = privatePath(output);
  assert.ok(!existsSync(directory), 'use a fresh comparison output directory');
  const left = load(leftPath);
  const right = load(rightPath);
  for (const field of ['protocol', 'evidence', 'mode', 'compiler', 'repeat', 'agent_hash', 'registration_analyzer_hash']) assert.equal(left.manifest[field], right.manifest[field], `incomparable ${field}`);
  assert.equal(left.manifest.provider_config_hash ?? null, right.manifest.provider_config_hash ?? null, 'incomparable provider model, pricing or limits');
  for (const field of ['suite_hash', 'execution_hash', 'scorer_hash']) assert.ok(left.report[field] && left.report[field] === right.report[field], `incomparable ${field}; rescore both runs consistently`);
  assert.deepEqual([...left.scores.keys()].sort(), [...right.scores.keys()].sort(), 'incomparable attempt sets');
  const judgeVersions = run => [...new Set([...run.scores.values()].filter(s => s.semantic.state === 'evaluated').map(s => s.judge_version))].sort();
  assert.deepEqual(judgeVersions(left), judgeVersions(right), 'incomparable judge versions; rescore both runs');
  const pairs = [...left.scores].map(([id, a]) => {
    const b = right.scores.get(id);
    const known = typeof a.task_success === 'boolean' && typeof b.task_success === 'boolean';
    return { attempt_id: id, case_id: a.case_id, left_success: a.task_success, right_success: b.task_success, change: !known ? 'unassessed' : a.task_success === b.task_success ? 'unchanged' : b.task_success ? 'improved' : 'regressed', left_hard_failures: a.hard_failures, right_hard_failures: b.hard_failures };
  });
  const counts = Object.fromEntries(['improved', 'regressed', 'unchanged', 'unassessed'].map(change => [change, pairs.filter(p => p.change === change).length]));
  const result = { protocol: '0.0', evidence: left.manifest.evidence, left: { candidate: left.manifest.candidate, report_hash: left.report_hash }, right: { candidate: right.manifest.candidate, report_hash: right.report_hash }, paired_attempts: pairs.length, independent_cases: new Set(pairs.map(p => p.case_id)).size, counts, conclusion: 'inconclusive', reason: 'Paired descriptive evidence only; offline/replay, unreviewed tasks and small book samples do not establish model superiority.', pairs };
  save(join(directory, 'comparison.json'), result);
  return result;
}
