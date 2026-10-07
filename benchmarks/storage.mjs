import assert from 'node:assert/strict';
import { existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve, sep } from 'node:path';
import { hash, root } from './data.mjs';
import { openSync, closeSync, unlinkSync } from 'node:fs';
import { hostname } from 'node:os';

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

export function treeBytes(directory) {
  return readdirSync(directory, { withFileTypes: true }).reduce((total, entry) => {
    const path = join(directory, entry.name);
    assert.ok(!entry.isSymbolicLink(), 'symlink in measured artifacts');
    return total + (entry.isDirectory() ? treeBytes(path) : lstatSync(path).size);
  }, 0);
}

export function acquireLock(directory, recover) {
  const path = privatePath(join(directory, '.lock'));
  let recovery;
  try {
    if (recover && existsSync(path)) {
      recovery = openSync(`${path}.recovery`, 'wx', 0o600);
      const raw = readFileSync(path, 'utf8');
      const owner = JSON.parse(raw);
      assert.equal(owner.host, hostname(), 'cannot reclaim a lock from another host');
      assert.ok(Number.isSafeInteger(owner.pid) && owner.pid > 0, 'invalid lock owner');
      let dead = false;
      try { process.kill(owner.pid, 0); } catch (error) { if (error.code === 'ESRCH') dead = true; else throw error; }
      assert.ok(dead, 'run is already locked by a live process');
      assert.equal(readFileSync(path, 'utf8'), raw, 'lock changed during recovery');
      unlinkSync(path);
    }
    const fd = openSync(path, 'wx', 0o600);
    writeFileSync(fd, JSON.stringify({ pid: process.pid, host: hostname() }));
    return () => { closeSync(fd); unlinkSync(path); };
  } finally {
    if (recovery !== undefined) { closeSync(recovery); unlinkSync(`${path}.recovery`); }
  }
}
