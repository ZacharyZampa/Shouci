# Shouci (收词)

Collect Chinese vocabulary without leaving what you're doing. Press a
shortcut, type English, pinyin, or characters, pick the word, press Return.
Later, study the words in Pleco or Anki.

Free and open source (MIT). Your words stay on your Mac: Shouci has no
account and goes online only to download its dictionary.

Want to help? [CONTRIBUTING.md](CONTRIBUTING.md). How it's built:
[DEV.md](DEV.md) and [macos/README.md](macos/README.md).

## Install (macOS)

You need:

- macOS 14 or later
- **Xcode**, free from the App Store. The Command Line Tools alone are not
  enough. After installing it, run once:
  `sudo xcode-select -s /Applications/Xcode.app/Contents/Developer`
- **Rust**, from [rustup.rs](https://rustup.rs)
- An internet connection the first time, to download the dictionary

Then:

```sh
git clone https://github.com/ZacharyZampa/Shouci.git
cd Shouci
./scripts/install.sh
```

This downloads and builds the dictionary, builds Shouci for Apple silicon and
Intel, installs it to `~/Applications/Shouci.app`, and starts it. Look for
**文** in the menu bar.

Shouci is built on your Mac and signed to run locally, so Gatekeeper doesn't
ask about it. (There is no paid Apple developer account behind it, so a copy
downloaded from elsewhere is blocked the first time: allow it in System
Settings › Privacy & Security › Open Anyway.)

| | |
| --- | --- |
| Update | `git pull && ./scripts/install.sh` |
| See what's installed | `./scripts/install.sh --status` |
| Uninstall | `./scripts/install.sh --uninstall` (your words stay; see below) |

**Coming from the proof of concept?** On first launch Shouci copies your
words from `~/Library/Application Support/pleco-companion/` into its new
library. The old folder is left as it was.

## Using Shouci

### Quick search

Press **Control-Option-V** anywhere (or click 文). The app you were in stays
in front; Shouci gets out of the way when you're done.

- Type **English** (`to travel`), **pinyin** (`lvxing`, `lǚ xíng`, `lv3xing2`),
  or **characters** (`旅行`). Shouci works out which; the chip beside the
  field shows how it read your search, and its menu changes it.
- **↑ ↓** choose, **Return** saves, **⌘Z** takes the save back, **Esc**
  closes. **⌘O** opens your library.
- Words you already have are marked **Saved**.
- No dictionary entry? Save the characters as **needs review** and fill in
  the reading and meaning later. Shouci also shows the dictionary words found
  inside what you typed (`蚌埠住了` → 蚌埠, 住, 了).

### Your library

Open it with **⌘O** in quick search, right-click 文 › Open Shouci, or open
Shouci from Spotlight. While it's open, Shouci has a Dock icon and menus.

- Browse **All Vocabulary**, **Needs Review**, **Recently Added**,
  **Archived**, and the **Trash**, or a collection or tag.
- Search the toolbar field to find saved words and **add** new ones from the
  dictionary.
- Select a word to see your entry next to the dictionary's; **Edit** (or
  Return, or double-click) changes it. **Add Word** (⌘N) adds one by hand.
- Select several words (⌘-click, ⇧-click) to tag them, add them to a
  collection, mark them for review, archive them, export them, or move them
  to the Trash.

### Pleco and Anki

Shouci moves words in and out as plain-text files.

- **Import** (⇧⌘I): export from Pleco (Import/Export › Export Cards) or Anki
  (File › Export › Notes in Plain Text), then choose the file. Shouci detects
  the format and shows what each line would do before anything changes. For
  words you already have, choose Skip, Merge, or Overwrite.
- **Export** (⇧⌘E): choose Pleco or Anki and which words (new since the last
  export, everything, the selection, or the current view). The file goes to
  Downloads unless you choose another place. Then import it in Anki (File ›
  Import) or Pleco (Import/Export › Import Cards).

Shouci never writes over a file it imported from.

### Settings

Right-click 文 › Settings, or ⌘, in Shouci.

- **General:** the shortcut, showing 文 in the menu bar, what quick search
  does after a save, tone marks or tone numbers, traditional characters in
  lists, and opening at login.
- **Dictionaries:** which dictionaries search uses, and checking for a new
  one.
- **Data:** where your words are kept, import and export, and bringing words
  over from an older library.

## Where your data is

| What | Where |
| --- | --- |
| Your words | `~/Library/Application Support/Shouci/user.db` |
| The dictionary | `~/Library/Application Support/Shouci/dictionaries/` |
| Open at login | `~/Library/LaunchAgents/com.zacharyzampa.shouci.plist` (only if turned on) |

Uninstalling removes the app and the login item; delete
`~/Library/Application Support/Shouci` to remove your words too. Shouci logs
errors to the system log and never logs what you search.

## The dictionary

[CC-CEDICT](https://www.mdbg.net/chinese/dictionary?page=cc-cedict)
(CC BY-SA 4.0), with OpenSubtitles word frequency and HSK 3.0 levels. Shouci
checks for a new version once a month; if the download fails, it keeps the
copy it has.

Results come in a fixed order, so the same search always gives the same list:
how well the word matches (an exact meaning first, then a meaning that starts
with your words, then the rest), then how early that meaning comes in the
entry, then how common the word is.

To rebuild it from scratch:

```sh
cargo run --release -p vocab-dictionary --example ensure -- --force \
  ~/Library/Application\ Support/Shouci/dictionaries/cc-cedict.db
```

## CLI and TUI

The command-line tool (`shouci`) and the terminal UI (`shouci-tui`) are
moving onto the new core and are not built from this branch yet. The proof-of-
concept versions are on `main`.

<img width="1053" height="866" alt="TUI Saved List" src="https://github.com/user-attachments/assets/7092da99-9401-4afc-a540-722a333b5952" />

## Build and test

```sh
./scripts/check.sh                       # Rust: format, lint, all tests
macos/scripts/build-core.sh              # the Rust core for the Mac app
open macos/Shouci.xcodeproj              # the Mac app in Xcode
swift test --package-path macos/ShouciCore
```

More in [CONTRIBUTING.md](CONTRIBUTING.md) and
[macos/README.md](macos/README.md).
