# Codexia benchmarks

Run from any directory with Node.js 22 or later:

```sh
node benchmarks/runner.mjs validate
node benchmarks/runner.mjs validate --catalog <local-catalog.json>
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
