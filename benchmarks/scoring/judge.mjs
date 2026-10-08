import assert from 'node:assert/strict';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { setImmediate } from 'node:timers/promises';
import { privatePath, save, treeHash, acquireLock } from '../storage.mjs';
import { hash, read, root, lines } from '../data.mjs';
import { providerConfig, reserveInvocation, finishInvocation, usageRecord, budgetSummary } from '../budget.mjs';
import { calibration } from './calibration.mjs';
import { preparePackets, template, validateReviews } from './review.mjs';

const judgeIdentity = config => hash(JSON.stringify({ config, benchmark_hash: treeHash(join(root, 'benchmarks')) }));

export async function judgeCalibration(output, configPath, referencePath, signal) {
  assert.ok(output && configPath, 'judge-calibration requires --output and --provider-config');
  const directory = privatePath(output);
  const config = providerConfig(configPath);
  assert.ok(!existsSync(directory), 'use a fresh judge output; existing calls are never replayed');
  mkdirSync(dirname(directory), { recursive: true, mode: 0o700 });
  mkdirSync(directory, { mode: 0o700 });
  const prepared = join(directory, 'inputs');
  calibration('prepare-calibration', prepared, undefined, referencePath);
  const inputs = read(join(prepared, 'inputs.json'));
  const judgments = read(join(prepared, 'judgments-template.json'));
  judgments.judge_version = judgeIdentity(config);
  judgments.judgments = [];
  const budget = join(directory, 'budget');
  save(join(budget, 'config.json'), config);
  save(join(budget, 'ledger.json'), { calls: [] });
  let status = 'running';
  let assessment = null;
  function persist() {
    save(join(directory, 'judgments.json'), judgments);
    const result = { status, evidence: config.evidence, judge_version: judgments.judge_version, expected_samples: inputs.samples.length, evaluated_samples: judgments.judgments.length, calibrated: assessment?.calibrated ?? false, calibration: assessment, budget: budgetSummary(read(join(budget, 'ledger.json')), 'judge'), hard_spend_cap: false };
    save(join(directory, 'report.json'), result);
    return result;
  }
  persist();
  for (const sample of inputs.samples) {
    await setImmediate();
    if (signal?.aborted) { status = 'cancelled'; break; }
    const request = { task: 'semantic_judge', instruction: inputs.instructions, input: sample, output_schema: { score: 'integer 0..3', verdict: 'supports|refutes|insufficient', evidence: 'string', hard_failure: true } };
    const result = dispatch(directory, config, request, judgment => {
      assert.ok(Number.isInteger(judgment.score) && judgment.score >= 0 && judgment.score <= 3);
      assert.ok(['supports', 'refutes', 'insufficient'].includes(judgment.verdict));
      assert.ok(typeof judgment.evidence === 'string' && judgment.evidence.trim());
      assert.equal(typeof judgment.hard_failure, 'boolean');
      return { id: sample.id, score: judgment.score, verdict: judgment.verdict, evidence: judgment.evidence, hard_failure: judgment.hard_failure };
    });
    status = result.status;
    if (result.value) judgments.judgments.push(result.value);
    persist();
    if (status !== 'completed') break;
  }
  if (status === 'completed' && judgments.judgments.length === inputs.samples.length) {
    assessment = calibration('calibrate', join(directory, 'assessment'), join(directory, 'judgments.json'), referencePath);
  }
  return persist();
}

function dispatch(directory, config, request, validate) {
  const budget = join(directory, 'budget');
  const id = randomUUID();
  const call = join(directory, 'calls', id);
  const input = JSON.stringify(request);
  const reservation = reserveInvocation(budget, id, Buffer.byteLength(input), 'judge');
  if (!reservation.allowed) {
    save(join(directory, 'stop.json'), { reason: reservation.reason });
    return { status: 'budget_exceeded' };
  }
  save(join(call, 'request.json'), request);
  const usageFile = join(call, 'usage.json');
  const result = spawnSync(config.adapter, [], { input, encoding: 'utf8', timeout: 60000, maxBuffer: 1024 * 1024,
    env: { ...process.env, CODEXIA_ANALYZER_MODEL: config.model, CODEXIA_ANALYZER_PROMPT_VERSION: undefined, CODEXIA_CAPTURE_CONTENT: '0', CODEXIA_USAGE_FILE: usageFile, CODEXIA_ONLINE_RUN_DIR: call },
  });
  let usage = null;
  try { usage = usageRecord(read(usageFile)); } catch { /* Unknown usage stops dispatch. */ }
  let status = result.error?.code === 'ETIMEDOUT' ? 'timed_out' : result.status === 0 ? 'completed' : 'provider_error';
  let value;
  if (status === 'completed') {
    try { value = validate(JSON.parse(result.stdout)); } catch { status = 'invalid_output'; }
  }
  finishInvocation(budget, id, status, usage);
  save(join(call, 'result.json'), { status, usage, output: result.stdout || null });
  if (!usage && status === 'completed') status = 'unknown_usage';
  return { status, value };
}

