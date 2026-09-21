# Shouci (收词)

Keyboard-first macOS tools for collecting Chinese vocabulary.

Fully free and open source. With a promise of no user info collection.

Want to help? [CONTRIBUTING.md](CONTRIBUTING.md). Architecture: [DEV.md](DEV.md).

Two databases: read-only `dictionary.db`, writable `user.db`.

## Screenshots

### Menu Bar
<img width="404" height="390" alt="Mac Menu Bar" src="https://github.com/user-attachments/assets/99d94816-abf5-4eda-96c9-aa80fdaac3dc" />


### TUI
<img width="1053" height="866" alt="TUI Saved List" src="https://github.com/user-attachments/assets/7092da99-9401-4afc-a540-722a333b5952" />
<img width="1063" height="867" alt="TUI Search" src="https://github.com/user-attachments/assets/fa5752dd-e011-4687-b0dc-afbdd452afcc" />



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

Dictionary path if `--dictionary` is omitted: `VOCAB_DICTIONARY` →
`data/dictionary/dictionary.db` (current directory or parents) → app-data
`dictionary.db`.

## Build & test

See [CONTRIBUTING.md](CONTRIBUTING.md) for issues, PRs, and test rules.

