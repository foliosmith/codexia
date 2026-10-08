# Beta product boundaries

The product is the compiled Book Package and source-linked Reader Cards.
The initial scope is Chinese and English nonfiction, with analysis and contextual
reading tasks grounded in one book at a time. The reference Reader is the local
validation client; general-purpose chat and additional production clients are
outside this scope. An existing multi-book API does not imply cross-book analysis
or acceptance of an externally hosted product.
The reference Reader opens on book status, progress and next reading actions.
Its reading view defaults to chapter context; chat is a secondary tab. Book Map,
chapter checkpoints, reflection and export remain the primary workflow.
`web/index.html` renders API data and sends actions through HTTP. Model invocation,
context construction and offline answer selection remain in `src/web_runtime.rs`.

The package has seven core semantic objects: Book IR, Chapter Analysis,
Book Map, Concept, Claim, Entity and Checkpoint. Manifest, structure, block streams,
compile status and evaluation reports are storage/operational representations;
recall cards are derived from checkpoints. The existing package has more than
eight files. This is an accepted file-layout deviation from the early 6–8 object
target, not a reason to add new domain objects or break existing packages.
Reader Card is a separate presentation contract with eleven variants, including
answer and reflection. Its published JSON Schema and SDK types describe that set.

The local Reader uses the HTTP surface provided by `codexia serve`: `/v1/bootstrap`
for initial book/session data, book/chapter endpoints for content and analysis,
and session/action/export endpoints for interaction. These are the reference
client's local HTTP contract; the authenticated multi-book service is
`codexia api`. The Reader does not call a model directly or implement its own
Compiler. Bootstrap and local state routes must not be mistaken for proof that
the same UI can already be hosted directly on the authenticated multi-book API.
Multi-book session and export IDs include their book ID. Duplicate legacy IDs
return `409 ambiguous_resource` before dispatch; callers must use an unambiguous
resource instead of relying on arbitrary book selection.

Quality acceptance requires actual online compilation and nonempty grounded
outputs. The offline registration analyzer is only parsing and transport evidence.
SourceRef validity, schema validity and spoiler boundaries are hard requirements;
manual Golden Book annotations supply a separate semantic review baseline.
Source validity alone does not prove that an interpretation is correct.
Book Map, package and whole-book indexes expose whole-book information and can
reveal endings. Read-range spoiler acceptance applies to contextual explain,
ask, checkpoint and reflect requests carrying ReaderState.
Reading chapters use the logical-section overlay when ordered TOC anchors resolve
uniquely at retained block boundaries. Format `0.2` supports chapters split within
a spine item or spanning files, while preserving physical block IDs, fingerprints
and source references. Unusable TOCs fall back to physical spine units and the
Reader discloses that basis; a physical unit may contain several literary chapters.
Semantic review must verify the actual reading layout and keep prefatory material
separate. See [logical chapters and compatibility](reader-integration.md#logical-chapters-and-package-compatibility).

Contextual retrieval filters persisted reading coverage and clips partial blocks
before lexical ranking, including Chinese character bigrams. It can miss synonyms;
no matching evidence and evidence exceeding the input budget are distinct outcomes.
Reader adapter protocol `0.2` supplies exact `evidence_refs` for model citations;
valid addresses still do not prove that a claim is supported. Chapter analysis
includes bounded prior-source excerpts. These mechanisms already exist and need
regression and semantic evaluation, rather than replacement by default.

Cost acceptance requires measured provider usage, verified Profile/Lazy Analysis/
source-hash caching behavior, and an explicit cost-accounting basis. An API token
price estimate is not a ChatGPT subscription invoice. Beta gates must preserve
that distinction and must fail when their required evidence is absent.

Local runtime safeguards already include loopback restrictions for unauthenticated
listeners, HTTP and analyzer input/output limits, execution deadlines, concurrency
limits, cancellation, validated chapter reuse and candidate package publication.
See [local operation and limits](reader-integration.md#local-trial-operation-and-limits)
for the settings and recovery contract. Their presence does not establish sustained
load, disk-failure, orphan-process or backup-restore acceptance. External deployment
also requires its own authentication/TLS, webhook delivery and operational review.
The reference shells remain reference implementations until real-client acceptance.

## Compatibility and candidate experiments

Existing format `0.1` packages remain readable; format `0.2` requires a valid logical
partition. Layout changes reject incompatible Reader state explicitly and preserve
the old state for recovery; there is no implicit annotation/progress migration.
Analysis identity changes invalidate reuse, and failed upgrade candidates must not
replace the active validated package. Recovery/reuse is not proof of dependency-aware
incremental analysis or automatic range extension.

Internal experiments must preserve the published Package, SourceRef, Reader Card,
HTTP and SDK contracts. A necessary incompatible change requires an explicit version,
compatibility handling and package/state rollback before adoption. Adapter prompt
protocol versions are separate from stored package and public client schema versions.

The existing `basic`, `standard` and `deep` profiles control analysis depth. Book-type
profiles or an AnalysisPlan, BookState/Observation and hierarchical synthesis remain
candidate experiments, not approved replacements. Current book synthesis sends all
selected chapter analyses in one request. Measure its quality, context and failure
limits before introducing hierarchy, and adopt a candidate only after a comparable
evaluation shows a benefit without weakening source, scope or recovery guarantees.

Offline engineering reports, reviewed online semantic results and consenting-reader
observations are separate evidence. Keep the existing [quality gates](quality-gates.md);
benchmark infrastructure or a completed capability checklist cannot substitute for
any missing acceptance evidence.
