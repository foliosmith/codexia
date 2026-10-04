#!/usr/bin/env node
import { parseArgs } from 'node:util';
import { fileURLToPath } from 'node:url';
import { loadSuite } from './data.mjs';

try {
  const { values, positionals } = parseArgs({ options: { catalog: { type: 'string', default: fileURLToPath(new URL('suites/v0.0/catalog.json', import.meta.url)) } }, allowPositionals: true });
  if (positionals.length !== 1 || positionals[0] !== 'validate') throw new Error('usage: node benchmarks/runner.mjs validate [--catalog <catalog.json>]');
  const suite = loadSuite(values.catalog);
  console.log(JSON.stringify({ valid: true, cases: suite.cases.length, books: suite.books.size, fingerprint: suite.fingerprint }));
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
