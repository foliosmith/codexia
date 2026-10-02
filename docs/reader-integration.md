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
new compile. `--force` disables reuse. Uploads compare the same analysis identity as the CLI. A different profile or
adapter version creates a separate candidate under `versions/<analysis_key>/`.
Only a validated candidate replaces `active.json`; an unsuccessful upgrade keeps
the previous runtime and package readable, including after restart. Status
reports the attempted `profile` and `analysis_key`, plus `active_profile` when a
previous package remains available. Repeating an active identical request reuses
the job; a conflicting active variant returns 409. Retry uses the recorded profile
and the current adapter identity.

CLI replacement builds beside the destination in `.NAME.pending` and preserves
the prior package until validation succeeds. Publication uses `.NAME.previous`
for rollback and a sibling native file lock. If interrupted between directory
renames, the next compiler run restores a missing destination from that backup.
A leftover backup alongside a destination is reported for explicit recovery,
rather than silently deleted. Do not store unrelated files in these reserved
compiler paths. Automatic incremental range extension remains separate work.

## Local trial operation and limits

The trial entry is `codexia serve <package> --state-dir <state> --bind 127.0.0.1:8787 --agent-command <adapter>`.
It is a single-user, local process. Unauthenticated Reader/Studio listeners reject non-loopback addresses. Keep the package, state and provider run directory owner-only (`chmod 700`); API key files must be owner-only (`chmod 600`) and contain at least 16 nonblank characters. Remote access requires a separately configured authenticated TLS reverse proxy; it is outside this trial.

`GET /health/live` reports the listener; `/health/ready` reports adapter executable availability and whether the Reader is in model or offline mode. Readiness does not probe provider credentials or make a paid request. Stop local CLI compilation with the terminal's interrupt (process group); completed chapter files remain available for retry. The authenticated API also accepts `POST /v1/books/{id}/cancel`, marks an interrupted compile failed, and supports the existing retry endpoint.

Defaults are 8 MiB per HTTP body, 16 KiB headers, 32 simultaneous connections, four analyzer calls per process and one API compilation. A request over the body limit returns JSON `413 request_too_large`. EPUB container, decompression, XML and document limits are enforced by pagelet; a failed spine parse aborts compilation rather than publishing omitted content. Larger books must be compiled through the CLI, which still applies decompression and model-context limits.

Positive integer environment overrides:

| Variable | Default | Behavior at limit |
| --- | --- | --- |
| `CODEXIA_MAX_CONTEXT_BYTES` | 524288 | Reject serialized analyzer input before spawning; Reader HTTP 413 |
| `CODEXIA_MAX_OUTPUT_BYTES` | 4194304 | Reject oversized analyzer stdout; stderr capped at 65536 bytes |
| `CODEXIA_READER_TIMEOUT_MS` | 120000 | Terminate owned process tree, HTTP 504 |
| `CODEXIA_ANALYZER_TIMEOUT_MS` | 1200000 | Fail the current compiler stage; keep validated chapters |
| `CODEXIA_MAX_ANALYZERS` | 4 | Reject additional work, Reader HTTP 503 |
| `CODEXIA_MAX_COMPILES` | 1 | API rejects additional compilations with HTTP 503 |
| `CODEXIA_COMPILE_TIMEOUT_MS` | 3600000 | Terminate compiler and descendants, retain failed/retryable job |

Reader actions accept an optional unique `request_id` (1–80 ASCII letters, digits, hyphens or underscores). The Reader displays a cancel button; `POST /v1/books/{id}/actions/{request_id}/cancel` requests termination and the original call returns 409. This differs from closing a tab, which does not cancel server work. Unix termination uses native `pgrep`/`kill`, Windows uses `taskkill /T`; adapters must not daemonize or detach descendants.

Diagnostics are under `<state>/diagnostics` and `<package>/diagnostics`: action ID, task, analysis identity, allowed ranges, retrieved block IDs, bytes, elapsed time, validation status and usage. They exclude questions, source prose, answers, credentials and provider stderr. Adapters can write `{input_tokens,output_tokens}` to the supplied `CODEXIA_USAGE_FILE`. Unknown usage and cost remain `null`; explicit `CODEXIA_INPUT_USD_PER_MILLION` and `CODEXIA_OUTPUT_USD_PER_MILLION` yield a simple uncached-token estimate, not invoice reconciliation. Completed chapter artifacts and compile status identify retries/reuse; a reused chapter produces no new model-call event.

The online adapter retains redacted provider events by default and removes temporary schema/output files. Only acceptance runs explicitly set `CODEXIA_CAPTURE_CONTENT=1`; those artifacts can contain book or reader content and must be owner-only and deleted after review (trial policy: within seven days). Do not enable captures for participant sessions. Native provider tools may have their own logging policies.

Back up the package and state directories together while the server is stopped. Restore both to a new owner-only directory, run `codexia validate <package>`, then start the Reader with the restored state. Failed upgrade candidates do not replace the active package. A backup restore rehearsal and multi-hour soak remain separate production gates.