export async function judgeCandidates(output, configPath, referencePath, calibrationRun, source, suite, signal) {
  assert.ok(output && configPath && calibrationRun && source, 'judge requires --output, --provider-config, --calibration-run and --left');
  const directory = privatePath(output);
  source = privatePath(source);
  calibrationRun = privatePath(calibrationRun);
  const config = providerConfig(configPath);
  const version = judgeIdentity(config);
  const prior = read(join(calibrationRun, 'report.json'));
  assert.equal(prior.judge_version, version, 'judge identity changed; recalibrate');
  assert.ok(prior.status === 'completed' && prior.calibrated, 'passing calibration required');
  assert.equal(suite.catalog.split, 'dev', 'held-out judging requires verified isolation; unsupported');
  const manifest = read(join(source, 'manifest.json'));
  assert.equal(manifest.execution_hash, suite.executionHash, 'execution inputs changed');
  const trials = lines(join(source, 'trials.jsonl'));
  const reviews = template(trials, suite);
  assert.ok(reviews.reviews.length, 'no completed attempts to judge');
  for (const review of reviews.reviews) {
    const trial = trials.find(t => t.attempt_id === review.attempt_id);
    assert.equal(suite.gold.get(trial.case_id).status, 'reviewed', 'gold is draft; independent review required');
  }
  assert.ok(!existsSync(directory), 'use a fresh judge output; existing calls are never replayed');
  mkdirSync(dirname(directory), { recursive: true, mode: 0o700 });
  mkdirSync(directory, { mode: 0o700 });
  const checked = calibration('calibrate', join(directory, 'calibration-check'), join(calibrationRun, 'judgments.json'), referencePath);
  assert.equal(checked.judge_version, version, 'calibration judgments identity changed');
  assert.ok(checked.calibrated, 'reference calibration no longer passes');
  const release = acquireLock(source, false);
  try {
    const packets = preparePackets(source, trials, suite);
    const bindings = read(join(packets, 'bindings.json'));
    const budget = join(directory, 'budget');
    save(join(budget, 'config.json'), config);
    save(join(budget, 'ledger.json'), { calls: [] });
    const accepted = { version: '0.0', reviews: [] };
    let status = 'running';
    function persist() {
      const result = { status, evidence: config.evidence, judge_version: version, calibration_hash: checked.calibration_hash, expected_attempts: bindings.length, evaluated_attempts: accepted.reviews.length, budget: budgetSummary(read(join(budget, 'ledger.json')), 'judge'), hard_spend_cap: false };
      save(join(directory, 'report.json'), result);
      save(join(directory, 'partial-reviews.json'), accepted);
      return result;
    }
    persist();
    for (const binding of bindings) {
      await setImmediate();
      if (signal?.aborted) { status = 'cancelled'; break; }
      const packet = read(join(packets, `${binding.packet_id}.json`));
      assert.equal(hash(JSON.stringify(packet)), binding.packet_hash, 'review packet changed');
      const base = reviews.reviews.find(r => r.attempt_id === binding.attempt_id);
      const request = { task: 'semantic_judge', instruction: `${packet.instructions} Score each dimension: 0 wrong/unsupported, 1 materially incomplete, 2 minor omissions, 3 fully supported. Return each dimension exactly once with source evidence and explicit hard_failure.`, input: packet,
        output_schema: { checks: packet.dimensions.map(dimension => ({ dimension, score: 'integer 0..3', verdict: 'supports|refutes|insufficient', evidence: 'string', hard_failure: true })) } };
      const result = dispatch(directory, config, request, value => {
        const review = { ...base, reviewer: `automatic:${config.evidence}`, judge_version: version, checks: value.checks };
        validateReviews({ version: '0.0', reviews: [review] }, trials, suite);
        return review;
      });
      status = result.status;
      if (status === 'completed') accepted.reviews.push(result.value);
      persist();
      if (status !== 'completed') break;
    }
    if (status === 'completed' && accepted.reviews.length === bindings.length) save(join(directory, 'reviews.json'), accepted);
    return persist();
  } finally { release(); }
}
