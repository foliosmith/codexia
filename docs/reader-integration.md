# Third-party Reader Integration

## 1. Compile and wait

Send the EPUB bytes to `POST /v1/books`, then poll the returned `status_href`
until `state` is `ready`. Authenticate with `X-API-Key` or `Authorization:
Bearer …`.

## 2. Create a reader session

Create a session with the first visible `chapter_id`. Persist the returned
`session_id`. Navigation updates `current_location` only. An explicit reading
confirmation PATCHes `read_until` with a block ID and Unicode-scalar character
endpoint: it confirms the prefix of that chapter, never intervening chapters.
The server merges each chapter's endpoint monotonically into `read_coverage`
and derives `completed_chapter_ids` and progress from it. Client-supplied progress
is not used to grant coverage. The returned `read_until` remains a compatibility
high-water location; contextual authorization uses persisted coverage.

Send the returned locations, progress and coverage with each action. Sessions
saved before coverage existed start with empty coverage and require reading
confirmation again; opening or restoring a location grants no reading access.
Stateless legacy requests without coverage declare only the prefix of their
`read_until` chapter. `current_chapter` explicitly permits only the current
chapter; `full_book` explicitly permits the whole book.

The reference Reader uses “已读至此” on a selection and “确认本章已读” at chapter
end. A scroll or chapter jump does not mark content read. Chapter summaries and
checkpoints require full chapter coverage in read-range mode. Read-range exports
include completed chapters only; partial chapters are omitted.

## 3. Render the package

Use the package and chapter endpoints for content. Treat `block_id` as the DOM
anchor and keep its `text_fingerprint`; source navigation must resolve through
these stable IDs rather than fuzzy text search.

## 4. Invoke the Book Agent

`explain`, `ask`, `checkpoint`, and `reflect` require the current `ReaderState`.
Do not fabricate a wider `read_until`. Render the returned Reader Cards using
[`reader-card.schema.json`](../schemas/reader-card.schema.json), and make every
`source_ref` an explicit jump back to the source block.

## 5. Handle errors and limits

- `401`: missing or invalid API key;
- `403`: source or response crossed the spoiler boundary;
- `429`: retry after the rate-limit window;
- `502`: analyzer output failed the Reader Card contract.

Use `GET /v1/usage` for request, byte and estimated-cost totals. Webhooks emit
`book.processing.completed` or `book.processing.failed`; consumers should be
idempotent by `(event, book_id)`.

The TypeScript reference client lives in [`sdk/typescript`](../sdk/typescript/).

## 6. Recover a failed compilation

The API persists each book's job in `books/<id>/job.json`. On startup it validates
each completed package before registering it. Missing, interrupted or corrupt
packages remain `failed` without preventing other books from loading. A failure
is not retried automatically. With the source EPUB and analyzer available, send
`POST /v1/books/<id>/retry` once and poll the returned status URL. Each accepted
request starts one attempt; ready or active jobs return 409. No unbounded retry
loop is started by the server.

The compiler atomically saves individually validated chapter analyses and its
`analysis_key`, attempt number, completed chapter artifacts, stages and error in
`compile_status.json`. Resuming rechecks chapter metadata and grounding before
reuse; missing or invalid chapters are regenerated. Synthesis is rerun and the
complete package is validated before the API publishes ready. Native output-file
locking prevents a second compiler from writing the same directory, including
when an orphan compiler still runs after the API exits. File locking requires
Rust 1.89 or later to build.

Resume identity includes source, profile, analyze-through, analyzer executable
content, compiler/pipeline/protocol versions, `CODEXIA_ANALYZER_MODEL`,
`CODEXIA_ANALYZER_PROMPT_VERSION`, and optional `CODEXIA_ANALYZER_REVISION`.
Adapters whose configuration lives elsewhere must change the revision when it
changes. Old packages without this key remain readable but are not reused by a
new compile. `--force` disables reuse. Profile upgrades through upload, automatic
incremental range extension, and preserving a previous package during forced
replacement remain separate work; retry is for failed jobs of the recorded
profile.
