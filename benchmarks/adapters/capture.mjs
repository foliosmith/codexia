#!/usr/bin/env node
import { mkdirSync, readdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { reserveInvocation, finishInvocation, usageRecord } from '../budget.mjs';

process.umask(0o077);
const directory = process.env.CODEXIA_BENCH_ATTEMPT;
const calls = join(directory, 'calls');
mkdirSync(calls, { recursive: true });
if (readdirSync(calls).length >= Number(process.env.CODEXIA_BENCH_MAX_CALLS)) {
  writeFileSync(join(directory, 'budget-exceeded.json'), JSON.stringify({ reason: 'max_calls' }));
  process.exit(1);
}
const id = randomUUID();
const call = join(calls, id);
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
let input = Buffer.concat(chunks);
const request = JSON.parse(input);
const phase = ['chapter_analysis', 'chapter_reanalysis', 'book_synthesis'].includes(request.task) ? 'compile' : 'answer';
if (phase === 'answer' && process.env.CODEXIA_BENCH_CANDIDATE === 'A1') {
  request.context.chapter_analysis = null;
  request.context.related_concepts = [];
  request.context.argument_flow = [];
  delete request.input.expected_points;
  input = Buffer.from(JSON.stringify(request));
}
const budgetDirectory = process.env.CODEXIA_BENCH_BUDGET_DIR;
let provider;
if (budgetDirectory) {
  const reservation = reserveInvocation(budgetDirectory, id, input.length, phase);
  if (!reservation.allowed) {
    writeFileSync(join(directory, 'budget-exceeded.json'), JSON.stringify({ reason: reservation.reason }));
    process.exit(1);
  }
  provider = reservation.config;
}
mkdirSync(call);
writeFileSync(join(call, 'request.json'), input);
writeFileSync(join(call, 'event.json'), JSON.stringify({ id, phase, status: 'started', usage: null }));
const started = performance.now();
const result = spawnSync(process.env.CODEXIA_BENCH_AGENT, [], {
  input, timeout: Number(process.env.CODEXIA_READER_TIMEOUT_MS), maxBuffer: 4 * 1024 * 1024,
  env: { ...process.env, CODEXIA_ONLINE_RUN_DIR: call, ...(provider ? { CODEXIA_ANALYZER_MODEL: provider.model, CODEXIA_ANALYZER_PROMPT_VERSION: undefined, CODEXIA_CAPTURE_CONTENT: '0' } : {}) },
});
writeFileSync(join(call, 'output.txt'), result.stdout || '');
let usage = null;
if (existsSync(process.env.CODEXIA_USAGE_FILE)) {
  try { usage = usageRecord(JSON.parse(readFileSync(process.env.CODEXIA_USAGE_FILE))); } catch { /* Invalid usage remains unknown. */ }
}
const status = result.error?.code === 'ETIMEDOUT' ? 'timed_out' : result.status === 0 ? 'completed' : 'provider_error';
writeFileSync(join(call, 'event.json'), JSON.stringify({ id, phase, status, duration_ms: Math.round(performance.now() - started), usage, input_bytes: input.length, model: provider?.model ?? null, evidence: provider?.evidence ?? 'offline-or-replay' }));
if (budgetDirectory) finishInvocation(budgetDirectory, id, status, usage);
if (result.status !== 0) process.exit(1);
process.stdout.write(result.stdout);
