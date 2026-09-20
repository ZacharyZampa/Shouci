# Pleco Mac Companion

Keyboard-first macOS tools for Chinese vocabulary next to Pleco. Local-only, no AI.
Architecture and contributor notes: [DEV.md](DEV.md).

Two databases: read-only `dictionary.db`, writable `user.db`.

## Install (macOS)

Needs a Rust toolchain and network on first dictionary fetch. VocabBar is
unsigned — if Gatekeeper blocks it, right-click → Open.

```sh
./scripts/install.sh              # fetches CC-CEDICT + frequency + HSK, then CLI/TUI/VocabBar
./scripts/install.sh --status
./scripts/install.sh --uninstall
```

Binaries go to `~/.local/bin` (symlinked on `~/.cargo/bin`). Dictionary and
`user.db` live in `~/Library/Application Support/pleco-companion/`.

| App | How |
| --- | --- |
| VocabBar | **Ctrl+Opt+V** or left-click 文. Right-click 文 for Settings / Quit |
| TUI | `vocab-tui` |
| CLI | `vocab search` / `add` / `list` / `import` / `export` |

## Daily use

**VocabBar** — type English, pinyin, or Chinese; live hits; **Enter** saves,
**Esc** dismisses. Right-click a row to save, mark needs-review, archive, or
delete. Settings: rebind the hotkey, Open at Login.

**TUI** — `F1` saved list, `F2` search (`Shift+Tab` toggles). Search: type to
search, `Tab` cycles pinyin/english/chinese, **Enter** saves, **Esc** clears
(quits when empty), `PgUp`/`PgDn` scroll detail. Saved: `Tab` filters status,
`Ctrl+R` reloads. `Ctrl+Q` quits.

**CLI**

```sh
vocab search auto jingzi
vocab search english school --format json --limit 5
vocab search pinyin "lǚ xíng"
vocab add 学校 --mode chinese
vocab list --status needs_review
vocab import fixtures/pleco/v1/valid/categories.txt
vocab export /tmp/pleco-export.txt
```

`vocab add`: one strong match → save confirmed; none → save `needs_review`;
several → save nothing and list candidates.

## Dictionary

If `dictionary.db` is missing, the apps download CC-CEDICT, OpenSubtitles
frequency, and HSK 3.0 on first launch (needs network). After that they
refresh when the calendar month changes; a failed refresh keeps the current
file. Same first fetch runs from `./scripts/install.sh`. CC-CEDICT is
CC BY-SA 4.0 ([MDBG](https://www.mdbg.net/chinese/dictionary?page=cc-cedict)).
Force a rebuild with
`cargo run -p vocab-dictionary --example ensure -- --force`.

Search ranks match quality first, then frequency, then HSK, then entry id.
Same query always yields the same order.

Dictionary path if `--dictionary` is omitted: `VOCAB_DICTIONARY` → repo
`data/dictionary/dictionary.db` (if present) → app-data `dictionary.db`.

## Build & test

After clone:

```sh
./scripts/setup-hooks.sh          # pre-commit = fmt + clippy + all tests
./scripts/check.sh                # fetches the dictionary if missing, then the gate
```

Tests include CLI process I/O plus TUI/menu-bar search against
`fixtures/search/top1000.tsv` (50-word smoke + 1000-word pinyin/hanzi sweep).
GitHub Actions runs `scripts/check.sh` on macOS. Do not weaken fixtures to
make a change pass.

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Workspace

| Crate | Role |
| --- | --- |
| `vocab-core` | Types, statuses, errors |
| `vocab-dictionary` | Ingest + SQLite dictionary |
| `vocab-search` | Ranked search |
| `vocab-pinyin` | Normalize / segment pinyin |
| `vocab-pleco` | Pleco UTF-8 text codec |
| `vocab-db` | `user.db` |
| `vocab-capture` | Shared search / save / CLI add |
| `vocab-tui` | Terminal UI |
| `vocab-mac` | Menu-bar app (VocabBar) |
| `vocab-cli` | `vocab` |

## Principles

1. Retrieval is not translation — English returns candidates; the user picks.
2. No silent uncertainty — ambiguity and missing data become `needs_review`.
3. Deterministic — same DB + query ⇒ same order.
4. Provenance is kept on every saved item.
5. Pleco is file-based UTF-8 text, never `.pqb`.

VocabBar log: `tail -f ~/Library/Logs/VocabBar/debug.log`.
