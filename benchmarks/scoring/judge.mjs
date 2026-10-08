import assert from 'node:assert/strict';
import { existsSync, mkdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { setImmediate } from 'node:timers/promises';
import { privatePath, save, treeHash } from '../storage.mjs';
import { hash, read, root } from '../data.mjs';
import { providerConfig, reserveInvocation, finishInvocation, usageRecord, budgetSummary } from '../budget.mjs';
import { calibration } from './calibration.mjs';

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
  judgments.judge_version = hash(JSON.stringify({ config, benchmark_hash: treeHash(join(root, 'benchmarks')) }));
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
    const id = randomUUID();
    const call = join(directory, 'calls', id);
    const request = { task: 'semantic_judge', instruction: inputs.instructions, input: sample, output_schema: { score: 'integer 0..3', verdict: 'supports|refutes|insufficient', evidence: 'string', hard_failure: true } };
    const input = JSON.stringify(request);
    const reservation = reserveInvocation(budget, id, Buffer.byteLength(input), 'judge');
    if (!reservation.allowed) { status = 'budget_exceeded'; save(join(directory, 'stop.json'), { reason: reservation.reason }); break; }
    save(join(call, 'request.json'), request);
    const usageFile = join(call, 'usage.json');
    const result = spawnSync(config.adapter, [], { input, encoding: 'utf8', timeout: 60000, maxBuffer: 1024 * 1024,
      env: { ...process.env, CODEXIA_ANALYZER_MODEL: config.model, CODEXIA_ANALYZER_PROMPT_VERSION: undefined, CODEXIA_CAPTURE_CONTENT: '0', CODEXIA_USAGE_FILE: usageFile, CODEXIA_ONLINE_RUN_DIR: call },
    });
    let usage = null;
    try { usage = usageRecord(read(usageFile)); } catch { /* Missing usage remains unknown and stops dispatch. */ }
    status = result.error?.code === 'ETIMEDOUT' ? 'timed_out' : result.status === 0 ? 'completed' : 'provider_error';
    let judgment;
    if (status === 'completed') {
      try {
        judgment = JSON.parse(result.stdout);
        assert.ok(Number.isInteger(judgment.score) && judgment.score >= 0 && judgment.score <= 3);
        assert.ok(['supports', 'refutes', 'insufficient'].includes(judgment.verdict));
        assert.ok(typeof judgment.evidence === 'string' && judgment.evidence.trim());
        assert.equal(typeof judgment.hard_failure, 'boolean');
        judgment = { id: sample.id, score: judgment.score, verdict: judgment.verdict, evidence: judgment.evidence, hard_failure: judgment.hard_failure };
      } catch { status = 'invalid_output'; }
    }
    finishInvocation(budget, id, status, usage);
    save(join(call, 'result.json'), { status, usage, output: result.stdout || null });
    if (status === 'completed') judgments.judgments.push(judgment);
    if (!usage && status === 'completed') status = 'unknown_usage';
    persist();
    if (status !== 'completed') break;
  }
  if (status === 'completed' && judgments.judgments.length === inputs.samples.length) {
    assessment = calibration('calibrate', join(directory, 'assessment'), join(directory, 'judgments.json'), referencePath);
  }
  return persist();
}
