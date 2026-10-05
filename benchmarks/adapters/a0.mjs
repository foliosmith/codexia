import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { once } from 'node:events';
import { createServer } from 'node:net';
import { existsSync, mkdirSync, readFileSync, readdirSync, copyFileSync } from 'node:fs';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { root, read, hash } from '../data.mjs';
import { save, treeHash } from '../storage.mjs';

export function compile(book, directory, binary) {
  mkdirSync(directory, { recursive: true });
  const epub = join(directory, 'source.epub');
  if (!existsSync(epub)) {
    if (book.epub) copyFileSync(book.epub, epub);
    else execFileSync('python3', [join(root, 'benchmarks/adapters/build-fixture.py'), book.source, epub], { timeout: 10000 });
    save(join(directory, 'source-identity.json'), { sha256: hash(readFileSync(epub)) });
  }
  assert.equal(hash(readFileSync(epub)), read(join(directory, 'source-identity.json')).sha256, 'compile source changed');
  const pkg = join(directory, 'package');
  const start = performance.now();
  const output = execFileSync(binary, ['compile', epub, '--out', pkg, '--analyzer-command', join(root, 'tests/fixtures/analyzer.mjs'), '--analysis-jobs', '1'], { timeout: 120000, env: { ...process.env, CODEXIA_TEST_GROUNDING_MODE: '' } });
  execFileSync(binary, ['validate', pkg], { timeout: 10000 });
  save(join(directory, 'compile.json'), { mode: 'offline-registration', duration_ms: Math.round(performance.now() - start), usage: null, estimated_usd: null, output: output.toString(), package_hash: treeHash(pkg) });
  return pkg;
}

export function collectCalls(directory) {
  const calls = join(directory, 'calls');
  if (!existsSync(calls)) return [];
  return readdirSync(calls).sort().map(id => {
    const path = join(calls, id);
    let event = { id, status: 'interrupted', usage: null };
    try { event = { ...event, ...read(join(path, 'event.json')) }; } catch { /* Keep interrupted calls in accounting. */ }
    let request = null;
    try { request = read(join(path, 'request.json')); } catch { /* Capture can be interrupted before a complete request. */ }
    return { ...event, request, output: existsSync(join(path, 'output.txt')) ? readFileSync(join(path, 'output.txt'), 'utf8') : null };
  });
}

async function address() {
  const socket = createServer();
  socket.listen(0, '127.0.0.1');
  await once(socket, 'listening');
  const port = socket.address().port;
  await new Promise((resolve, reject) => socket.close(error => error ? reject(error) : resolve()));
  return `127.0.0.1:${port}`;
}

