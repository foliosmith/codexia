import { test, expect } from "@playwright/test";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { resolve, join } from "node:path";

test.use({ trace: "on", screenshot: "only-on-failure" });
test.setTimeout(120_000);

test("real HTTP upload, reader actions, exports, restart and browser reading", async ({ page }, info) => {
  const root = info.outputPath("library");
  mkdirSync(root, { recursive: true });
  const binary = resolve("target/debug/codexia");
  const fixture = resolve("tests/fixtures/analyzer.mjs");
  const epub = resolve(process.env.CODEXIA_ACCEPTANCE_EPUB || "private/golden-books/source/alices-adventures-in-wonderland-pg11.epub");
  const key = "codexia-acceptance-local-key";
  const keyFile = join(root, "key");
  writeFileSync(keyFile, key, { mode: 0o600 });
  const apiUrl = "http://127.0.0.1:18787";
  const readerUrl = "http://127.0.0.1:18788";
  const children = new Set();
  const requests = [];
  const contextFile = join(root, "agent-context.json");
  const apiArgs = ["api", root, "--bind", "127.0.0.1:18787", "--api-key-file", keyFile, "--analyzer-command", fixture, "--rate-limit-per-minute", "1000"];

  async function start(args, url) {
    const child = spawn(binary, args, { env: { ...process.env, CODEXIA_TEST_CONTEXT: contextFile }, stdio: ["ignore", "pipe", "pipe"] });
    children.add(child);
    let log = "";
    child.stdout.on("data", (chunk) => { log += chunk; });
    child.stderr.on("data", (chunk) => { log += chunk; });
    child.on("error", (error) => { log += String(error); });
    await expect.poll(async () => {
      if (child.exitCode !== null) throw new Error(log);
      try { return (await fetch(url, { headers: { "X-API-Key": key }, signal: AbortSignal.timeout(1000) })).status; }
      catch { return 0; }
    }, { timeout: 10_000 }).toBe(200);
    return child;
  }
  async function stop(child) {
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, "exit");
      child.kill("SIGTERM");
      await exited;
    }
    children.delete(child);
  }
  async function api(path, method = "GET", body, expected = 200) {
    const response = await fetch(`${apiUrl}${path}`, {
      method,
      headers: { "X-API-Key": key, "Content-Type": Buffer.isBuffer(body) ? "application/epub+zip" : "application/json" },
      body: body === undefined ? undefined : Buffer.isBuffer(body) ? body : JSON.stringify(body),
      signal: AbortSignal.timeout(15_000),
    });
    const value = await response.json();
    requests.push({ path, method, status: response.status });
    expect(response.status, JSON.stringify(value)).toBe(expected);
    return value;
  }
  try {
    let server = await start(apiArgs, `${apiUrl}/health`);
    const uploaded = await api("/v1/books", "POST", readFileSync(epub), 202);
    const bookId = uploaded.book_id;
    const bookPath = `/v1/books/${bookId}`;
    await expect.poll(async () => (await api(uploaded.status_href)).state, { timeout: 30_000 }).toBe("ready");
    const packageDir = join(root, "books", bookId, "package");
    const ir = JSON.parse(readFileSync(join(packageDir, "book_ir.json"), "utf8"));
    const chapters = ir.chapters.filter((chapter) => !chapter.is_noise && chapter.block_count > 0);
    expect(chapters.length).toBeGreaterThan(1);
    const chapterId = (chapter) => `chapter_${String(chapter.spine_index + 1).padStart(3, "0")}`;
    const first = chapterId(chapters[0]);
    const second = chapterId(chapters[1]);
    const partialContent = await api(`${bookPath}/chapters/${second}/content`);
    const partialBlock = partialContent.blocks.find((block) => Array.from(block.text).length > 15);
    const partialLocation = { chapter_id: second, block_id: partialBlock.block_id, char_offset: 5, epub_cfi: null };
    await api(`${bookPath}/ask`, "POST", {
      question: "What happens after this point?",
      reader_state: { session_id: null, current_location: partialLocation, read_until: partialLocation, completed_chapter_ids: [], progress_basis_points: 0 },
      spoiler_mode: "read_range",
    });
    const partialContext = JSON.parse(readFileSync(contextFile, "utf8")).context;
    const boundaryIndex = partialContent.blocks.findIndex((block) => block.block_id === partialBlock.block_id);
    const unreadIds = partialContent.blocks.slice(boundaryIndex + 1).map((block) => block.block_id);
    expect(partialContext.nearby_blocks.some((block) => unreadIds.includes(block.block_id))).toBe(false);
    const boundaryBlock = partialContext.nearby_blocks.find((block) => block.block_id === partialBlock.block_id);
    if (boundaryBlock) expect(Array.from(boundaryBlock.text).length).toBeLessThanOrEqual(5);
    expect(partialContext.chapter_analysis).toBeNull();
    await api(`${bookPath}/chapters/${second}/checkpoint`, "POST", {
      reader_state: { session_id: null, current_location: partialLocation, read_until: partialLocation, completed_chapter_ids: [], progress_basis_points: 0 },
      spoiler_mode: "read_range",
    }, 403);
    for (const suffix of ["/package", "/map", "/concepts", "/claims", `/chapters/${first}/analysis`]) await api(bookPath + suffix);
    const content = await api(`${bookPath}/chapters/${first}/content`);
    const block = content.blocks.at(-1);
    const location = { chapter_id: first, block_id: block.block_id, char_offset: 0, epub_cfi: null };
    const created = await api("/v1/reader-sessions", "POST", { book_id: bookId, current_location: location, spoiler_mode: "read_range" }, 201);
    const session = await api(`/v1/reader-sessions/${created.session_id}`, "PATCH", {
      current_location: location,
      read_until: { ...location, char_offset: Array.from(block.text).length },
      progress_basis_points: 100,
    });
    const reader_state = {
      session_id: session.session_id,
      current_location: session.current_location,
      read_until: session.read_until,
      completed_chapter_ids: [],
      progress_basis_points: session.progress_basis_points,
    };
    const source = { block_id: block.block_id, start_char: 0, end_char: Array.from(block.text).length, text_fingerprint: block.text_fingerprint };
    const explanation = await api(`${bookPath}/explain`, "POST", { selected_text: block.text, source_ref: source, reader_state, spoiler_mode: "read_range" });
    expect(explanation.cards[0].source_refs).toEqual([source]);
    const answer = await api(`${bookPath}/ask`, "POST", { question: "What happens in the unread final chapter?", reader_state, spoiler_mode: "read_range" });
    expect(answer.spoiler_boundary.excluded_chapter_ids).toContain(second);
    const checkpoint = await api(`${bookPath}/chapters/${first}/checkpoint`, "POST", { reader_state, spoiler_mode: "read_range" });
    const cp = checkpoint.cards[0].content;
    const reflection = await api(`${bookPath}/chapters/${first}/reflect`, "POST", { reader_state, checkpoint_id: cp.checkpoint_id, question_id: cp.recall_questions[0].question_id, answer: "The chapter was registered." });
    expect(reflection.cards[0].card_type).toBe("reflection");
    await api(`${bookPath}/chapters/${second}/checkpoint`, "POST", { reader_state, spoiler_mode: "read_range" }, 403);
    const unread = (await api(`${bookPath}/chapters/${second}/content`)).blocks[0];
    await api(`${bookPath}/explain`, "POST", { selected_text: unread.text, source_ref: { block_id: unread.block_id, start_char: 0, end_char: Array.from(unread.text).length, text_fingerprint: unread.text_fingerprint }, reader_state, spoiler_mode: "read_range" }, 403);
    await api(`/v1/reader-sessions/${session.session_id}/notes`, "POST", { chapter_id: first, block_id: block.block_id, text: "acceptance-first-chapter-note" }, 201);
    await api(`/v1/reader-sessions/${session.session_id}/notes`, "POST", { chapter_id: second, block_id: unread.block_id, text: "acceptance-unread-chapter-note" }, 201);
    for (const format of ["json", "markdown", "obsidian"]) {
      for (const scope of ["whole_book", "chapters", "read_range"]) {
        const exported = await api(`${bookPath}/exports`, "POST", { format, scope, chapter_ids: [first], session_id: session.session_id }, 201);
        expect(exported.format).toBe(format);
        expect(exported.scope).toBe(scope);
        if (format === "json") {
          const data = JSON.parse(readFileSync(exported.local_path, "utf8"));
          const ids = data.chapters.map((chapter) => chapter.chapter_id);
          if (scope === "whole_book") expect(ids).toContain(second);
          else { expect(ids).toContain(first); expect(ids).not.toContain(second); }
          if (scope === "chapters") expect(ids).toEqual([first]);
        } else if (format === "markdown") {
          expect(readFileSync(exported.local_path, "utf8").length).toBeGreaterThan(0);
          const markdown = readFileSync(exported.local_path, "utf8");
          expect(markdown).toContain("acceptance-first-chapter-note");
          expect(markdown.includes("acceptance-unread-chapter-note")).toBe(scope === "whole_book");
        } else {
          const files = readdirSync(exported.local_path);
          expect(files.some((name) => name.startsWith(first))).toBe(true);
          expect(files.some((name) => name.startsWith(second))).toBe(scope === "whole_book");
        }
      }
    }
    const usage = await api("/v1/usage");
    expect(usage.compile_count).toBe(1);
    await stop(server);
    server = await start(apiArgs, `${apiUrl}/health`);
    expect((await api(uploaded.status_href)).state).toBe("ready");
    expect((await api("/v1/usage")).compile_count).toBe(usage.compile_count);
    expect(await api(`/v1/reader-sessions/${session.session_id}`)).toEqual(session);

    await start(["serve", packageDir, "--state-dir", join(root, "reader-state"), "--bind", "127.0.0.1:18788"], `${readerUrl}/v1/bootstrap`);
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await page.goto(`${readerUrl}/#read/${second}`);
    await expect(page.locator(".reader-main")).toBeVisible();
    const paragraph = page.locator(".reader-block").filter({ hasText: /Alice/ }).first();
    const selected = await paragraph.evaluate((element) => {
      const node = element.firstChild;
      const text = node.textContent;
      const start = text.indexOf("Alice");
      const range = document.createRange();
      range.setStart(node, start);
      range.setEnd(node, start + 5);
      getSelection().removeAllRanges();
      getSelection().addRange(range);
      element.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      return { text: "Alice", start: Array.from(text.slice(0, start)).length, block_id: element.dataset.blockId };
    });
    const explainRequest = page.waitForRequest((request) => request.url().endsWith("/explain"));
    await page.locator('#selection-menu [data-intent="explain"]').click();
    const payload = (await explainRequest).postDataJSON();
    expect(payload.selected_text).toBe(selected.text);
    expect(payload.source_ref.start_char).toBe(selected.start);
    expect(payload.source_ref.end_char).toBe(selected.start + 5);
    await expect(page.locator(".source-button").first()).toBeVisible();
    await page.locator("#chapter-end").scrollIntoViewIfNeeded();
    await expect(page.locator("#chapter-end")).toHaveAttribute("data-visible", "true");
    await page.locator(".source-button").first().click();
    await expect(page.locator(`[id="${selected.block_id}"]`)).toBeInViewport();
    await page.locator("#checkpoint-button").click();
    await expect(page.locator(".checkpoint-question").first()).toBeVisible();
    await page.locator(".checkpoint-question textarea").first().fill("Yes, this chapter was registered.");
    await page.locator("[data-reflect]").first().click();
    await expect(page.locator("#reflection-result .agent-card")).toBeVisible();
    await page.reload();
    await expect(page.locator(".reader-main")).toBeVisible();
    const persisted = await page.evaluate(() => JSON.parse(document.body.dataset.codexiaReaderState));
    expect(persisted.current_location.chapter_id).toBe(second);
    expect(persisted.progress_basis_points).toBeGreaterThan(0);
    expect(errors).toEqual([]);
    await page.screenshot({ path: info.outputPath("reader.png"), fullPage: true });
  } finally {
    await Promise.all([...children].map(stop));
    writeFileSync(info.outputPath("http-requests.json"), `${JSON.stringify(requests, null, 2)}\n`);
  }
});
