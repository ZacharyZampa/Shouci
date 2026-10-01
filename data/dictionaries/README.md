# Dictionaries

One read-only SQLite file per dictionary: `<id>.db` (`cc-cedict.db`). Each
describes itself in its `dictionary_metadata` table. Never mixed with the
writable `user.db`.

- `sources/` — gitignored downloads (CC-CEDICT, OpenSubtitles frequency, HSK 3.0).
- `sources.manifest.example.json` — example ingest manifest (not loaded at runtime).

Built on first launch, or with
`cargo run -p vocab-dictionary --example ensure -- data/dictionaries/cc-cedict.db`.
