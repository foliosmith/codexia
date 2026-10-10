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
labelled offline registration. Use `cargo build --locked --offline` first. Every attempt
starts a real loopback Reader and a fresh state directory; the default adapter
extracts offered source blocks and never judges semantic correctness.

`--agent-command <executable>` supplies an offline/replay adapter using the existing
JSON stdin/stdout contract. It is trusted executable code, not a sandbox; never
use this option for an unbudgeted online provider. Explicit Reader provider calls
use `--provider-config` below; held-out runs remain disabled. `--repeat 3` records three independent
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

`--candidate A1` builds its own source-only package in either execution mode.
The deterministic `adapters/source-only.mjs` supplies the structural documents
required by the existing Compiler and Reader. Its generic checkpoints have no
compiled answers; summaries, maps and flashcards are explicitly scaffolding,
not semantic analysis or quality evidence. No model is used for compilation.
A1 rejects `--compile-with-provider`, records `compiler: source-only`, and reports
zero compile model tokens/cost while retaining actual local compile time/bytes.
The answering adapter, reading boundary, lexical retrieval and citation checks
are shared with A0. A1 strips chapter analysis, concepts, argument flow and
expected points from answering requests. A0 `ask` already excludes those fields,
so its answering path remains equivalent; do not infer a quality difference
from the candidate name.

Every `reflect` step must declare `question` and `answer`. Both candidates use
that frozen question; the capture adapter replaces the package question and
removes its unrelated expected points before dispatch. This evaluates feedback
on a fixed task, not the quality of generated checkpoint questions. The actual
question is retained per step, in provider requests and in blinded review packets.
Only the current task is written to the attempt's private `task.json`; gold and
other questions are not supplied to the answering adapter. Existing Reader
checkpoint admission, persisted reflection and output validation still run.

New runs declare `reflection_protocol: fixed-question-v1`. Add explicit questions
to a new version of any legacy suite; keep the old suite and runs as historical
evidence. Legacy executions cannot resume under the new protocol or be compared
with new runs. A1 has its own package and never reuses an A0 checkpoint/analysis.

`compare` requires matching inputs, gold, scorer, answering adapter, repeat count,
execution mode, binary/benchmark identity, reflection protocol and judge versions.
The intentional A0 semantic-compiler versus A1 source-only difference is allowed
and recorded on both sides of the comparison; other compiler mismatches fail. It checks artifact hashes, retains all paired
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
not implemented. Both modes use offline registration unless explicitly configured
with `--compile-with-provider` as described below.
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

## Explicit Reader provider configuration

Pass `--provider-config private/benchmarks/provider.json` to `run` and `resume`.
This replaces `--agent-command`, enables the configured model through the existing
JSON stdin/stdout adapter, and freezes the model, adapter hash, limits and pricing.
Reader answering uses this configuration; compilation is offline registration
unless `--compile-with-provider` is also supplied. Automatic calibration judging requires the explicit `judge-calibration` command;
candidate scoring uses `judge` after calibration admission. No online call is
made by default, validation, report generation or corpus auditing.

For a config at that path, this is a template, not usable pricing. Replace all
angle-bracket fields with your chosen model and numeric rate snapshot. Do not put
API keys or credentials in this file; use the adapter's existing authentication.
Unknown fields, including credential fields, are rejected before a run is created.

```json
{
  "evidence": "online",
  "adapter": "../../scripts/online-analyzer.mjs",
  "model": "<explicit-model>",
  "max_invocations": 3,
  "max_request_bytes": 65536,
  "stop_after_input_tokens": 20000,
  "stop_after_output_tokens": 10000,
  "stop_after_estimated_usd": 1,
  "pricing": {
    "source": "<rate-source>",
    "as_of": "<YYYY-MM-DD>",
    "input_per_million": "<numeric-rate>",
    "cached_input_per_million": "<numeric-rate>",
    "output_per_million": "<numeric-rate>"
  }
}
```

Adapter paths are relative to the config file. `evidence: "simulation"` uses the
same accounting path for local fixture adapters and is never reported as online
evidence. The standard online adapter must use this configuration rather than the
unbudgeted replay option. Other executables remain trusted adapter code, not an
OS sandbox or a guarantee against undisclosed provider-side requests.

The invocation count is a hard dispatcher limit across all attempts, reserved
before spawning and never refunded for failed starts. A provider can retry
internally: one invocation is not necessarily one HTTP/model request. Byte limits
apply to the serialized Codexia request, before the adapter adds its own prompt.
The case deadline still bounds the local process; it is not a remote billing cap.

Token and cost values are **stop thresholds on reported usage**, not hard spending
limits. A single call can cross a threshold; the report shows the overshoot and
blocks later dispatch. Unknown, invalid, interrupted or unsupported usage (such as
cache-write usage without a supported price contract) also blocks further calls.
Cached input is priced separately. Failed calls with reported usage are included.
Do not delete or reset the ledger to retry: resume preserves consumed reservations
and rejects changed models, adapters, pricing or limits. An interrupted reservation
can conservatively stop a run even if later logs contain usage.

`budget/ledger.json` records unique invocation identities and compile/answer phases.
Reports distinguish known estimated costs from incomplete totals; offline compile
and unconfigured judge amounts stay null. Rates are operator-supplied API-equivalent
estimates, not an account invoice.
Use provider-enforced limits if an absolute monetary ceiling is required. Existing
private originals and capture retention rules still apply to all online inputs.

