#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawn, execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const [catalogPath, outputPath, selectedBook, selectedProfile = "standard"] = process.argv.slice(2);
assert.ok(catalogPath && outputPath, "usage: node scripts/accept-online.mjs <catalog.json> <output-directory>");
const root = resolve(outputPath);
const binary = resolve("target/debug/codexia");
const analyzer = resolve(process.env.CODEXIA_ACCEPTANCE_ANALYZER || "scripts/online-analyzer.mjs");
const read = (path) => JSON.parse(readFileSync(path, "utf8"));
const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
const catalog = read(catalogPath);
assert.ok(process.env.CODEXIA_ANALYZER_MODEL, "CODEXIA_ANALYZER_MODEL is required");
const books = catalog.books.filter((book) => selectedBook ? book.id === selectedBook : ["alice-pg11", "souls-of-black-folk-pg408", "pride-and-prejudice-pg1342"].includes(book.id));
assert.equal(books.length, selectedBook ? 1 : 3, "requested Golden Books must be present");
assert.ok(["basic", "standard", "deep"].includes(selectedProfile));
mkdirSync(root, { recursive: true });
assert.ok(!existsSync(join(root, "report.json")), "use a fresh output directory to preserve the previous acceptance report");
const report = {
  started_at: new Date().toISOString(),
  commit: execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim(),
  binary_sha256: sha256(binary),
  analyzer_sha256: sha256(analyzer),
  model: process.env.CODEXIA_ANALYZER_MODEL,
  prompt_version: process.env.CODEXIA_ANALYZER_PROMPT_VERSION || "chapters:online-v2;synthesis:synthesis-v3",
  scope: selectedBook ? "single-profile" : "P0-online",
  runs: [],
  passed: false,
};
const save = () => writeFileSync(join(root, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
save();

try {
  for (const [book, profile] of selectedBook ? [[books[0], selectedProfile]] : [...books.map((book) => [book, "standard"]), [books.find((book) => book.id === "alice-pg11"), "deep"], [books.find((book) => book.id === "alice-pg11"), "basic"]]) {
    const runDir = join(root, `${book.id}-${profile}`);
    const packageDir = join(runDir, "package");
    mkdirSync(runDir, { recursive: true });
    assert.equal(sha256(book.file.path), book.file.sha256, `${book.id}: source hash`);
    const run = { book_id: book.id, profile, source_sha256: book.file.sha256, package: packageDir, states: [], passed: false };
    report.runs.push(run);
    save();
    const args = ["compile", resolve(book.file.path), "--profile", profile, "--out", packageDir, "--analyzer-command", analyzer, "--analysis-jobs", "2"];
    const env = { ...process.env, CODEXIA_ONLINE_RUN_DIR: runDir };
    console.log(`Compiling ${book.id} (${profile})`);
    const started = performance.now();
    const result = await compile(args, env, packageDir, run.states);
    run.duration_ms = Math.round(performance.now() - started);
    run.initial_cache_hit = result.output.includes("Reused cached package");
    writeFileSync(join(runDir, "compile.log"), result.output);
    run.exit_code = result.code;
    run.events = providerEvents(runDir);
    run.prompt_versions = [...new Set(run.events.map((event) => event.prompt_version))];
    run.replayed_events = run.events.filter((event) => event.replayed).length;
    run.provider_span_ms = run.events.length ? Math.max(...run.events.map((event) => Date.parse(event.finished_at))) - Math.min(...run.events.map((event) => Date.parse(event.started_at))) : 0;
    save();
    assert.equal(result.code, 0, `${book.id}: compile failed; see ${runDir}/compile.log`);
    const status = read(join(packageDir, "compile_status.json"));
    assert.equal(status.complete, true);
    assert.equal(status.analyzed_through, null);
    assert.equal(status.analyzed_chapter_count, status.total_analyzable_chapter_count);
    const stages = ["parse", "normalize", "chapter_analysis", "book_synthesis", "grounding_validation"];
    assert.deepEqual(status.ready_stages, stages);
    let previous = 0;
    for (const state of run.states) {
      assert.deepEqual(state.ready_stages, stages.slice(0, state.ready_stages.length));
      assert.ok(state.ready_stages.length >= previous, "progress must not go backwards");
      previous = state.ready_stages.length;
    }
    run.validation = execFileSync(binary, ["validate", packageDir], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
    assert.equal(read(join(packageDir, "eval_report.json")).valid, true);
    const beforeCache = providerEvents(runDir).length;
    const cached = await compile(args, env, packageDir, []);
    assert.equal(cached.code, 0);
    assert.match(cached.output, /Reused cached package/);
    assert.equal(providerEvents(runDir).length, beforeCache, "cache must not invoke provider");
    run.cache_verified = true;
    run.passed = true;
    save();
  }
  report.passed = true;
} catch (error) {
  report.error = String(error);
  process.exitCode = 1;
  console.error(report.error);
} finally {
  report.finished_at = new Date().toISOString();
  save();
}

async function compile(args, env, packageDir, states) {
  const child = spawn(binary, args, { env, stdio: ["ignore", "pipe", "pipe"] });
  let output = "";
  child.stdout.on("data", (chunk) => { output += chunk; });
  child.stderr.on("data", (chunk) => { output += chunk; });
  let done = false;
  const completion = new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("close", (code) => resolve({ code, output }));
  }).finally(() => { done = true; });
  while (!done) {
    try {
      const state = read(join(packageDir, "compile_status.json"));
      if (JSON.stringify(states.at(-1)) !== JSON.stringify(state)) states.push(state);
    } catch (error) {
      if (error.code !== "ENOENT" && !(error instanceof SyntaxError)) throw error;
    }
    await delay(50);
  }
  return completion;
}

function providerEvents(runDir) {
  let directories;
  try { directories = readdirSync(join(runDir, "provider")); }
  catch (error) { if (error.code === "ENOENT") return []; throw error; }
  return directories.map((directory) => read(join(runDir, "provider", directory, "event.json")));
}
