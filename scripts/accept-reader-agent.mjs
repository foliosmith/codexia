#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const [packageDir, outputDir, chapterId = "chapter_003"] = process.argv.slice(2);
assert.ok(packageDir && outputDir && process.env.CODEXIA_ANALYZER_MODEL, "usage: CODEXIA_ANALYZER_MODEL=<model> node scripts/accept-reader-agent.mjs <package> <output-directory> [chapter-id]");
const root = resolve(outputDir);
mkdirSync(root, { recursive: true });
const report = { package: resolve(packageDir), chapter_id: chapterId, model: process.env.CODEXIA_ANALYZER_MODEL, passed: false };
const server = spawn(resolve("target/debug/codexia"), ["serve", resolve(packageDir), "--state-dir", join(root, "state"), "--bind", "127.0.0.1:18791", "--agent-command", resolve("scripts/online-analyzer.mjs")], { env: { ...process.env, CODEXIA_ONLINE_RUN_DIR: root }, stdio: ["ignore", "ignore", "pipe"] });
let log = "";
server.stderr.on("data", (chunk) => { log += chunk; });
const api = async (path, method = "GET", body) => {
  const response = await fetch(`http://127.0.0.1:18791${path}`, { method, body: body && JSON.stringify(body), signal: AbortSignal.timeout(950_000) });
  const value = await response.json();
  assert.ok(response.ok, JSON.stringify(value));
  return value;
};
try {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    if (server.exitCode !== null) throw new Error(log);
    if (!log.includes("Codexia:")) { await delay(100); continue; }
    try { ready = (await fetch("http://127.0.0.1:18791/v1/bootstrap", { signal: AbortSignal.timeout(1000) })).ok; }
    catch { ready = false; }
    if (ready) break;
    await delay(100);
  }
  assert.ok(ready, "Reader server must start");
  const bootstrap = await api("/v1/bootstrap");
  const bookPath = `/v1/books/${bootstrap.book.book_id}`;
  const chapter = await api(`${bookPath}/chapters/${chapterId}/content`);
  const block = chapter.blocks.find((block) => block.kind === "paragraph" && Array.from(block.text).length > 80);
  assert.ok(block);
  const last = chapter.blocks.at(-1);
  const location = { chapter_id: chapterId, block_id: block.block_id, char_offset: 0, epub_cfi: null };
  const session = await api("/v1/reader-sessions", "POST", { book_id: bootstrap.book.book_id, current_location: location, spoiler_mode: "read_range" });
  const updated = await api(`/v1/reader-sessions/${session.session_id}`, "PATCH", { current_location: location, read_until: { ...location, block_id: last.block_id, char_offset: Array.from(last.text).length }, progress_basis_points: 1000 });
  const reader_state = { session_id: updated.session_id, current_location: updated.current_location, read_until: updated.read_until, completed_chapter_ids: [], progress_basis_points: updated.progress_basis_points };
  const selected = Array.from(block.text).slice(0, 80).join("");
  report.explain = await api(`${bookPath}/explain`, "POST", { selected_text: selected, source_ref: { block_id: block.block_id, start_char: 0, end_char: 80, text_fingerprint: block.text_fingerprint }, reader_state, spoiler_mode: "read_range" });
  assert.ok(report.explain.cards[0].content.explanation.trim());
  assert.equal(report.explain.cards[0].card_type, "explanation");
  report.ask = await api(`${bookPath}/ask`, "POST", { question: "What happens in the final chapter? Use only what I have read.", reader_state, spoiler_mode: "read_range" });
  assert.ok(report.ask.cards[0].content.answer.trim());
  assert.equal(report.ask.cards[0].card_type, "answer");
  assert.equal(report.ask.cards[0].spoiler_status, "within_boundary");
  assert.ok(report.ask.spoiler_boundary.excluded_chapter_ids.length > 0);
  report.checkpoint = await api(`${bookPath}/chapters/${chapterId}/checkpoint`, "POST", { reader_state, spoiler_mode: "read_range" });
  const checkpoint = report.checkpoint.cards[0].content;
  const question = checkpoint.recall_questions[0];
  report.reflect = await api(`${bookPath}/chapters/${chapterId}/reflect`, "POST", { checkpoint_id: checkpoint.checkpoint_id, question_id: question.question_id, answer: question.expected_points.join(" "), reader_state });
  assert.equal(report.reflect.cards[0].card_type, "reflection");
  assert.ok(report.reflect.cards[0].content.feedback.trim());
  assert.ok(report.reflect.cards[0].content.score_basis_points >= 0 && report.reflect.cards[0].content.score_basis_points <= 10_000);
  report.passed = true;
} catch (error) {
  report.error = String(error);
  process.exitCode = 1;
  console.error(report.error);
} finally {
  if (server.exitCode === null && server.signalCode === null) {
    const exited = once(server, "exit");
    server.kill("SIGTERM");
    await exited;
  }
  writeFileSync(join(root, "reader-agent.json"), `${JSON.stringify(report, null, 2)}\n`);
}
