# Beta quality and cost gates

First complete the [online and Reader acceptance](acceptance.md). Then measure
every registered Golden Book and every completed profile through the existing
Studio Eval Pipeline:

```sh
CODEXIA_SEMANTIC_REVIEW=<artifact-bound-review.json> \
node scripts/accept-quality.mjs private/golden-books/catalog.json \
  private/acceptance/online/report.json private/acceptance/baseline
```

The command records parse quality, citation validity, chapter coverage, claim
grounding and concept grounding. Books with only registration packages are
explicitly marked `offline-registration-only` and retain sample counts; empty
outputs cannot establish online analysis quality. Online packages must be complete
for their recorded source and profile, have nonempty concepts/claims/citations,
pass `codexia validate`, and have a full set of provider-generation records.

Acceptance requires three distinct standard books, a deep run of one of those
books, and a basic run. Duplicate book/profile rows and missing usage are rejected.
SourceRef, schema and spoiler errors are not waivable quality warnings. The
cited-block coverage warning is retained separately: it is a proxy, not a semantic
summary score. Review generated questions, theses, concepts and selected chapter
summaries against the manual seeds as a separate step.
SourceRef validation establishes block identity and in-bounds Unicode-scalar
offsets; evidence text remains model-produced prose, including paraphrases.

The first successful report becomes a provisional baseline. It permits no decrease
in the five measured quality metrics, and at most 25% above each observed profile's
time and API-equivalent cost. These are single-run limits, not p95 latency claims.
Source/prompt/model changes require a fresh generation report, not a cache-hit
timing presented as a cold run. Keep prior reports and investigate failures before
replacing a baseline.

Check a candidate against that frozen baseline:

```sh
CODEXIA_SEMANTIC_REVIEW=<artifact-bound-review.json> \
node scripts/accept-quality.mjs private/golden-books/catalog.json \
  <candidate-report.json> private/acceptance/candidate-check \
  private/acceptance/baseline/report.json &&
CODEXIA_ONLINE_REPORT=<candidate-report.json> \
CODEXIA_SEMANTIC_REVIEW=<artifact-bound-review.json> npm --prefix tests run e2e
```

The gate returns nonzero and writes `passed: false` when evidence, quality, cost or
timing fails. Beta release preparation must stop on that result. This repository
does not yet have a deployment/release workflow; this command is its runnable
acceptance gate, not evidence that an external release service enforces it.

Costs use actual reported Astra input/cache/output token counts and the official
2026-09-26 [Standard API price snapshot](https://developers.openai.com/api/docs/pricing). They are API-equivalent estimates, not a
ChatGPT subscription invoice. Accepted-package costs attribute replayed usage to
the original generation. When supplied, the separate deduplicated experiment
ledger also includes failed/exploratory calls with known usage and lists attempts
whose usage is unavailable. Neither number should be described as a reconciled
account bill. The current Beta accounting basis is token usage plus this explicit
estimate; invoice reconciliation remains unverified.

## Evaluation 0.2

Studio metrics now return `{state, value_basis_points, reason}`. States are
`evaluated`, `not_evaluated`, `not_applicable`, and `missing_required`. Only
`evaluated` has a numeric score. Old numeric Studio history is displayed as
unassessed legacy evidence and must be reevaluated; frozen numeric online
baselines remain comparable to newly evaluated scores without rewriting them.

By default all profiles require chapters, citations, concepts and claims. For a
book/task that intentionally omits claims or concepts, POST `/v1/studio/evals`
with `not_applicable: {claim_grounding: "reviewed reason"}` (or
`concept_grounding`). Existing output must still be evaluated; citations and
parsing cannot be waived. The online Beta corpus continues to require nonempty
claims and concepts for every accepted profile.

`structural_valid` reports the source-reference checks; `valid` additionally
requires the configured outputs. `beta_ready` also requires a complete passing
semantic sample review. None of these structural percentages measures semantic
accuracy. Studio's button runs structural evaluation only.

Register the reference annotations through `/v1/studio/golden-books`, then POST
`{golden_id}` to `/v1/studio/evals`. Record the returned `analysis_fingerprint` and
`annotations_fingerprint`. A subsequent request may include `semantic_review`:

```json
{
  "golden_id": "returned-golden-id",
  "semantic_review": {
    "analysis_fingerprint": "returned-analysis-fingerprint",
    "annotations_fingerprint": "returned-annotations-fingerprint",
    "reviewer": "reviewer and date",
    "checks": [
      {"dimension":"citation_support","expected":"reference expectation","observed":"inspected passage and conclusion","passed":true},
      {"dimension":"key_point_coverage","expected":"manual key points","observed":"inspected coverage and scope","passed":true},
      {"dimension":"spoiler_boundary","expected":"excluded later facts","observed":"inspected bounded answer or checkpoint","passed":true}
    ]
  }
}
```

These are attributed review judgments, not an automated semantic oracle. Include
sample locations and limitations in `observed`; do not claim whole-book coverage
from samples. A changed analysis or changed annotation invalidates the review.
A legitimate citation that contradicts the answer, missing key points, or later
facts in a bounded answer must be recorded as failed checks. Input scope alone
cannot suppress knowledge already present in the model.

Supply the reviewed samples to the existing quality gate with
`CODEXIA_SEMANTIC_REVIEW=<review.json>`. Its file shape is
`{"reviews":[{"book_id":"...","profile":"standard","review":{...}}]}`;
`review` is the `semantic_review` object above. Every online book/profile requires
its own matching review. Missing, stale, or failed reviews block the gate;
registration-only books remain unassessed and cannot establish a Beta baseline.
This requirement applies to both initial baselines and frozen-baseline checks.
