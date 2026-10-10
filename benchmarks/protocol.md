# Benchmark protocol 0.0

A case is one task (possibly several ordered Reader interactions), on one fixed
source version, under a declared reading boundary and budget. Attempts never
increase the number of independent cases or books. Reflection steps freeze both
the question and the user's answer. `fixed-question-v1` replaces a package's
checkpoint prompt and omits its unrelated expected points at provider dispatch
for both candidates; it measures feedback, not generated-question quality.
A1 uses a separately compiled deterministic source-only package with no semantic
Compiler calls. Structural scaffolding is not an analytical result. Its local
compilation time and bytes are measured; its model compilation cost is zero. Reading endpoints are Unicode
scalar offsets; confirming a chapter does not confirm skipped chapters.

Catalog, case, book and gold schemas live in `schemas/`. Original source anchors
include chapter href, exact text and SHA-256. Runtime mapping must uniquely match
the original href and text and must fail on ambiguity or loss. Runtime block IDs
are not the sole gold locator. The controlled fixture format supports text-only
paragraphs. Private catalogs can also bind these independent anchors to a
fixed-hash real EPUB; annotating that source and verifying its rights remain
explicit corpus work.

Execution states and evaluation states are independent. Evaluation uses
`evaluated`, `not_evaluated`, `not_applicable`, `missing_required`. Applicability
is fixed before execution; missing candidate capability is not non-applicability.
Execution failures, empty responses and skipped scoring stay in the denominator.
Unsupported or unreviewed semantics cannot establish a passing quality baseline.

Structural citation checks establish identity and bounds, not semantic support.
Semantic review checks fidelity, necessary conditions, attribution, evidence and
unread disclosure against gold. A correct refusal passes only when allowed source
material cannot answer the question. A retrieval miss is not a successful refusal.
An author's claim is not automatically a verified claim about the world.

Regression cases protect existing promises; capability failures remain visible
without silently replacing release gates. Scores cannot average away hard source,
scope, publication or persistence failures. Record failures as n/N tested, not as
proof of zero population risk. Small corpora support paired exploratory evidence,
not stable p95/SLA or general quality claims.

All run outputs are private, including source excerpts, inputs, answers, browser
traces, provider events and reports. Source permissions and split membership are
independent. Git ignore and changing cwd are not process read isolation. Held-out
execution requires a verified access boundary; the runner must not imply it has
one. Only the current question and allowed source may reach the candidate; gold
and other hidden questions must not. Never put credentials in recorded configs.

Offline stubs and replay prove transport and engineering contracts only. Online
compiler, answer and judge costs must be recorded separately with unknown usage
as null. No automatic online execution or implicit provider budget is permitted.
Participant material follows the existing Reader retention policy; do not enable
full captures for participant sessions. Persistent replay uses public originals.
