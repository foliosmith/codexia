# Codexia benchmarks

Run from any directory with Node.js 22 or later:

```sh
node benchmarks/runner.mjs validate
node benchmarks/runner.mjs validate --catalog <local-catalog.json>
node benchmarks/runner.mjs run --output private/benchmarks/runs/first
node benchmarks/runner.mjs resume --output private/benchmarks/runs/first
node benchmarks/runner.mjs report --output private/benchmarks/runs/first
node benchmarks/runner.mjs review-template --output private/benchmarks/runs/first
node benchmarks/runner.mjs score --output private/benchmarks/runs/first --reviews <completed-review.json>
node benchmarks/runner.mjs export --output private/benchmarks/runs/first --reviews <completed-review.json>
```

The public v0.0 development suite contains eight original controlled scenarios.
Its source text is CC0-1.0. Gold annotations are **draft**, not independently
reviewed evidence. This suite is not the planned 40-question bilingual nonfiction
baseline or a held-out evaluation.

Code, schemas, general rubrics and rights-cleared development fixtures belong
here. Restricted sources, private questions and gold, held-out identities,
reviews, packages and all actual run artifacts belong in `private/benchmarks/`.
Public development gold is intentionally visible; do not count it as held out.
The local catalog uses the same schema, with paths relative to that catalog.
Do not add local absolute paths, credentials or private-source pointers to the
public catalog. Missing private material is an error, not a passing empty suite.

See [the protocol](protocol.md). Existing package quality gates remain in
[quality-gates.md](../docs/quality-gates.md). No independent npm package,
service, database or new evaluation UI is required.

Execution requires Python 3 (standard-library EPUB construction), an offline-built
`target/debug/codexia`, and the existing `tests/fixtures/analyzer.mjs` for explicitly
labelled offline registration. Use `cargo build --offline` first. Every attempt
starts a real loopback Reader and a fresh state directory; the default adapter
extracts offered source blocks and never judges semantic correctness.

`--agent-command <executable>` supplies an offline/replay adapter using the existing
JSON stdin/stdout contract. It is trusted executable code, not a sandbox; never
use this option for an unbudgeted online provider. Online runs and held-out runs
are currently rejected/not implemented. `--repeat 3` records three independent
sessions per case; pass the same repeat and adapter on resume/report.

All actual outputs must be under this checkout's `private/`, with no symlink
components. Runs create a fresh directory; resume checks suite, binary, adapter,
benchmark code and package hashes, retains finished attempts and marks interrupted
attempts cancelled. Retries require a new run, so failures cannot be overwritten.
Resume reclaims a same-host lock only when its recorded owner no longer exists;
live and foreign-host locks are rejected. Interrupted captures retain their known
calls and unknown usage. A hard-killed runner may leave Reader/adapter processes;
inspect and stop those before treating failed-call accounting as final. Offline
compilation can resume an existing package after source identity is checked.

Exit 0 means the engineering run/report has no hard failures, **not** that model
quality passed. `report.json` explicitly says `inconclusive` without reviewed
semantics. Inspect `trials.jsonl`, `scores.jsonl` and per-attempt artifacts for
failures, exact contexts and evidence. Actual logs and answers stay private.

`review-template` writes an unfinished private review template. Semantic scoring
requires reviewed gold (with an attributed reviewer) and four explicit checks:
citation support, key-point coverage, spoiler boundary and attribution. Each
check records 0–3, supports/refutes/insufficient, evidence and any hard failure.
An attribution field records a claim of review; it cannot prove reviewer quality
or independence. Do not mark machine-authored gold reviewed without that work.
The shipped draft gold deliberately prevents accepting semantic reviews.

`score` verifies case/gold/artifact identities and archives each assessment by
review hash. Partial reviews remain visibly incomplete. A minimum score of 3
across required dimensions is necessary for task success; hard failures always
fail. Even complete passing reviews of offline outputs cannot establish online
model quality. `report` without `--reviews` rebuilds the structural report; prior
assessments remain under `assessments/`. Changing judge versions requires rescoring
both candidates, not comparing old and new scores directly.

`export` writes `export-summary.json` inside the private run directory using an
explicit aggregate-field allowlist. It excludes case/book identities, paths,
reviewer notes, text and attachments. It does not publish or copy anything into
the public tree. Review the aggregate before sharing it externally.

To use a private real EPUB, keep its independent anchor annotation in the book
JSON and add `epub: {"path":"sources/book.epub","sha256":"..."}` to that book's
local catalog entry. Both annotation and EPUB hashes are checked. Anchor hrefs
and exact normalized paragraph text must uniquely match the runtime source; a
mismatch is a mapping failure, never an automatically repaired gold answer.
The annotation format supports nested EPUB-relative chapter hrefs. Annotate all
reading endpoints and required evidence; it need not copy the whole book.
The default compiler still uses offline registration and does not create a
semantic baseline from the real EPUB. Version, rights and human gold review
remain corpus preparation work.
