import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

export const root = fileURLToPath(new URL('../', import.meta.url));
export const hash = value => createHash('sha256').update(value).digest('hex');
export const read = path => JSON.parse(readFileSync(path, 'utf8'));
export const lines = path => readFileSync(path, 'utf8').split('\n').filter(line => line.trim()).map(JSON.parse);

// Only the keywords used by the checked-in schemas are implemented; unknown ones fail closed.
function validate(value, schema, at) {
  for (const key of Object.keys(schema)) assert.ok(['$schema', 'type', 'enum', 'const', 'required', 'properties', 'additionalProperties', 'items', 'minItems', 'minLength', 'pattern', 'minimum', 'maximum'].includes(key), `unsupported schema keyword ${key}`);
  if (schema.type) {
    const types = { object: value !== null && typeof value === 'object' && !Array.isArray(value), array: Array.isArray(value), string: typeof value === 'string', integer: Number.isSafeInteger(value), boolean: typeof value === 'boolean' };
    assert.ok(types[schema.type], `${at}: expected ${schema.type}`);
  }
  if (schema.enum) assert.ok(schema.enum.includes(value), `${at}: invalid enum`);
  if ('const' in schema) assert.equal(value, schema.const, `${at}: invalid constant`);
  if (schema.required) for (const key of schema.required) assert.ok(Object.hasOwn(value, key), `${at}: missing ${key}`);
  if (schema.properties) for (const [key, child] of Object.entries(value)) {
    assert.ok(key in schema.properties || schema.additionalProperties !== false, `${at}: unknown ${key}`);
    if (key in schema.properties) validate(child, schema.properties[key], `${at}.${key}`);
  }
  if (schema.items) value.forEach((child, index) => validate(child, schema.items, `${at}[${index}]`));
  if (schema.minItems !== undefined) assert.ok(value.length >= schema.minItems, `${at}: too few items`);
  if (schema.minLength !== undefined) assert.ok(value.trim().length >= schema.minLength, `${at}: empty text`);
  if (schema.pattern) assert.ok(new RegExp(schema.pattern).test(value), `${at}: invalid format`);
  if (schema.minimum !== undefined) assert.ok(value >= schema.minimum, `${at}: below minimum`);
  if (schema.maximum !== undefined) assert.ok(value <= schema.maximum, `${at}: above maximum`);
}

export function check(name, value) {
  validate(value, read(new URL(`schemas/${name}.schema.json`, import.meta.url)), name);
}

export function loadSuite(path) {
  path = resolve(path);
  const catalog = read(path);
  check('catalog', catalog);
  const base = dirname(path);
  const files = [path, resolve(base, catalog.cases), resolve(base, catalog.gold)];
  const books = new Map();
  for (const entry of catalog.books) {
    assert.ok(!books.has(entry.id), `duplicate book ${entry.id}`);
    const source = resolve(base, entry.path);
    const bytes = readFileSync(source);
    assert.equal(hash(bytes), entry.sha256, `source hash mismatch: ${entry.id}`);
    const book = JSON.parse(bytes);
    check('book', book);
    assert.equal(book.id, entry.id);
    const anchors = new Map();
    const hrefs = new Set();
    for (const chapter of book.chapters) {
      assert.ok(!hrefs.has(chapter.href), `duplicate chapter ${chapter.href}`);
      hrefs.add(chapter.href);
      for (const paragraph of chapter.paragraphs) {
        assert.ok(!anchors.has(paragraph.id), `duplicate anchor ${paragraph.id}`);
        assert.equal(hash(paragraph.text), paragraph.sha256, `paragraph hash mismatch: ${paragraph.id}`);
        anchors.set(paragraph.id, { ...paragraph, href: chapter.href });
      }
    }
    books.set(entry.id, { ...book, anchors, source });
    files.push(source);
  }
  const cases = lines(files[1]);
  assert.ok(cases.length, 'empty cases');
  const ids = new Set();
  for (const item of cases) {
    check('case', item);
    assert.ok(!ids.has(item.id), `duplicate case ${item.id}`);
    ids.add(item.id);
    const book = books.get(item.book_id);
    assert.ok(book, `unknown book ${item.book_id}`);
    if (item.applicability === 'not_applicable') assert.ok(item.reason, 'non-applicability needs a reason');
    for (const step of item.steps) {
      assert.ok(step.task !== 'ask' || step.question, `${item.id}: ask needs question`);
      assert.ok(step.task === 'ask' || step.selection, `${item.id}: selection required`);
      assert.ok(step.task !== 'reflect' || step.answer, `${item.id}: reflection needs answer`);
      if (step.selection) assert.ok(book.anchors.has(step.selection), `unknown anchor ${step.selection}`);
      for (const endpoint of step.read) {
        const anchor = book.anchors.get(endpoint.anchor);
        assert.ok(anchor, `unknown anchor ${endpoint.anchor}`);
        assert.ok(endpoint.end_char === undefined || endpoint.end_char <= [...anchor.text].length, 'read endpoint out of bounds');
      }
    }
  }
  const gold = new Map();
  for (const item of lines(files[2])) {
    check('gold', item);
    assert.ok(ids.has(item.case_id) && !gold.has(item.case_id), `unexpected/duplicate gold ${item.case_id}`);
    if (item.status === 'reviewed') assert.ok(item.reviewer, 'reviewed gold requires reviewer');
    const book = books.get(cases.find(c => c.id === item.case_id).book_id);
    for (const evidence of item.evidence_sets.flat()) assert.ok(book.anchors.has(evidence), `unknown evidence ${evidence}`);
    gold.set(item.case_id, item);
  }
  assert.equal(gold.size, cases.length, 'missing gold');
  const fingerprints = files.map(file => hash(readFileSync(file)));
  return { catalog, cases, gold, books, fingerprint: hash(JSON.stringify(fingerprints)) };
}