`validate --provider-config <file>` validates configuration without dispatching.
Add `--compile-with-provider` to both run and resume to use the configured adapter
for chapter analysis and synthesis as well as Reader answering. Those calls share
one global invocation/token/cost ledger. Native compilation still validates and
publishes the package; changing model, adapter or benchmark revision changes its
analysis cache identity. The configured adapter uses its own default prompt version
rather than an inherited `CODEXIA_ANALYZER_PROMPT_VERSION` override.

In cold mode, the case's `max_calls` includes compilation and answering; for the
three-chapter controlled book a complete run needs four compile calls before the
Reader call. In fixed-package mode, shared compilation consumes the global budget
once and the per-case limit covers Reader calls. No applicable cases means no
compilation. Failed or interrupted compile calls remain in the ledger; cold
attempts retain their compile-call counts even when final artifacts were not saved.
An exhausted budget can leave a partial package and no answers; this is a recorded
failure, not permission to silently raise a limit or fall back to offline answers.

### DeepSeek bounded Compiler and Reader runs

Use `benchmarks/adapters/deepseek.mjs` as the configured adapter and supply
`DEEPSEEK_API_KEY` only through the process environment. Select an explicit
available model and record current official USD rates in the private provider
configuration. Never store the key in that configuration or captured inputs.
The adapter uses the fixed official HTTPS endpoint, rejects redirects, performs
one request without retries, disables thinking and caps every input at 65,536
bytes. Unsupported task names are rejected before dispatch.

| Task | Maximum output tokens | HTTP timeout |
|---|---:|---:|
| Chapter analysis/reanalysis and book synthesis | 8,192 | 90 seconds |
| Reader explanation/question/reflection and semantic Judge | 1,024 | 45 seconds |

These are per-request limits. Set the case deadline and invocation allowance for
the entire workflow, including compilation and Reader startup. A small one-chapter
cold run with one explanation and one reflection needs four invocations: chapter
analysis, synthesis, explanation and reflection. Real work can time out or exceed
the output limit; it must retain a failure, never silently truncate a valid answer.
The 64 KiB input limit can reject longer chapters or larger synthesis requests.

Token usage is retained even for truncated or invalid JSON output; missing usage
stops subsequent dispatch through the shared ledger. Compiler publication and
Reader citation/range validation remain owned by the existing runtime.

Use `--compile-with-provider` with A0 to enable model compilation explicitly;
without it, compilation remains offline registration. A1 stays source-only.
Start with original public fixtures and a small invocation allowance. The
Compiler/Reader path is covered with local simulated HTTP responses; that does
not establish live provider reliability or semantic quality. Reported costs use
the supplied pricing snapshot and are estimates, not invoices or hard spend caps.

### Automatic calibration judgments

```sh
node benchmarks/runner.mjs judge-calibration \
  --provider-config private/benchmarks/judge-provider.json \
  --output private/benchmarks/calibration/new-run
```

Optionally supply `--calibration` with an attributed reviewed reference set.
Each sample is sent separately without reference expectations; the provider sees
only source, question, answer, rubric and output shape. The fresh private run
records frozen judge identity, requests, outputs, a `judge` phase budget ledger,
partial judgments and a report. It never retries or overwrites existing runs.
Invalid output, missing usage, timeout or exhausted allowance stops further
dispatch. Token/money thresholds remain post-usage checks, not hard spending caps.

A completed invocation is not a calibration pass. The existing admission rule
still requires at least 20 independently reviewed references, all judgments, and
exact agreement. Default references are drafts, so simulated or real generated
judgments cannot mark them reviewed.

### Calibrated candidate judgments

```sh
node benchmarks/runner.mjs judge \
  --catalog private/benchmarks/reviewed-suite/catalog.json \
  --left private/benchmarks/runs/candidate-run \
  --provider-config private/benchmarks/judge-provider.json \
  --calibration private/benchmarks/reviewed-calibration.json \
  --calibration-run private/benchmarks/calibration/passed-run \
  --output private/benchmarks/judgments/new-run
node benchmarks/runner.mjs score \
  --catalog private/benchmarks/reviewed-suite/catalog.json \
  --output private/benchmarks/runs/candidate-run \
  --reviews private/benchmarks/judgments/new-run/reviews.json
```

Use the candidate run's `--repeat` value when scoring a repeated run. Before any
provider dispatch, `judge` requires reviewed gold, unchanged execution inputs,
a completed passing calibration with the same frozen provider/benchmark identity,
and re-evaluates the calibration judgments against the exact reference file.
Changing model, adapter, configuration or benchmark code requires recalibration.
Holdout remains unsupported. The reference-set gate does not establish universal
judge correctness or independently prove that a declared human review happened.

Each completed attempt is judged once using the existing blinded, range-clipped
packet. Candidate/model metadata and attempt bindings stay outside the request.
All four semantic dimensions must pass the existing review contract; artifacts,
cases and gold remain hash-bound when `score` imports the result.

The judge run has its own bounded ledger and report; its costs are separate from
the candidate's compile/answer totals. Failed or partial runs retain private
evidence and `partial-reviews.json`, but publish `reviews.json` only after all
completed attempts are assessed with known usage. No automatic retries occur.
Simulation is explicitly attributed in both the report and review records.
