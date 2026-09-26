#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { once } from "node:events";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

const [catalogPath, onlinePath, outputPath, baselinePath] = process.argv.slice(2);
assert.ok(catalogPath && onlinePath && outputPath, "usage: node scripts/accept-quality.mjs <catalog.json> <online-report.json> <output-directory> [baseline.json]");
const read = (path) => JSON.parse(readFileSync(path, "utf8"));
const rates = { short: { input: 10, cached: 1, write: 12.5, output: 50 }, long: { input: 20, cached: 2, write: 25, output: 75 } };
function estimate(usage) {
  const cached = usage.cached_input_tokens || 0;
  const writes = usage.cache_write_input_tokens || 0;
  assert.ok([usage.input_tokens, usage.output_tokens, cached, writes].every((value) => Number.isSafeInteger(value) && value >= 0), "usage tokens must be non-negative integers");
  assert.ok(cached + writes <= usage.input_tokens, "usage cache tokens exceed input tokens");
  const rate = usage.input_tokens > 272_000 ? rates.long : rates.short;
  return ((usage.input_tokens - cached - writes) * rate.input + cached * rate.cached + writes * rate.write + usage.output_tokens * rate.output) / 1_000_000;
}
const catalog = read(catalogPath);
const online = read(onlinePath);
const semanticPath = process.env.CODEXIA_SEMANTIC_REVIEW;
const semanticReviews = semanticPath ? read(semanticPath).reviews : [];
const root = resolve(outputPath);
mkdirSync(root, { recursive: true });
const report = {
  schema_version: "0.2",
  semantic_review: semanticPath ? resolve(semanticPath) : null,
  created_at: new Date().toISOString(),
  online_report: resolve(onlinePath),
  pricing: { source: "https://developers.openai.com/api/docs/pricing", retrieved_at: "2026-09-26", currency: "USD", rates_per_million: rates, long_context_above_input_tokens: 272_000, basis: "API Standard equivalent estimate, not a ChatGPT subscription invoice" },
  books: [],
  runs: [],
  recorded_experiment_cost: null,
  passed: false,
};
try {
  assert.equal(new Set(online.runs.map((run) => `${run.book_id}/${run.profile}`)).size, online.runs.length, "duplicate book/profile evidence");
  const evaluations = catalog.books.flatMap((book) => {
    const runs = online.runs.filter((run) => run.book_id === book.id && run.passed);
    return runs.length ? runs.map((run) => ({ book, run })) : [{ book, run: null }];
  });
  for (const { book, run } of evaluations) {
    const packageDir = run?.package || book.registration_package.path;
    execFileSync("target/debug/codexia", ["validate", packageDir], { stdio: "pipe" });
    const stateDir = join(root, `${book.id}-${run?.profile || "registration"}`);
    const server = spawn(resolve("target/debug/codexia"), ["studio", resolve(packageDir), "--state-dir", stateDir, "--bind", "127.0.0.1:18789"], { stdio: ["ignore", "ignore", "pipe"] });
    let log = "";
    server.stderr.on("data", (chunk) => { log += chunk; });
    server.on("error", (error) => { log += String(error); });
    try {
      let ready = false;
      for (let attempt = 0; attempt < 100; attempt++) {
        if (server.exitCode !== null) throw new Error(log);
        if (!log.includes("Codexia:")) { await delay(100); continue; }
        try { ready = (await fetch("http://127.0.0.1:18789/v1/studio/snapshot", { signal: AbortSignal.timeout(1000) })).ok; }
        catch { /* The local process may not have bound its socket yet. */ }
        if (ready) break;
        await delay(100);
      }
      assert.ok(ready, `Studio did not start: ${log}`);
      const goldenResponse = await fetch("http://127.0.0.1:18789/v1/studio/golden-books", {method:"POST",body:JSON.stringify({label:book.id,annotations:book.annotations})});
      assert.ok(goldenResponse.ok); const golden = await goldenResponse.json();
      const semantic = semanticReviews.find(item => item.book_id === book.id && item.profile === run?.profile);
      const response = await fetch("http://127.0.0.1:18789/v1/studio/evals", { method: "POST", body: JSON.stringify({golden_id:golden.golden_id, semantic_review:semantic?.review}), signal: AbortSignal.timeout(10_000) });
      assert.ok(response.ok, `semantic review or evaluation rejected: ${await response.clone().text()}`);
      const evaluation = await response.json();
      const sampleSizes = {
        source_refs: evaluation.grounding_report.stats.source_ref_count,
        concepts: read(join(packageDir, "concepts.json")).concepts.length,
        claims: read(join(packageDir, "claims.json")).claims.length,
      };
      const result = { book_id: book.id, profile: run?.profile || "registration", evidence: run ? "online" : "offline-registration-only", package: resolve(packageDir), sample_sizes: sampleSizes, evaluation };
      report.books.push(result);
      writeFileSync(join(stateDir, "quality.json"), `${JSON.stringify(result, null, 2)}\n`);
      assert.equal(evaluation.structural_valid, true, `${book.id}: grounding`);
      if (run) {
        assert.equal(evaluation.metrics.source_ref_validity.value_basis_points, 10_000);
        assert.equal(evaluation.valid, true, `${book.id}: required outputs`);
        assert.ok(evaluation.grounding_report.stats.source_ref_count > 0, "online evidence must have citations");
        assert.ok(sampleSizes.concepts > 0 && sampleSizes.claims > 0, "empty outputs cannot establish analysis quality");
      }
    } finally {
      if (server.exitCode === null && server.signalCode === null) {
        const exited = once(server, "exit");
        server.kill("SIGTERM");
        await exited;
      }
    }
  }
  for (const run of online.runs) {
    const book = catalog.books.find((book) => book.id === run.book_id);
    assert.ok(book, `${run.book_id}: unknown Golden Book`);
    const status = read(join(run.package, "compile_status.json"));
    assert.equal(status.complete, true);
    assert.equal(status.analyzed_through, null);
    assert.equal(status.analyzed_chapter_count, status.total_analyzable_chapter_count);
    assert.equal(status.profile, run.profile);
    assert.equal(status.source_hash, book.registration_package.source_hash);
    assert.ok(run.events?.some((event) => event.task === "book_synthesis" && event.status === "completed"), "online synthesis evidence is required");
    assert.ok(new Set(run.events.filter((event) => event.task === "chapter_analysis" && event.status === "completed").map((event) => event.task_id)).size >= status.analyzed_chapter_count, "online chapter evidence is incomplete");
    let estimatedUsd = 0;
    let inputTokens = 0;
    let outputTokens = 0;
    for (const event of run.events || []) {
      assert.equal(event.analysis_profile, run.profile);
      assert.equal(event.model, "gpt-6-astra", "supply reviewed pricing before estimating another model");
      assert.ok(event.usage, `${run.book_id}: provider usage unavailable`);
      const usage = event.usage;
      estimatedUsd += estimate(usage);
      inputTokens += usage.input_tokens;
      outputTokens += usage.output_tokens;
    }
    const replayed = run.events.some((event) => event.replayed);
    const providerSpan = Math.max(...run.events.map((event) => Date.parse(event.finished_at))) - Math.min(...run.events.map((event) => Date.parse(event.started_at)));
    report.runs.push({ book_id: run.book_id, profile: run.profile, duration_ms: replayed ? providerSpan : run.duration_ms, duration_basis: replayed ? "original provider timeline, including recovery gaps" : "observed compile wall time", input_tokens: inputTokens, output_tokens: outputTokens, estimated_usd: estimatedUsd, passed: run.passed });
  }
  assert.equal(online.passed, true, "full online acceptance must pass before setting Beta thresholds");
  const standardBooks = new Set(report.runs.filter((run) => run.profile === "standard" && run.passed).map((run) => run.book_id));
  assert.ok(standardBooks.size >= 3, "at least three distinct standard books are required");
  assert.ok(report.runs.some((run) => run.profile === "deep" && run.passed && standardBooks.has(run.book_id)), "a deep run of a standard book is required");
  assert.ok(report.runs.some((run) => run.profile === "basic" && run.passed), "basic profile evidence is required");
  assert.ok(report.books.filter(book => book.evidence === "online").every(book => book.evaluation.beta_ready), "artifact-bound semantic review is required for every online book/profile");
  if (baselinePath) {
    const baseline = read(baselinePath);
    for (const limit of baseline.thresholds.runs) assert.ok(report.runs.some((run) => run.book_id === limit.book_id && run.profile === limit.profile && run.passed), `${limit.book_id}/${limit.profile}: baseline case missing`);
    for (const run of report.runs) {
      const limit = baseline.thresholds.runs.find((item) => item.book_id === run.book_id && item.profile === run.profile);
      assert.ok(limit, `${run.book_id}/${run.profile}: baseline missing`);
      assert.ok(run.duration_ms <= limit.max_duration_ms, `${run.book_id}: duration exceeds baseline`);
      assert.ok(run.estimated_usd <= limit.max_estimated_usd, `${run.book_id}: estimated cost exceeds baseline`);
    }
    for (const book of report.books.filter((book) => book.evidence === "online")) {
      const minimum = baseline.thresholds.books.find((item) => item.book_id === book.book_id && item.profile === book.profile);
      assert.ok(minimum, `${book.book_id}: quality baseline missing`);
      for (const [metric, value] of Object.entries(minimum.minimum_metrics)) {
        const minimumValue = typeof value === "number" ? value : value.value_basis_points;
        const current = book.evaluation.metrics[metric];
        assert.ok(current && (minimumValue == null ? current.state === value.state : current.state === "evaluated" && current.value_basis_points >= minimumValue), `${book.book_id}: ${metric} regression`);
      }
    }
  } else {
    assert.ok(online.runs.every((run) => !run.initial_cache_hit), "a cold-generation report is required to set performance thresholds");
    report.thresholds = {
      policy: "No quality regression; at most 25% above observed cold-run time and API-equivalent cost. Single-run provisional limits, not latency percentiles.",
      books: report.books.filter((book) => book.evidence === "online").map((book) => ({ book_id: book.book_id, profile: book.profile, minimum_metrics: book.evaluation.metrics })),
      runs: report.runs.map((run) => ({ book_id: run.book_id, profile: run.profile, max_duration_ms: Math.ceil(run.duration_ms * 1.25), max_estimated_usd: run.estimated_usd * 1.25 })),
    };
  }
  if (online.experiment_events) {
    const events = online.experiment_events.filter((event) => event.model === "gpt-6-astra");
    report.recorded_experiment_cost = {
      basis: "Retained, deduplicated Astra provider events; includes exploratory and failed calls with reported usage. Not an account invoice.",
      estimated_usd_with_reported_usage: events.filter((event) => event.usage).reduce((sum, event) => sum + estimate(event.usage), 0),
      measured_event_count: events.filter((event) => event.usage).length,
      unpriced_events: events.filter((event) => !event.usage).map((event) => ({ event_id: event.event_id, status: event.status, artifact: event.artifact })),
    };
  }
  report.passed = true;
} catch (error) {
  report.error = String(error);
  console.error(report.error);
  process.exitCode = 1;
} finally {
  writeFileSync(join(root, "report.json"), `${JSON.stringify(report, null, 2)}\n`);
}
