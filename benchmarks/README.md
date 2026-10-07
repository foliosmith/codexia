# Codexia benchmarks

Run from any directory with Node.js 22 or later:

```sh
node benchmarks/runner.mjs validate
node benchmarks/runner.mjs validate --catalog <local-catalog.json>
node benchmarks/runner.mjs run --output private/benchmarks/runs/first
node benchmarks/runner.mjs run --mode cold-compile-reader --output private/benchmarks/runs/cold
node benchmarks/runner.mjs run --candidate A1 --output private/benchmarks/runs/raw-source
node benchmarks/runner.mjs compare --left private/benchmarks/runs/first --right private/benchmarks/runs/raw-source --output private/benchmarks/comparisons/first
node benchmarks/runner.mjs audit-corpus --catalog <dev-catalog.json> --holdout <holdout-catalog.json> --output private/benchmarks/corpus-audit
node benchmarks/runner.mjs resume --output private/benchmarks/runs/first
node benchmarks/runner.mjs report --output private/benchmarks/runs/first
node benchmarks/runner.mjs review-template --output private/benchmarks/runs/first
node benchmarks/runner.mjs prepare-review --output private/benchmarks/runs/first
node benchmarks/runner.mjs prepare-calibration --output private/benchmarks/calibration/inputs
node benchmarks/runner.mjs calibrate --judgments <judge-outputs.json> --output private/benchmarks/calibration/result
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

New runs record execution identity separately from gold. `score`/`report` may
reevaluate existing answers after gold or scorer updates without calling a
provider; changed questions, sources and reading boundaries are still rejected.
Execution resume remains strict, including gold and executable identities. Legacy
runs lacking execution identity require the original suite. Each report records
the current suite and scorer hashes. Old archived assessments remain intact.

`prepare-review` writes randomly named private packets and a separate bindings
file under `review-packets/`. Packets omit candidate/model names, attempt IDs,
card IDs and filesystem paths; source evidence is clipped to each step's allowed
Unicode range. Gold may name forbidden unread facts for leakage assessment and
is explicitly distinguished from allowed evidence. Share packets only with the
reviewer, never with the candidate; retain the bindings locally. Answer prose
can still hint at identity, so this is metadata blinding, not perfect anonymity.

Twenty original public calibration outputs cover correct paraphrases, omitted
conditions, false attribution, uncertainty, cross-chapter counterexamples,
refusal, empty answers, wrong reflections, injection and a Chinese boundary.
`prepare-calibration` separates judge inputs from expected labels. `calibrate`
checks input identity, duplicates, evidence, agreement and missing samples.
These model-authored labels remain draft: agreement cannot establish calibration
without an attributed human review of every reference expectation.
The command evaluates recorded judge outputs and never calls a paid provider.

After actual human review, provide the private reference file through
`--calibration <reference.json>` to both calibration commands. Mark the dataset
and each expectation `reviewed`, and record its `reviewer`. The admission gate
requires at least 20 samples, all judgments present, and exact agreement on each
score, verdict and hard-failure flag. Any disagreement in a complete reviewed
set returns nonzero. Passing means agreement on that frozen reference set, not
universal judge accuracy or proof that the attributed human work occurred.
The repository's default draft references are never promoted automatically.

`--candidate A1` is the raw-source ablation for **fixed-package Reader** tasks:
it uses the same reading boundary and lexical retrieval as A0, but strips chapter
analysis, related concepts, argument flow and reflection expected points before
invoking the same answering adapter. The current A0 `ask` path already excludes
those fields, so A0/A1 ask inputs are effectively equivalent; the name alone is
not a distinct architecture. Reflection uses the same package checkpoint question
as the task, without its compiled answer. Independent A1 checkpoint generation
and a full cold-compile comparison remain unimplemented.

`compare` requires matching inputs, gold, scorer, answering adapter, repeat count,
execution mode and judge versions. It checks artifact hashes, retains all paired
attempts and reports unknowns separately from regressions/improvements. Reports
remain descriptive and inconclusive with offline evidence; they do not select a
winner from missing semantic judgments. Changed judges require both runs to be
rescored first. Comparison outputs stay private.

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

Reports retain the existing Studio package evaluation separately from task
scores. An offline registration package can be structurally valid while its
required semantic outputs or Beta review are missing. Retrieval diagnostics
measure whether a complete annotated evidence set was offered to the provider;
they do not measure whether an answer was supported. Draft gold diagnostics stay
labelled draft; attempts lacking source mappings remain unassessed.

For the focused no-provider regression suite, build the local binary and run
`npm --prefix tests run benchmark`. The suite uses original fixtures, exercises
the real Reader, and retains evidence under `private/acceptance/benchmarks/`.
This is a runnable developer gate, not an installed PR/release automation.

Execution modes are explicit. The default `fixed-package-reader` compiles one
registration package per book and shares it across isolated Reader attempts.
`cold-compile-reader` creates a fresh package for every applicable attempt and
includes that compilation in its wall time. Resume requires the original mode;
finished attempts are not recompiled, and an interrupted cold attempt remains a
failure rather than being relabelled a warm success. Warm-cache experiments are
not implemented. Both modes still use the offline registration compiler adapter.
In cold mode, the case's `timeout_ms` covers source preparation, compilation,
validation and Reader execution together. A compile timeout remains a failed
attempt with a compilation record and no claimed Reader result.

Reports deduplicate shared compilation records, separate Reader and compile
times, and record original EPUB bytes, full package bytes and analysis bytes.
Analysis bytes are chapter analyses plus book map/concepts/claims/entities/
checkpoints/recall cards; the full package also retains source and validation
data. Byte counts are not token estimates. `first_result_ms` is the sum of
compilation and Reader completion, not time to a semantically verified answer
or a streaming first token. Comparisons reject different execution modes.

New run manifests give content captures a seven-day retention deadline. After
review, retain only a checked aggregate export outside the run directory and
remove the complete run directory (including packages, requests, answers,
review packets and traces). This is a manual retention policy, not an automatic
deletion job; no existing files are removed by a benchmark command. Restricted
source libraries and human gold have their own separately agreed retention and
rights. Use public original fixtures for long-lived replay, and never enable
these full captures for participant sessions.

`audit-corpus` reports question counts, language/book/family distribution,
regression/capability counts, dimension coverage and reviewed-gold counts. The
optional holdout catalog is checked against declared families and identical EPUB
bytes (or identical controlled source text). Conflicts return nonzero and retain
the audit report. Undeclared translations or rewrites still need provenance
review. This command does not run candidates or establish process isolation;
held-out execution remains disabled even for a valid split.

`repeated_cases` records every planned and completed attempt per question and the
number of successes. `all_passed` remains null until every planned attempt has
a known outcome; there is no best-of-N success selection.
