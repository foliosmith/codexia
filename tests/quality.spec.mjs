import { test, expect } from "@playwright/test";
import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

test("Beta gate measures profiles and rejects invalid evidence or budgets", async ({}, info) => {
  const input = process.env.CODEXIA_ONLINE_REPORT;
  test.skip(!input, "Requires the real online acceptance report");
  const baseline = JSON.parse(readFileSync(input, "utf8"));
  expect(baseline.passed).toBe(true);
  const standard = baseline.runs.find((run) => run.profile === "standard");
  const invalidUsage = structuredClone(baseline.runs);
  invalidUsage[0].events[0].usage.input_tokens = "missing";
  const variants = {
    "duplicate-books": [standard, standard, standard, ...baseline.runs.filter((run) => run.profile !== "standard")],
    "missing-basic": baseline.runs.filter((run) => run.profile !== "basic"),
    "invalid-usage": invalidUsage,
  };
  mkdirSync(info.outputPath(), { recursive: true });
  const positiveDir = info.outputPath("positive");
  const positive = spawnSync(process.execPath, ["scripts/accept-quality.mjs", "private/golden-books/catalog.json", input, positiveDir], { encoding: "utf8", timeout: 30_000 });
  expect(positive.status, positive.stderr).toBe(0);
  const measured = JSON.parse(readFileSync(join(positiveDir, "report.json"), "utf8")).books.filter((book) => book.evidence === "online");
  expect(measured.length).toBe(baseline.runs.length);
  expect([...new Set(measured.map((book) => book.profile))].sort()).toEqual(["basic", "deep", "standard"]);
  for (const [name, runs] of Object.entries(variants)) {
    const candidate = info.outputPath(`${name}.json`);
    writeFileSync(candidate, JSON.stringify({ ...baseline, runs }));
    const directory = info.outputPath(name);
    const result = spawnSync(process.execPath, ["scripts/accept-quality.mjs", "private/golden-books/catalog.json", candidate, directory], { encoding: "utf8", timeout: 30_000 });
    expect(result.error).toBeUndefined();
    expect(result.status, result.stderr).toBe(1);
    const report = JSON.parse(readFileSync(join(directory, "report.json"), "utf8"));
    expect(report.passed).toBe(false);
    expect(report.error).toMatch(/distinct|duplicate|basic|usage/);
  }
  for (const [name, field] of [["cost-limit", "max_estimated_usd"], ["duration-limit", "max_duration_ms"]]) {
    const policy = JSON.parse(readFileSync(join(positiveDir, "report.json"), "utf8"));
    policy.thresholds.runs[0][field] = 0;
    const policyPath = info.outputPath(`${name}.json`);
    writeFileSync(policyPath, JSON.stringify(policy));
    const directory = info.outputPath(name);
    const result = spawnSync(process.execPath, ["scripts/accept-quality.mjs", "private/golden-books/catalog.json", input, directory, policyPath], { encoding: "utf8", timeout: 30_000 });
    expect(result.status, result.stderr).toBe(1);
    expect(JSON.parse(readFileSync(join(directory, "report.json"), "utf8")).error).toMatch(/exceeds baseline/);
  }
});
