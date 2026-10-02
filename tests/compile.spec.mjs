import { test, expect } from "@playwright/test";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";

test("cited inferences remain inferred while unsupported grounded objects are rejected", async ({}, info) => {
  const binary = resolve("target/debug/codexia");
  const epub = resolve(process.env.CODEXIA_ACCEPTANCE_EPUB || "private/golden-books/source/alices-adventures-in-wonderland-pg11.epub");
  for (const mode of ["inferred-refs", "grounded-without-refs"]) {
    const output = info.outputPath(mode);
    const result = spawnSync(binary, ["compile", epub, "--out", output, "--analyzer-command", resolve("tests/fixtures/analyzer.mjs"), "--analyze-through", "2"], { env: { ...process.env, CODEXIA_TEST_GROUNDING_MODE: mode }, encoding: "utf8" });
    writeFileSync(info.outputPath(`${mode}.log`), result.stderr);
    expect(result.status, result.stderr).toBe(mode === "inferred-refs" ? 0 : 1);
    if (mode === "inferred-refs") {
      execFileSync(binary, ["validate", output]);
      const concept = JSON.parse(readFileSync(join(output, "concepts.json"), "utf8")).concepts[0];
      expect(concept.grounding).toBe("inferred");
      expect(concept.appearances.length).toBeGreaterThan(0);
    }
  }
});

test("compiler leaves failed and partially analyzed packages non-ready", async ({}, info) => {
  const root = info.outputPath("compile-failures");
  mkdirSync(root, { recursive: true });
  const binary = resolve("target/debug/codexia");
  const epub = resolve(process.env.CODEXIA_ACCEPTANCE_EPUB || "private/golden-books/source/alices-adventures-in-wonderland-pg11.epub");
  const fixture = resolve("tests/fixtures/analyzer.mjs");
  const cases = [];
  for (const mode of ["invalid-json", "partial-failure", "provider-timeout"]) {
    const directory = join(root, mode);
    mkdirSync(directory, { recursive: true });
    const command = join(directory, "analyzer.mjs");
    writeFileSync(command, `#!/usr/bin/env node
import {spawnSync} from 'node:child_process';
const chunks=[];for await(const chunk of process.stdin)chunks.push(chunk);
const input=Buffer.concat(chunks);const request=JSON.parse(input);
const mode=${JSON.stringify(mode)};
if(mode==='invalid-json'){process.stdout.write('not JSON');process.exit(0);}
if(mode==='partial-failure'&&request.context.chapter.chapter_id==='chapter_003'){process.stderr.write('intentional chapter failure');process.exit(1);}
if(mode==='provider-timeout'){
  const result=spawnSync(process.execPath,['-e','setTimeout(()=>{},10000)'],{timeout:20});
  process.stderr.write(result.error?.code||'missing timeout');process.exit(result.error?.code==='ETIMEDOUT'?124:1);
}
const result=spawnSync(${JSON.stringify(fixture)},[],{input,encoding:'utf8'});
process.stdout.write(result.stdout);process.stderr.write(result.stderr);process.exit(result.status??1);
`, { mode: 0o755 });
    const packageDir = join(directory, "package");
    const result = spawnSync(binary, ["compile", epub, "--out", packageDir, "--analyzer-command", command, "--analysis-jobs", "1", "--analyze-through", "3"], { encoding: "utf8", timeout: 10_000 });
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(1);
    expect(result.stderr).toContain(mode === "invalid-json" ? "invalid_output" : "analyzer_failed");
    const status = JSON.parse(readFileSync(join(packageDir, "compile_status.json"), "utf8"));
    expect(status.complete).toBe(false);
    expect(status.ready_stages).toEqual(["parse", "normalize"]);
    const recovery = execFileSync(binary, ["compile", epub, "--out", packageDir, "--analyzer-command", fixture, "--analysis-jobs", "1", "--analyze-through", "3"], { encoding: "utf8" });
    const recovered = JSON.parse(readFileSync(join(packageDir, "compile_status.json"), "utf8"));
    expect(recovered.complete).toBe(true);
    execFileSync(binary, ["validate", packageDir]);
    cases.push({ mode, exit_code: result.status, error: result.stderr, status, recovered, recovery });
  }
  writeFileSync(info.outputPath("compile-failures.json"), `${JSON.stringify(cases, null, 2)}\n`);
});
