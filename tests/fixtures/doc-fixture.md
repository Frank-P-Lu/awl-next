# Doc fixture

A committed, test-owned markdown fixture. The corpus/ranking/index unit tests
in `src/fuzzy.rs`, `src/index.rs`, and `src/main/run.rs` reference this file's
basename (`doc-fixture.md`) as their sample "a markdown file at the project
root" instead of the real repo `README.md`, so that `README.md` — a genuine
top-level doc — is free to move without touching test data.

The fixture is deliberately tiny; the tests use only its NAME, never its bytes.
Its existence is asserted by the link law (`src/embedded_docs_law.rs`).
