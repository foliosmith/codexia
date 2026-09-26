#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdirSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const [packageDir, chapterId, outputDir] = process.argv.slice(2);
assert.ok(packageDir && chapterId && outputDir && process.env.CODEXIA_ANALYZER_MODEL, "usage: CODEXIA_ANALYZER_MODEL=<model> node scripts/compare-analyzers.mjs <package> <chapter-id> <output-directory>");
const root = resolve(outputDir);
mkdirSync(root, { recursive: true });
const report = { package: resolve(packageDir), chapter_id: chapterId, passed: false };
const server = spawn(resolve("target/debug/codexia"), ["studio", resolve(packageDir), "--state-dir", join(root, "state"), "--bind", "127.0.0.1:18790", "--agent-command", resolve("scripts/online-analyzer.mjs")], { env: { ...process.env, CODEXIA_ONLINE_RUN_DIR: root }, stdio: ["ignore", "ignore", "pipe"] });
let log = "";
server.stderr.on("data", (chunk) => { log += chunk; });
const api = async (path, body) => {
  const response = await fetch(`http://127.0.0.1:18790/v1/studio/${path}`, { method: body ? "POST" : "GET", body: body && JSON.stringify(body), signal: AbortSignal.timeout(650_000) });
  const value = await response.json();
  assert.ok(response.ok, JSON.stringify(value));
  return value;
};
try {
  let ready = false;
  for (let i = 0; i < 100; i++) {
    if (server.exitCode !== null) throw new Error(log);
    if (!log.includes("Codexia:")) { await delay(100); continue; }
    try { ready = (await fetch("http://127.0.0.1:18790/v1/studio/snapshot", { signal: AbortSignal.timeout(1000) })).ok; }
    catch { ready = false; }
    if (ready) break;
    await delay(100);
  }
  assert.ok(ready, "Studio must start");
  report.before = await api("evals", {});
  const label = `${process.env.CODEXIA_ANALYZER_MODEL}/${process.env.CODEXIA_ANALYZER_PROMPT_VERSION || "online-v2"}`;
  report.version = await api(`chapters/${chapterId}/reanalyze`, { analyzer_label: label });
  report.diff = await api("compare", { kind: "chapter", id: chapterId, left_version: "base", right_version: report.version.version_id });
  report.versions = await api(`chapters/${chapterId}/versions`);
  report.after = await api("evals", {});
  assert.ok(report.versions.length >= 2);
  assert.ok(report.diff.length > 0);
  assert.equal(report.after.valid, true);
  assert.equal(report.after.metrics.source_ref_validity.value_basis_points, 10_000);
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
  writeFileSync(join(root, "comparison.json"), `${JSON.stringify(report, null, 2)}\n`);
}
