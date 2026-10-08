import assert from 'node:assert/strict';
import { dirname, resolve, join } from 'node:path';
import { readFileSync } from 'node:fs';
import { hash, read } from './data.mjs';
import { privatePath, save, acquireLock } from './storage.mjs';

export function providerConfig(path) {
  path = privatePath(path);
  const value = read(path);
  const keys = ['evidence', 'adapter', 'model', 'max_invocations', 'max_request_bytes', 'stop_after_input_tokens', 'stop_after_output_tokens', 'stop_after_estimated_usd', 'pricing'];
  assert.deepEqual(Object.keys(value).sort(), keys.sort(), 'provider config must contain only the documented fields; never store credentials');
  assert.ok(['online', 'simulation'].includes(value.evidence), 'provider evidence must be online or simulation');
  assert.ok(typeof value.model === 'string' && value.model.trim() && value.model.length <= 128 && !/[\r\n]/.test(value.model), 'model required');
  for (const key of ['max_invocations', 'max_request_bytes', 'stop_after_input_tokens', 'stop_after_output_tokens']) assert.ok(Number.isSafeInteger(value[key]) && value[key] > 0, `${key} must be a positive integer`);
  assert.ok(value.max_request_bytes <= 524288, 'request budget exceeds runtime byte limit');
  assert.ok(Number.isFinite(value.stop_after_estimated_usd) && value.stop_after_estimated_usd > 0, 'positive estimated cost stop threshold required');
  const pricing = value.pricing;
  assert.ok(pricing && typeof pricing === 'object', 'pricing required');
  assert.deepEqual(Object.keys(pricing).sort(), ['source', 'as_of', 'input_per_million', 'cached_input_per_million', 'output_per_million'].sort(), 'unsupported pricing fields');
  assert.ok(typeof pricing.source === 'string' && pricing.source.trim(), 'pricing source required');
  assert.ok(typeof pricing.as_of === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(pricing.as_of), 'pricing snapshot date required');
  for (const key of ['input_per_million', 'cached_input_per_million', 'output_per_million']) assert.ok(Number.isFinite(pricing[key]) && pricing[key] >= 0, 'pricing rates must be nonnegative finite numbers');
  assert.ok(pricing.cached_input_per_million <= pricing.input_per_million, 'cached rate cannot exceed input rate in this pricing contract');
  assert.ok(typeof value.adapter === 'string' && value.adapter.trim(), 'provider adapter required');
  const adapter = resolve(dirname(path), value.adapter);
  return { ...value, adapter, adapter_hash: hash(readFileSync(adapter)) };
}

export function usageRecord(value) {
  if (!value || !['input_tokens', 'output_tokens'].every(k => Number.isSafeInteger(value[k]) && value[k] >= 0)) return null;
  const cached = value.cached_input_tokens ?? 0;
  if (!Number.isSafeInteger(cached) || cached < 0 || cached > value.input_tokens || (value.cache_write_input_tokens ?? 0) !== 0) return null;
  return { input_tokens: value.input_tokens, cached_input_tokens: cached, output_tokens: value.output_tokens };
}

export function cost(usage, pricing) {
  const u = usageRecord(usage);
  if (!u) return null;
  const value = ((u.input_tokens - u.cached_input_tokens) * pricing.input_per_million + u.cached_input_tokens * pricing.cached_input_per_million + u.output_tokens * pricing.output_per_million) / 1000000;
  return Number.isFinite(value) ? value : null;
}

export function budgetSummary(ledger, phase) {
  const calls = phase ? ledger.calls.filter(c => (c.phase ?? 'answer') === phase) : ledger.calls;
  const known = calls.filter(call => usageRecord(call.usage) && Number.isFinite(call.estimated_usd));
  return { invocations: calls.length, known_usage_calls: known.length, unknown_usage_calls: calls.length - known.length, input_tokens: known.reduce((n, c) => n + c.usage.input_tokens, 0), output_tokens: known.reduce((n, c) => n + c.usage.output_tokens, 0), known_estimated_usd: known.reduce((n, c) => n + c.estimated_usd, 0) };
}

export function reserveInvocation(directory, id, inputBytes, phase = 'answer') {
  assert.ok(['compile', 'answer', 'judge'].includes(phase), 'unsupported provider phase');
  const release = acquireLock(directory, true);
  try {
    const config = read(join(directory, 'config.json'));
    const ledger = read(join(directory, 'ledger.json'));
    assert.ok(!ledger.calls.some(c => c.id === id), 'duplicate invocation');
    const summary = budgetSummary(ledger);
    let reason = null;
    if (inputBytes > config.max_request_bytes) reason = 'request_bytes';
    else if (summary.invocations >= config.max_invocations) reason = 'invocation_limit';
    else if (summary.unknown_usage_calls) reason = 'unaccounted_invocation';
    else if (summary.input_tokens >= config.stop_after_input_tokens) reason = 'reported_input_token_threshold';
    else if (summary.output_tokens >= config.stop_after_output_tokens) reason = 'reported_output_token_threshold';
    else if (summary.known_estimated_usd >= config.stop_after_estimated_usd) reason = 'estimated_cost_threshold';
    if (reason) return { allowed: false, reason };
    ledger.calls.push({ id, phase, status: 'reserved', usage: null, estimated_usd: null, input_bytes: inputBytes });
    save(join(directory, 'ledger.json'), ledger);
    return { allowed: true, config };
  } finally { release(); }
}

export function finishInvocation(directory, id, status, usage) {
  const release = acquireLock(directory, true);
  try {
    const config = read(join(directory, 'config.json'));
    const ledger = read(join(directory, 'ledger.json'));
    const call = ledger.calls.find(c => c.id === id);
    assert.ok(call && call.status === 'reserved', 'invocation already settled or missing');
    Object.assign(call, { status, usage: usageRecord(usage), estimated_usd: cost(usage, config.pricing) });
    save(join(directory, 'ledger.json'), ledger);
  } finally { release(); }
}
