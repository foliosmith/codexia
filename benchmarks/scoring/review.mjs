import assert from 'node:assert/strict';
import { hash } from '../data.mjs';

export const dimensions = ['citation_support', 'key_point_coverage', 'spoiler_boundary', 'attribution'];
export const identity = value => hash(JSON.stringify(value));

export function template(trials, suite) {
  return { version: '0.0', reviews: trials.filter(t => t.status === 'completed').map(trial => ({
    attempt_id: trial.attempt_id, artifact_hash: trial.artifact_hash,
    case_hash: identity(suite.cases.find(c => c.id === trial.case_id)), gold_hash: identity(suite.gold.get(trial.case_id)),
    reviewer: '', judge_version: '',
    checks: dimensions.map(dimension => ({ dimension, score: null, verdict: '', evidence: '', hard_failure: false })),
  })) };
}

export function validateReviews(value, trials, suite) {
  assert.equal(value.version, '0.0', 'unsupported review version');
  assert.ok(Array.isArray(value.reviews) && value.reviews.length, 'reviews required');
  const result = new Map();
  for (const review of value.reviews) {
    const trial = trials.find(t => t.attempt_id === review.attempt_id);
    assert.ok(trial && !result.has(trial.attempt_id), 'unknown or duplicate review attempt');
    assert.equal(trial.status, 'completed', 'cannot accept semantic review for an incomplete attempt');
    const gold = suite.gold.get(trial.case_id);
    assert.equal(gold.status, 'reviewed', 'gold is draft; independent review is required');
    assert.equal(review.artifact_hash, trial.artifact_hash, 'stale artifact review');
    assert.equal(review.case_hash, identity(suite.cases.find(c => c.id === trial.case_id)), 'stale case review');
    assert.equal(review.gold_hash, identity(gold), 'stale gold review');
    for (const key of ['reviewer', 'judge_version']) assert.ok(typeof review[key] === 'string' && review[key].trim(), `review ${key} required`);
    assert.ok(Array.isArray(review.checks) && review.checks.length === dimensions.length, 'all semantic dimensions required');
    assert.deepEqual(review.checks.map(c => c.dimension).sort(), [...dimensions].sort(), 'duplicate or unknown semantic dimension');
    for (const check of review.checks) {
      assert.ok(Number.isInteger(check.score) && check.score >= 0 && check.score <= 3, 'semantic score must be 0..3');
      assert.ok(['supports', 'refutes', 'insufficient'].includes(check.verdict), 'semantic verdict required');
      assert.ok(typeof check.evidence === 'string' && check.evidence.trim(), 'review evidence required');
      assert.equal(typeof check.hard_failure, 'boolean', 'hard_failure must be explicit');
      assert.ok(check.score < 3 || (check.verdict === 'supports' && !check.hard_failure), 'full score conflicts with verdict');
    }
    result.set(trial.attempt_id, review);
  }
  return result;
}

export function applyReview(score, review) {
  if (!review) return score;
  const value = Math.min(...review.checks.map(check => check.score));
  const hard = review.checks.filter(check => check.hard_failure).map(check => `semantic:${check.dimension}`);
  return { ...score, judge_version: review.judge_version,
    semantic: { state: 'evaluated', score: value, reason: `Artifact-bound attributed review ${identity(review)}; minimum across required dimensions.` },
    hard_failures: [...score.hard_failures, ...hard],
    task_success: score.structural.passed && !hard.length && value === 3,
  };
}