export async function execute({ item, book, pkg, directory, binary, agent, signal, candidate = 'A0' }) {
  signal = AbortSignal.any([signal, AbortSignal.timeout(item.budget.timeout_ms)]);
  const bind = await address();
  const server = spawn(binary, ['serve', pkg, '--state-dir', join(directory, 'state'), '--bind', bind, '--agent-command', join(root, 'benchmarks/adapters/capture.mjs')], {
    detached: process.platform !== 'win32', stdio: ['ignore', 'ignore', 'pipe'],
    env: { ...process.env, CODEXIA_BENCH_CANDIDATE: candidate, CODEXIA_BENCH_ATTEMPT: directory, CODEXIA_BENCH_AGENT: agent, CODEXIA_BENCH_MAX_CALLS: String(item.budget.max_calls), CODEXIA_READER_TIMEOUT_MS: String(item.budget.timeout_ms), CODEXIA_MAX_CONTEXT_BYTES: '524288', CODEXIA_MAX_ANALYZERS: '1' },
  });
  let serverError;
  server.on('error', error => { serverError = error; });
  let log = '';
  server.stderr.on('data', chunk => { log = (log + chunk).slice(-65536); });
  const exchanges = [];
  const result = { status: 'completed', exchanges, steps: [], calls: [], error: null };
  async function api(path, body, method = body ? 'POST' : 'GET') {
    const response = await fetch(`http://${bind}${path}`, { method, body: body && JSON.stringify(body), signal: AbortSignal.any([signal, AbortSignal.timeout(item.budget.timeout_ms + 2000)]) });
    const value = await response.json();
    exchanges.push({ path, method, request: body ?? null, status: response.status, response: value });
    save(join(directory, 'exchanges.json'), exchanges);
    if (!response.ok) throw new Error(`HTTP ${response.status}: ${JSON.stringify(value)}`);
    return value;
  }
  try {
    let ready = false;
    for (let attempt = 0; attempt < 100; attempt++) {
      signal.throwIfAborted();
      if (serverError) throw serverError;
      if (server.exitCode !== null) throw new Error(`Reader startup failed: ${log}`);
      if (log.includes('Codexia:')) { ready = true; break; }
      await delay(30, undefined, { signal });
    }
    assert.ok(ready, 'Reader startup timeout');
    const bootstrap = await api('/v1/bootstrap');
    result.package_eval = await api('/v1/studio/evals', {});
    const bookPath = `/v1/books/${bootstrap.book.book_id}`;
    const ir = read(join(pkg, 'book_ir.json'));
    const blocks = new Map();
    const anchors = new Map();
    const chapters = new Map();
    for (const anchor of book.anchors.values()) {
      const matches = ir.blocks.filter(block => block.source_ref.chapter_href === anchor.href && block.text === anchor.text);
      assert.equal(matches.length, 1, `source_mapping_error: ${anchor.id}`);
      const location = await api(`${bookPath}/blocks/${matches[0].block_id}/location`);
      if (!chapters.has(location.chapter_id)) {
        const chapter = await api(`${bookPath}/chapters/${location.chapter_id}/content`);
        chapters.set(location.chapter_id, chapter);
        for (const block of chapter.blocks) blocks.set(block.block_id, block);
      }
      const block = blocks.get(matches[0].block_id);
      anchors.set(anchor.id, { ...block, chapter_id: location.chapter_id });
    }
    save(join(directory, 'source-map.json'), Object.fromEntries(anchors));
    const first = anchors.get(item.steps[0].read[0].anchor);
    const location = (block, offset = 0) => ({ chapter_id: block.chapter_id, block_id: block.block_id, char_offset: offset, epub_cfi: null });
    let session = await api('/v1/reader-sessions', { book_id: bootstrap.book.book_id, current_location: location(first), spoiler_mode: 'read_range' });
    for (const [index, step] of item.steps.entries()) {
      signal.throwIfAborted();
      for (const endpoint of step.read) {
        const block = anchors.get(endpoint.anchor);
        session = await api(`/v1/reader-sessions/${session.session_id}`, { current_location: location(block), read_until: location(block, endpoint.end_char ?? [...block.text].length) }, 'PATCH');
      }
      const limits = new Map();
      for (const end of session.read_coverage) {
        const chapter = chapters.get(end.chapter_id);
        const lastIndex = chapter.blocks.findIndex(block => block.block_id === end.block_id);
        assert.ok(lastIndex >= 0, 'unknown persisted reading endpoint');
        for (const [i, block] of chapter.blocks.entries()) if (i <= lastIndex) limits.set(block.block_id, i === lastIndex ? end.char_offset : [...block.text].length);
      }
      const request = { request_id: `step-${index}`, reader_state: session, spoiler_mode: 'read_range' };
      let response;
      const block = anchors.get(step.selection);
      if (step.task === 'ask') response = await api(`${bookPath}/ask`, { ...request, question: step.question });
      else if (step.task === 'explain') response = await api(`${bookPath}/explain`, { ...request, selected_text: block.text, source_ref: { block_id: block.block_id, start_char: 0, end_char: [...block.text].length, text_fingerprint: block.text_fingerprint } });
      else {
        const checkpoint = await api(`${bookPath}/chapters/${block.chapter_id}/checkpoint`, { reader_state: session, spoiler_mode: 'read_range' });
        const content = checkpoint.cards[0].content;
        response = await api(`${bookPath}/chapters/${block.chapter_id}/reflect`, { request_id: request.request_id, reader_state: session, checkpoint_id: content.checkpoint_id, question_id: content.recall_questions[0].question_id, answer: step.answer });
      }
      result.steps.push({ task: step.task, response, limits: Object.fromEntries(limits) });
    }
    result.blocks = Object.fromEntries(blocks);
    result.anchors = Object.fromEntries(anchors);
  } catch (error) {
    result.error = error.message;
    result.status = signal.reason?.name === 'TimeoutError' || /timeout|timed.out/i.test(error.message) || error.name === 'TimeoutError' ? 'timed_out' : signal.aborted ? 'cancelled' : 'provider_error';
    if (existsSync(join(directory, 'budget-exceeded.json'))) result.status = 'budget_exceeded';
  } finally {
    if (server.pid && server.exitCode === null) {
      const exited = once(server, 'exit');
      if (process.platform === 'win32') spawn('taskkill', ['/PID', String(server.pid), '/T', '/F']);
      else { try { process.kill(-server.pid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; } }
      await exited;
    }
    result.calls = collectCalls(directory);
    if (result.calls.some(call => call.status === 'timed_out') && result.status !== 'cancelled') result.status = 'timed_out';
  }
  return result;
}
