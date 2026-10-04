# Codexia benchmarks

Run from any directory with Node.js 22 or later:

```sh
node benchmarks/runner.mjs validate
node benchmarks/runner.mjs validate --catalog <local-catalog.json>
node benchmarks/runner.mjs run --output private/benchmarks/runs/first
node benchmarks/runner.mjs resume --output private/benchmarks/runs/first
node benchmarks/runner.mjs report --output private/benchmarks/runs/first
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
A crash may leave `.lock`; verify its owner is no longer running before removing
it. A failed compile must currently be restarted in a new run.

Exit 0 means the engineering run/report has no hard failures, **not** that model
quality passed. `report.json` explicitly says `inconclusive` without reviewed
semantics. Inspect `trials.jsonl`, `scores.jsonl` and per-attempt artifacts for
failures, exact contexts and evidence. Actual logs and answers stay private.
