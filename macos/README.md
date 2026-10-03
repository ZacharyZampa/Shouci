# macos — Shouci for Mac

A menu-bar app over the Rust core. `crates/shouci-ffi` wraps `shouci-core`
for Swift with [UniFFI](https://mozilla.github.io/uniffi-rs/); the app links
it as an XCFramework.

**Ctrl+Opt+V** (rebindable) or a click on 文 opens quick search. **⌘O**
there, right-click 文 › Open Shouci, or opening the app from Finder or
Spotlight opens the library window. Right-click 文 for Settings and Quit.

| Path | Role |
| --- | --- |
| `Shouci.xcodeproj` | The app target (its files are the `Shouci/` folder) |
| `Shouci/` | The app: AppKit for 文, the panel, the windows, and the shortcut; SwiftUI for what's inside |
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
macos/scripts/package.sh                  # install a release build
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
- The window holds the whole library in memory (scopes, counts, and sorting
  are instant) and polls the core's data version, so words saved from quick
  search, `shouci`, or `shouci-tui` show up within two seconds.
- Import reads the file with every format the core has and keeps the one
  that reads it best, so detection is the core's own parsers, not a guess
  in Swift. (It is the rule of the core's `detect_import`, which the CLI and
  TUI call; the app keeps its own loop to show whether the choice was
  clear.) Nothing is written until Import, and export previews (with
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
