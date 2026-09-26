# Beta acceptance

Run from the repository root. EPUBs, model responses, state, traces and screenshots
remain in ignored `private/` storage.

## Online compiler (5.2)

```sh
cargo build --offline
CODEXIA_ANALYZER_MODEL=gpt-6-astra node scripts/accept-online.mjs \
  private/golden-books/catalog.json private/acceptance/online
CODEXIA_ANALYZER_MODEL=gpt-6-astra node scripts/compare-analyzers.mjs \
  private/golden-books/packages/alice chapter_003 private/acceptance/comparison
tests/node_modules/.bin/playwright test tests/compile.spec.mjs \
  --workers=1 --output=private/acceptance/compile
```

The online adapter requires an authenticated Codex CLI with access to the chosen
model. It uses the official ChatGPT HTTPS transport without changing user config.
The adapter uses low model reasoning effort for every processing profile; the
basic/standard/deep profiles control analysis scope and output detail.
It supplies book text as data, disables tools, and stores structured provider
outputs and usage locally. `CODEXIA_ANALYZER_TIMEOUT_MS` defaults to 900000
(1200000 for deep requests).
Chapter prompts use `online-v2`; bounded whole-book synthesis uses `synthesis-v3`.
Basic/standard/deep syntheses cap concept, claim and entity catalogs at 8/16/32
objects while retaining every chapter's map entry and checkpoint. These bounds
address the observed unbounded synthesis timeout and require semantic review
against Golden Book reference annotations.
The adapter constrains fingerprints to supplied values within provider schema
limits and rejects invalid chapter references before starting book synthesis.
Rust still independently validates the complete package. Failed output remains
recorded; it is never silently rewritten to pass validation.

Use a fresh output directory for each acceptance run; existing reports are preserved.
The compiler run covers three full standard books, one full deep book and one
basic run for the cost baseline. It validates
every package, checks monotonic readiness and full chapter coverage, and proves
that a second compile invokes no provider. `report.json` records the commit,
binary/analyzer hashes, stage observations, timings, usage, validation and cache
results. Provider failures leave a failed report; they are never counted as passes.
Append a book ID and profile to run one additional case, such as `alice-pg11 basic`.
Replay evidence retains the original response path and usage attribution. Its
latency basis is the original provider timeline, including recovery gaps, rather
than the near-zero time needed to read the recording.

The Studio comparison keeps the registered deterministic analyzer as its base and
compares an actual online reanalysis of the first narrative Alice chapter. Its
`comparison.json` includes both versions, the document diff and both evaluations.
This is an analyzer comparison, not evidence that the deterministic baseline had
semantic quality. The failure test covers malformed output, a failed chapter among
successful chapters, and an actual timed-out provider subprocess, then verifies
recovery by recompiling the failed package. Compiler-owned job deadlines remain a
phase-six item; adapters must enforce their own provider timeout.
If one chapter fails, successful peer responses are not published as chapter
checkpoints. Recompilation may repeat their provider work; the report records this
limitation rather than claiming incremental recovery.

## HTTP and Reader (5.3)

```sh
cargo build --offline
npm install --prefix tests
npm exec --prefix tests -- playwright install chromium
npm --prefix tests run e2e
```

Requires the registered Alice EPUB at
`private/golden-books/source/alices-adventures-in-wonderland-pg11.epub`, or set
`CODEXIA_ACCEPTANCE_EPUB` to an equivalent Alice EPUB with multiple chapters.
The multi-book regression also requires
`private/golden-books/source/the-souls-of-black-folk-pg408.epub`.
Ports 18787 and 18788 must be free. The test starts and stops its own servers.

The test uploads real EPUB bytes over HTTP, polls ready, reads package endpoints,
exercises explain/ask/checkpoint/reflect and sessions, verifies all three export
formats and scopes, and restarts the API to check book/session/usage recovery.
Chromium exercises text selection offsets, source navigation, chapter-end review,
reflection, and progress recovery. The deterministic provider isolates transport
and boundary behavior; it does not establish online model quality.

Artifacts are written under `private/acceptance/playwright/`: `trace.zip`,
`reader.png`, `http-requests.json`, and the isolated API library. Open a trace with
`npm exec --prefix tests -- playwright show-trace <trace.zip>`.

Regression cases include partially read paragraph context and chapter summaries,
premature checkpoints, unread chapter actions, and whole-book exports followed
by restricted exports without retaining excluded Obsidian files. Multi-book checks
verify session ownership and export downloads across restart. New resource IDs
include the book ID; ambiguous legacy IDs return `409 ambiguous_resource` before
any session mutation instead of selecting an arbitrary book.

For real online Reader Cards against an already validated package:

```sh
CODEXIA_ANALYZER_MODEL=gpt-6-astra node scripts/accept-reader-agent.mjs \
  <package-directory> private/acceptance/reader-agent chapter_003
```

This uses port 18791 and records actual explain/ask/checkpoint/reflect responses.
Review the saved answer to the unread-ending question against the Golden Book's
spoiler annotations; response shape and valid citations alone cannot prove
semantic non-disclosure.

## Failure recovery (5.6.3)

`tests/recovery.spec.mjs` uploads a healthy book and a book whose deterministic
analyzer fails one chapter. It restarts the API, explicitly retries, kills only
its own isolated process group during analysis, corrupts one saved chapter, and
resumes. It verifies that healthy books remain available, valid chapters are
reused, corrupt/unfinished chapters are regenerated, and incomplete or corrupt
packages never become ready. A simultaneous compiler targeting the same output
must fail the native file lock. Artifacts include `recovery-states.json`, the
provider call ledger, `job.json`, and per-book `compile_status.json`.

No online model is needed for this recovery test. It verifies process termination
and restart on the tested host, not power-loss durability or multi-instance
scheduling.
