# Beta product boundaries

The product is the compiled Book Package and source-linked Reader Cards.
The reference Reader opens on book status, progress and next reading actions.
Its reading view defaults to chapter context; chat is a secondary tab. Book Map,
chapter checkpoints, reflection and export remain the primary workflow.
`web/index.html` renders API data and sends actions through HTTP. Model invocation,
context construction and offline answer selection remain in `src/web_runtime.rs`.

The v0.1 package has seven core semantic objects: Book IR, Chapter Analysis,
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
Chapter IDs currently identify EPUB spine units. A unit can contain several
literary chapters or mixed prefatory material. Coverage counts those units;
semantic review must also check the manual literary-chapter references and keep
preface commentary separate from the author's narrative.

Cost acceptance requires measured provider usage, verified Profile/Lazy Analysis/
source-hash caching behavior, and an explicit cost-accounting basis. An API token
price estimate is not a ChatGPT subscription invoice. Beta gates must preserve
that distinction and must fail when their required evidence is absent.

Passing phase five does not assert production readiness. Upload/resource limits,
webhook signing/retries, orphan recovery, concurrency, shared contract tests,
deployment and backup recovery remain phase-six work. The reference shells remain
reference implementations until phase seven's real application acceptance.
