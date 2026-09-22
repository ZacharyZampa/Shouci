# shouci-mac — menu-bar app (Shouci)

Native menu-bar capture (`AppKit` via `objc2`). Same core crates as the CLI and TUI.

**Ctrl+Opt+V** or left-click 文 opens the popover. Right-click 文 is Settings / Quit.

## Layout

| File | Role |
| --- | --- |
| `src/main.rs` | App, status item 文, click wiring |
| `src/ui.rs` | Popover, rows, settings, save/delete |
| `src/capture.rs` | Row labels (`headline`, `gloss_line`) |
| `src/hotkey.rs` | Carbon `RegisterEventHotKey` |
| `src/settings.rs` | Hotkey + login defaults |
| `src/login.rs` | LaunchAgent for Open at Login |
| `src/debuglog.rs` | `~/Library/Logs/Shouci/debug.log` |

`unsafe` is allowed only in this crate (ObjC/Carbon). It stays in small wrappers; search and save are safe Rust.

## Build

```sh
./package.sh [output]   # default ~/Applications/Shouci.app
```

Also installed by `./scripts/install.sh` from the repo root. Rebind the hotkey in Settings.

```sh
tail -f ~/Library/Logs/Shouci/debug.log
```
