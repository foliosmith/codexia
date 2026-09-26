# Beta quality and cost gates

First complete the [online and Reader acceptance](acceptance.md). Then measure
every registered Golden Book and every completed profile through the existing
Studio Eval Pipeline:

```sh
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
node scripts/accept-quality.mjs private/golden-books/catalog.json \
  <candidate-report.json> private/acceptance/candidate-check \
  private/acceptance/baseline/report.json &&
CODEXIA_ONLINE_REPORT=<candidate-report.json> npm --prefix tests run e2e
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
