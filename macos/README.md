# macos — Shouci for Mac

A menu-bar app over the Rust core. `crates/shouci-ffi` wraps `shouci-core`
for Swift with [UniFFI](https://mozilla.github.io/uniffi-rs/); the app links
it as an XCFramework.

**Ctrl+Opt+V** (rebindable) or a click on 文 opens quick search. **⌘O**
there, right-click 文 › Open Shouci, or opening the app from Finder or
Spotlight opens the library window. Right-click 文 for Settings and Quit.

| Path | Role |
| --- | --- |
| `Shouci.xcodeproj` | The app target (its files are the `Shouci/` folder) and `ShouciTests` |
| `Shouci/` | The app: AppKit for 文, the panel, the windows, and the shortcut; SwiftUI for what's inside |
| `ShouciTests/` | The app's logic, tested against a scratch library: the target compiles `Shouci/` itself, so nothing launches |
| `ShouciCore/` | Swift package: the core as Swift (`ShouciCore.swift` is generated) and its tests |
| `scripts/build-core.sh` | Builds the Rust core into `ShouciCore/ShouciFFI.xcframework` and regenerates the bindings |
| `scripts/package.sh` | Release build, installed to `~/Applications/Shouci.app` |
| `AppIcon.png` | The 文 icon master; `Shouci/Assets.xcassets` holds its sizes (`sips -z`) |

Needs Xcode (the Command Line Tools alone lack SwiftUI's macros). No Apple
Developer account: the app is signed to run locally and not notarized.

## Build

```sh
macos/scripts/build-core.sh               # once, and after Rust changes
open macos/Shouci.xcodeproj               # or:
xcodebuild -project macos/Shouci.xcodeproj -scheme Shouci build
swift test --package-path macos/ShouciCore
xcodebuild -project macos/Shouci.xcodeproj -scheme Shouci -destination 'platform=macOS' test
macos/scripts/package.sh                  # install a release build
macos/scripts/make-dmg.sh                 # a release build as dist/Shouci-<version>.dmg
```

The app build stops with a message when the Rust core is missing or older
than its sources. Generated files are not committed.

Only one Shouci runs at a time: a second copy quits. To try a development
build while the installed app is running, give it another bundle id:
`xcodebuild … PRODUCT_BUNDLE_IDENTIFIER=com.zacharyzampa.shouci.dev build`.

## How it works

- The panel is a non-activating `NSPanel`: it takes typing while the app you
  were in stays active, and hides when it loses the keyboard.
- The search field is an `NSTextField`, so arrows, Return, and Esc reach the
  panel only after the input method passes on them; text still being
  composed is not searched.
- The library window is an AppKit window around a SwiftUI
  `NavigationSplitView`. While it is open Shouci has a Dock icon and menus;
  closed, it is back to the menu bar only.
- The window holds the whole library in memory and polls the core's data
  version, so words saved from quick search, `shouci`, or `shouci-tui` show
  up within two seconds. The current view's list and the sidebar's counts
  are worked out when the library, the view, or the sort changes
  (`LibraryModel.refreshList`), never while drawing: a click redraws
  several times, and grouping 15,000 words takes milliseconds, not nothing.
- The filter panel and smart collections (`SmartCollections.swift`,
  `FilterPanel.swift`) ask the core which words match (`matching_ids`)
  rather than deciding in Swift: only the core knows HSK levels and
  frequency. Each change to the filter is one call off the main thread; the
  list shows the view's words that are among the ids that come back. A
  reload asks again, along with every smart collection's words. The chips'
  wording, and whether two filters ask for the same words, come from the
  core too (`filter_conditions`, `same_conditions`), so `shouci smart` says
  the same thing, as do the HSK choices (`hsk_levels`) and each word's
  frequency band (`ItemView.frequency_band`).
- ⌘Z (`LibraryUndo.swift`) asks the core for the words a change touches
  before and after it, with every name and smart collection (`snapshot`),
  and undo and redo put one or the other back (`restore`), which the core
  refuses when a word or smart collection changed since. Adding a word and
  quick search's undo work the same way: a word saved new is missing from
  the first snapshot, so undoing deletes it and redoing puts it back. A
  typed word is first looked up (`manual_match`), so one brought back from
  the Trash goes back there. The
  actions go to the library window's own undo manager, so the Edit menu
  names them.
- A problem shows in the editor or filing sheet while one is open
  (`Notice`), and in the window's alert otherwise: the alert waits until no
  sheet is open.
- Import reads the file with the core's `detect_import`, the same call as
  the CLI and TUI: every format the core has, keeping the one that reads it
  best, so detection is the core's own parsers, not a guess in Swift. It
  also says whether the choice was clear, which is when the sheet shows
  Detected. Nothing is written until Import, and export previews (with
  counts) before writing.
- Settings live in `UserDefaults` under the menu-bar app's old keys
  (`ShouciHotkey*`), and open-at-login is the same user LaunchAgent
  (`com.zacharyzampa.shouci`), so both carry over.
- Logs go to the unified log (`log stream --predicate 'subsystem ==
  "com.zacharyzampa.shouci"'`). Searches are never logged.

## Rules

- Every call into `Core` can block: go through `AppModel.call`, which runs
  it off the main thread.
- Errors arrive as `ShouciError`; show `describe(error)` to people.
- New core API: add it to `Shouci`, then one method in
  `crates/shouci-ffi/src/lib.rs`. New core types: restate them in
  `crates/shouci-ffi/src/remote.rs` (the compiler checks they match).
