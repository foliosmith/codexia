# Beta acceptance

Run from the repository root. EPUBs, model responses, state, traces and screenshots
remain in ignored `private/` storage.

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
by restricted exports without retaining excluded Obsidian files.
