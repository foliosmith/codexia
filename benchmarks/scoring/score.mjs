import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { hash, check, root } from '../data.mjs';
import { save, privatePath, treeHash } from '../storage.mjs';
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
  const retrieval = [];
  const packageEvaluations = {};
  const compilations = new Map();
  const readerTimings = [];
  const scores = trials.map(trial => {
    check('trial', trial);
    if (trial.artifact !== `attempts/${trial.attempt_id}/artifact.json`) throw new Error('invalid artifact path');
    const bytes = readFileSync(privatePath(join(directory, trial.artifact)));
    if (hash(bytes) !== trial.artifact_hash) throw new Error(`artifact hash mismatch: ${trial.attempt_id}`);
    const item = suite.cases.find(item => item.id === trial.case_id);
    if (!item) throw new Error('unknown trial case');
    const artifact = JSON.parse(bytes);
    if (artifact.compilation) {
      compilations.set(artifact.compilation.id, artifact.compilation);
      readerTimings.push({ attempt_id: trial.attempt_id, reader_ms: artifact.timing.reader_ms, compile_ms: artifact.compilation.duration_ms, first_result_ms: trial.status === 'completed' ? artifact.compilation.duration_ms + artifact.timing.reader_ms : null, basis: 'Compile plus Reader completion; first completed result, not semantic correctness or streaming first token.' });
    }
    if (artifact.package_eval) packageEvaluations[item.book_id] = artifact.package_eval;
    const gold = suite.gold.get(item.id);
    const offered = artifact.calls.flatMap(call => call.request?.evidence_refs || []);
    const fractions = gold.evidence_sets.map(set => set.filter(anchor => {
      const block = artifact.anchors?.[anchor];
      return block && offered.some(ref => ref.block_id === block.block_id && ref.text_fingerprint === block.text_fingerprint && ref.start_char === 0 && ref.end_char === [...block.text].length);
    }).length / set.length);
    const mapped = Boolean(artifact.anchors);
    retrieval.push({ attempt_id: trial.attempt_id, gold_status: gold.status, state: !fractions.length ? 'not_applicable' : mapped ? 'evaluated' : 'not_evaluated', best_evidence_set_recall: fractions.length && mapped ? Math.max(...fractions) : null, complete_evidence_set: fractions.length && mapped ? fractions.includes(1) : null, basis: 'Full annotated source blocks offered to the provider, not semantic support or answer correctness.' });
    const result = applyReview(score(trial, item, artifact), reviews.get(trial.attempt_id));
    check('score', result);
    return result;
  });
  const counts = values => Object.fromEntries([...new Set(values)].map(value => [value, values.filter(v => v === value).length]));
  const execution = counts(trials.map(t => t.status));
  function group(key) {
    const groups = {};
    for (const [index, trial] of trials.entries()) {
      const item = suite.cases.find(c => c.id === trial.case_id);
      const values = key(item);
      for (const value of new Set(values)) {
        const group = groups[value] ??= { attempts: 0, completed: 0, structural_passes: 0, semantic_evaluated: 0, successes: 0, hard_failures: 0 };
        group.attempts++;
        group.completed += Number(trial.status === 'completed');
        group.structural_passes += Number(scores[index].structural.passed === true);
        group.semantic_evaluated += Number(scores[index].semantic.state === 'evaluated');
        group.successes += Number(scores[index].task_success === true);
        group.hard_failures += Number(scores[index].hard_failures.length > 0);
      }
    }
    return groups;
  }
  const result = {
    protocol: '0.0', evidence: manifest.evidence, candidate: manifest.candidate,
    suite_hash: suite.fingerprint, execution_hash: manifest.execution_hash ?? null, scorer_hash: treeHash(join(root, 'benchmarks/scoring')),
    attempts: trials.length, independent_cases: new Set(trials.map(t => t.case_id)).size,
    independent_books: new Set(trials.map(t => suite.cases.find(c => c.id === t.case_id).book_id)).size,
    expected_attempts: suite.cases.length * manifest.repeat,
    missing_attempts: suite.cases.length * manifest.repeat - trials.length,
    execution, structural_passes: scores.filter(s => s.structural.passed).length,
    hard_failures: scores.filter(s => s.hard_failures.length).length,
    semantic_evaluated: scores.filter(s => s.semantic.state === 'evaluated').length,
    task_successes: scores.filter(s => s.task_success === true).length,
    quality: scores.some(s => s.hard_failures.length || s.task_success === false) ? 'fail' : 'inconclusive',
    calls: trials.reduce((sum, t) => sum + t.call_count, 0), usage_coverage: { known_calls: trials.reduce((sum, t) => sum + t.known_usage_calls, 0), total_calls: trials.reduce((sum, t) => sum + t.call_count, 0) },
    costs: { compile_usd: null, answer_usd: null, judge_usd: null, basis: 'Unpriced offline/replay evidence; not an invoice.' },
    duration_ms: trials.map(t => t.duration_ms),
    dimensions: group(c => c.dimensions), books: group(c => [c.book_id]), languages: group(c => [suite.books.get(c.book_id).language]), tasks: group(c => c.steps.map(s => s.task)), suites: group(c => [c.suite]),
    package_evaluations: packageEvaluations, retrieval,
    mode: manifest.mode, compilations: [...compilations.values()], reader_timings: readerTimings,
    limitations: ['Draft gold is not independent human review.', 'Offline registration compilation and source extraction do not establish model quality even when sample reviews pass.', 'Held-out process isolation and online budget enforcement are not implemented.'],
    scores,
  };
  save(join(directory, 'report.json'), result);
  return result;
}
