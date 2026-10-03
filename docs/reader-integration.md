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
It is a single-user, local process. Unauthenticated Reader/Studio listeners reject non-loopback addresses and foreign browser origins/hosts. Keep the package, state and provider run directory owner-only (`chmod 700`); API key files must be owner-only (`chmod 600`) and contain at least 16 nonblank characters. Remote access requires a separately configured authenticated TLS reverse proxy; it is outside this trial.

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

## Logical chapters and package compatibility

Packages requiring splits within a spine item or a chapter spanning files use format `0.2`. `book_ir.json.logical_sections` partitions every retained physical block in source order into named reading units; each range records its physical `spine_index` and ordered `block_ids`. Physical chapters, IDs, fingerprints and EPUB source references are unchanged. Compiler analysis, synthesis, Reader progress/checkpoints and Studio use the same logical view. `--analyze-through` counts those reading units (including front matter).

The overlay requires ordered, uniquely resolved TOC leaf anchors at retained block boundaries. Unresolved or inline anchors, overlapping entries and unusable TOCs conservatively fall back to the existing physical units; the Reader labels that basis. A TOC already matching physical boundaries keeps format `0.1`. Heading text is not guessed into new chapters. For the Pride and Prejudice fixture, 16 physical units map to 64 reading units, including all 61 Chapter-labelled TOC items and front/back matter.

Existing `0.1` packages remain readable. A `0.2` package missing or corrupting its partition is rejected. Reader state records the layout identity; state from a different layout is rejected explicitly, preserving the old file. Use a separate state directory for the newly compiled package and retain the old package/state for existing annotations. For an API library, archive the affected book's state directory before activating the new layout; there is no silent progress/annotation migration. New schema versions do not reuse old analysis cache identities.

## Evidence retrieval

`ask` derives persisted reading coverage first, clips partial blocks, ranks matching words (including Chinese character bigrams), and supplies at most eight source blocks within half the configured input budget. The remaining budget covers instructions/schema; the shared command boundary enforces the full serialized limit. No matching evidence produces an explicit insufficient-evidence card without a model call. Matching evidence that cannot fit produces HTTP 413, rather than pretending it was absent. The `connect` intent retrieves matching blocks from previously read earlier chapters.

Model citations must refer to the raw blocks supplied for that action, remain inside their visible character ranges and match the original fingerprint. This proves provenance and scope, not semantic truth; participant review still checks whether the quoted evidence supports the answer. The offline Reader displays extracts instead of fabricating an answer. The lexical approach can miss paraphrases/synonyms; ask with specific terms or select the passage. Chapter prompts now carry bounded, source-addressable `source_excerpts` from prior chapters, never a first-600-character pseudo-summary (prompt protocol 0.2; stored analysis schema remains 0.1).

## Trial scope and decision criteria

Use this local Web Reader only, with an explicitly configured model adapter, a compiled package and a new compatible state directory. Offer selection explanation, prior-context connection, scoped questions, logical chapter review, recall feedback, source jumps and exports. Keep public deployment, SDK onboarding, webhooks, other clients and billing disabled for this round. When TOC mapping falls back to physical files, disclose it and do not use that book to assess logical chapter review.

Preparation is complete when deterministic boundary/recovery tests pass and the selected book is validated. This is not a claim that the updated prompts have passed a fresh live-provider semantic review. Before expanding access, review actual trial outputs with the existing artifact-bound semantic gate.

Proposed observation: 3–5 consenting readers, two sessions over seven days, using a rights-cleared nonfiction chapter and each reader's normal unaided workflow as comparison. Each session includes one difficult passage, source verification, an immediate explanation in the reader's own words, and a next-day recall question. Do not retain questions/answers in default logs; record aggregate human-scored outcomes separately with consent.

Proceed only with zero unread-content disclosures, unsupported source jumps or lost reading state; at least 80% of reviewed answers must be supported by cited text, and at least three readers must prefer continuing the workflow without worse delayed recall than their comparison. Observe median/p95 action wait, initial compilation time, cancellation/recovery success and per-book token/cost estimates; set an explicit local spending cap before real model use. If evidence support or delayed recall is weak, adjust explanation/retrieval before adding clients. Record insufficient evidence and budget rejections as unsuccessful attempts, not successful answers. Invitations, actual observations and the continue/change decision remain pending.

## Reader adapter protocol 0.2 and focused live acceptance

Reader model requests include `evidence_refs`, calculated from visible source blocks and the validated selection. Copy an entry verbatim when citing; the runtime rejects invented subranges even when their numeric offsets fall inside a block. This avoids asking a model to count Unicode characters. Refusals without supporting citations must use `grounding: inferred`, empty `source_refs`, and zero confidence. Reader HTTP request/response types and stored package schemas are unchanged by this adapter protocol revision.

`scripts/accept-reader-agent.mjs <package> <fresh-output-dir> [chapter-id] [scenario-json]` can run a focused real-provider probe. Set `CODEXIA_ANALYZER_MODEL`; an optional `CODEXIA_ACCEPTANCE_ANALYZER` selects another adapter. Scenario fields are `selection_contains`, `question`, `unread_question`, `reflection_answer`, `max_reflection_score`, and `exact_evidence_ranges`. Use a deliberately wrong recall answer to test corrective feedback. The exact-range check expects complete source blocks in this full-paragraph probe.

The report retains adapter/binary hashes, model, analysis identity, timings, provider usage, outputs and quoted citations in an owner-only directory. Its `passed` field covers transport/source contracts and the supplied score ceiling; semantic review and participant learning outcomes remain separate. Keep automatic probe state separate from participant state. Reusing an existing real-model package tests current Reader behavior, not a fresh compiler run. Review refusals as well as successful answers: a fluent response alone is not acceptance.
