# Fixtures

Contract for tests. Paths are `env!("CARGO_MANIFEST_DIR")/../../fixtures/...`.

- `pleco/` — Pleco UTF-8 text, valid and malformed. Grammar pinned in
  [pleco/README.md](pleco/README.md).
- `pinyin/NORMALIZATION.md` — `normalize()` input/output matrix, one row per
  test.
- `dictionary/` — small CEDICT, frequency, and HSK samples.
  `cedict-sample.u8` is also the dictionary in `shouci_core::testing`
  sandboxes, which the core, CLI, TUI, and FFI tests use.
- `search/top1000.tsv` — HSK probes for the core's search-quality tests
  (`crates/shouci-core/tests/search_quality.rs`). Regenerate with
  `python3 search/generate.py`.

Anki round trips are pinned by `crates/vocab-anki/tests/golden.rs` rather
than by files here. Adding a fixture means adding the test that reads it: an
orphaned golden file protects nothing.
