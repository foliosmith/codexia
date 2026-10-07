import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { hash, loadSuite } from './data.mjs';
import { privatePath, save } from './storage.mjs';

export function auditCorpus(devPath, holdoutPath, output) {
  assert.ok(output, 'audit-corpus requires --output');
  const directory = privatePath(output);
  assert.ok(!existsSync(directory), 'use a fresh corpus audit directory');
  const dev = loadSuite(devPath);
  assert.equal(dev.catalog.split, 'dev', 'development catalog must declare dev');
  const holdout = holdoutPath ? loadSuite(holdoutPath) : null;
  if (holdout) assert.equal(holdout.catalog.split, 'holdout', 'holdout catalog must declare holdout');
  const describe = suite => [...suite.books.values()].map(book => ({
    id: book.id, family: book.family, language: book.language,
    cases: suite.cases.filter(c => c.book_id === book.id).length,
    source_hash: book.epub ? hash(readFileSync(book.epub)) : hash(JSON.stringify(book.chapters.map(c => c.paragraphs.map(p => p.text)))),
  }));
  const development = describe(dev);
  const sealed = holdout ? describe(holdout) : [];
  const conflicts = [];
  for (const a of development) for (const b of sealed) {
    if (a.family === b.family) conflicts.push({ dev: a.id, holdout: b.id, reason: 'same_source_family' });
    if (a.source_hash === b.source_hash) conflicts.push({ dev: a.id, holdout: b.id, reason: 'identical_source' });
  }
  const summarize = suite => ({
    questions: suite.cases.length, source_families: new Set([...suite.books.values()].map(b => b.family)).size,
    reviewed_gold: [...suite.gold.values()].filter(g => g.status === 'reviewed').length,
    regression: suite.cases.filter(c => c.suite === 'regression').length,
    capability: suite.cases.filter(c => c.suite === 'capability').length,
    dimensions: Object.fromEntries(Array.from({ length: 18 }, (_, i) => `D${String(i + 1).padStart(2, '0')}`).map(d => [d, suite.cases.filter(c => c.dimensions.includes(d)).length])),
  });
  const result = { version: '0.0', dev: { ...summarize(dev), books: development }, holdout: holdout ? { ...summarize(holdout), books: sealed } : null, conflicts, split_valid: holdout ? conflicts.length === 0 : null, sealed_execution_ready: false, limitations: ['Checks declared source families and exact source duplication; unreported translations/rewrites require provenance review.', 'A valid split does not prove candidate process isolation, independent gold review or generalisation.'] };
  save(join(directory, 'corpus-audit.json'), result);
  return result;
}
