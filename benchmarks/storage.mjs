import assert from 'node:assert/strict';
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { hash, root } from './data.mjs';

export function privatePath(path) {
  const base = join(root, 'private');
  const target = resolve(path);
  const rel = relative(base, target);
  assert.ok(rel && !rel.startsWith(`..${sep}`) && rel !== '..' && !rel.startsWith(sep), 'output must be inside this checkout’s private directory');
  let current = base;
  for (const part of ['', ...rel.split(sep)]) {
    if (part) current = join(current, part);
    assert.ok(!lstatSync(current, { throwIfNoEntry: false })?.isSymbolicLink(), 'private output must not traverse symlinks');
  }
  assert.equal(realpathSync(root), resolve(root), 'checkout root must be canonical');
  return target;
}

export function save(path, value) {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  const temp = `${path}.tmp`;
  writeFileSync(temp, JSON.stringify(value, null, 2) + '\n', { mode: 0o600 });
  renameSync(temp, path);
}

export function treeHash(directory) {
  const files = [];
  function walk(dir) {
    for (const entry of readdirSync(dir, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const path = join(dir, entry.name);
      assert.ok(!entry.isSymbolicLink(), `symlink in hashed inputs: ${entry.name}`);
      if (entry.isDirectory()) walk(path);
      else files.push([relative(directory, path), hash(readFileSync(path))]);
    }
  }
  walk(directory);
  return hash(JSON.stringify(files));
}
