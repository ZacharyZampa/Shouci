# Dictionaries

One read-only SQLite file per dictionary: `<id>.db` (`cc-cedict.db`). Each
describes itself in its `dictionary_metadata` table. Never mixed with the
writable `user.db`.

- `sources/` — gitignored downloads (CC-CEDICT, OpenSubtitles frequency, HSK 3.0).

This folder holds the build the tests and CI use: `./scripts/check.sh`
builds it when missing, or run
`cargo run -p vocab-dictionary --example ensure -- data/dictionaries/cc-cedict.db`.
The app, `shouci`, and `shouci-tui` keep their own copy in
`~/Library/Application Support/Shouci/dictionaries/` (see `DEV.md`, Data).
