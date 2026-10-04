import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { hash, check } from '../data.mjs';
import { save } from '../storage.mjs';
import { applyReview } from './review.mjs';

export function score(trial, item, artifact) {
  const failures = [];
  if (trial.status !== 'completed') failures.push(`execution:${trial.status}`);
  if (artifact.calls.length > item.budget.max_calls) failures.push('call_budget');
  if (trial.status === 'completed' && artifact.steps.length !== item.steps.length) failures.push('missing_steps');
  for (const step of artifact.steps) {
    const cards = step.response?.cards;
    if (!cards?.length) { failures.push('empty_cards'); continue; }
    for (const card of cards) {
      const field = { ask: 'answer', explain: 'explanation', reflect: 'feedback' }[step.task];
      if (typeof card.content?.[field] !== 'string' || !card.content[field].trim()) failures.push('empty_output');
      for (const ref of card.source_refs || []) {
        const block = artifact.blocks?.[ref.block_id];
        if (!block || block.text_fingerprint !== ref.text_fingerprint) failures.push('source_identity');
        else if (!Number.isSafeInteger(ref.start_char) || !Number.isSafeInteger(ref.end_char) || ref.start_char < 0 || ref.end_char <= ref.start_char || ref.end_char > [...block.text].length) failures.push('citation_range');
        else if (ref.end_char > (step.limits[ref.block_id] ?? 0)) failures.push('reading_scope');
      }
    }
  }
  const na = item.applicability === 'not_applicable';
  return { attempt_id: trial.attempt_id, case_id: item.id, artifact_hash: trial.artifact_hash, judge_version: 'structural-0.0', structural: { state: na ? 'not_applicable' : 'evaluated', passed: na ? null : failures.length === 0 }, semantic: { state: na ? 'not_applicable' : 'not_evaluated', score: null, reason: na ? item.reason : 'No artifact-bound reviewed semantic assessment.' }, hard_failures: na ? [] : [...new Set(failures)], task_success: na ? null : failures.length ? false : null };
}

export function report(directory, manifest, suite, trials, reviews = new Map()) {
  const scores = trials.map(trial => {
    check('trial', trial);
    if (trial.artifact !== `attempts/${trial.attempt_id}/artifact.json`) throw new Error('invalid artifact path');
    const bytes = readFileSync(join(directory, trial.artifact));
    if (hash(bytes) !== trial.artifact_hash) throw new Error(`artifact hash mismatch: ${trial.attempt_id}`);
    const item = suite.cases.find(item => item.id === trial.case_id);
    if (!item) throw new Error('unknown trial case');
    const result = applyReview(score(trial, item, JSON.parse(bytes)), reviews.get(trial.attempt_id));
    check('score', result);
    return result;
  });
  const counts = values => Object.fromEntries([...new Set(values)].map(value => [value, values.filter(v => v === value).length]));
  const execution = counts(trials.map(t => t.status));
  const result = {
    protocol: '0.0', evidence: manifest.evidence, candidate: manifest.candidate,
    attempts: trials.length, independent_cases: new Set(trials.map(t => t.case_id)).size, independent_books: suite.books.size,
    execution, structural_passes: scores.filter(s => s.structural.passed).length,
    hard_failures: scores.filter(s => s.hard_failures.length).length,
    semantic_evaluated: scores.filter(s => s.semantic.state === 'evaluated').length,
    task_successes: scores.filter(s => s.task_success === true).length,
    quality: scores.some(s => s.hard_failures.length || s.task_success === false) ? 'fail' : 'inconclusive',
    calls: trials.reduce((sum, t) => sum + t.call_count, 0), usage_coverage: { known_calls: trials.reduce((sum, t) => sum + t.known_usage_calls, 0), total_calls: trials.reduce((sum, t) => sum + t.call_count, 0) },
    costs: { compile_usd: null, answer_usd: null, judge_usd: null, basis: 'Unpriced offline/replay evidence; not an invoice.' },
    duration_ms: trials.map(t => t.duration_ms),
    dimensions: Object.fromEntries([...new Set(suite.cases.flatMap(c => c.dimensions))].map(d => [d, { attempts: trials.filter(t => suite.cases.find(c => c.id === t.case_id).dimensions.includes(d)).length }])),
    limitations: ['Draft gold is not independent human review.', 'Offline registration compilation and source extraction do not establish model quality even when sample reviews pass.', 'Held-out isolation and real EPUB corpus ingestion are not implemented.'],
    scores,
  };
  save(join(directory, 'report.json'), result);
  return result;
}
