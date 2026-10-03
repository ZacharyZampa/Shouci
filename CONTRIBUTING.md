# Contributing to Shouci

You do not need to be an expert on Chinese to help.
I want to make an app that works for us all and will be open to any kind of support!

## How to start

1. [Open an issue](https://github.com/ZacharyZampa/Shouci/issues) for anything
   non-trivial (ranking miss, UX change, new source, new file format). Could
   help prevent duplicate work!
2. Fork, branch from `main`, keep the change focused.

```sh
./scripts/setup-hooks.sh    # pre-commit runs the same gate as CI to help us keep things working
./scripts/check.sh          # boundaries + fmt + clippy + all tests (fetches the dict if needed)
```

For the Mac app you also need Xcode (free; the Command Line Tools alone are
not enough):

```sh
macos/scripts/build-core.sh                  # after any Rust change
swift test --package-path macos/ShouciCore
open macos/Shouci.xcodeproj
```

CI is macOS, and the app needs macOS 14 or later. Nothing is tested on other
systems yet.

Do not commit dictionary builds (`data/dictionaries/*.db`), `user.db`, or
anything under `data/dictionaries/sources/`. If things needing secrets are
added, don't commit them.

## Before you open a PR

`shouci --help` and `shouci-tui` are the fastest way to see what a change
actually does, against your own library or a scratch one
(`SHOUCI_HOME=/tmp/shouci-try shouci …`). `./scripts/check.sh` must pass.

The user-facing docs are part of the change. If you add a keybinding, a
command, a flag, a file format, or a data path, update `README.md` (what
people use), `DEV.md` (how it fits together), and `macos/README.md` for the
app, in the same PR. `README.md` documents the `shouci` binary, not `vocab`:
that was an early name and it is gone.

## What “good” testing looks like (to me)

- **I/O focused.** Query, file, or other input in, headword, saved word, or
  other output out. Add a use case to `crates/shouci-core/tests/use_cases.rs`,
  a flow to `crates/shouci-cli/tests/e2e.rs`, or a probe in
  `fixtures/search/top1000.tsv` when search behavior changes.
- **Do not remove a failing test to make the CR pass.** If a test is wrong, then we must fix it. But new or modified functionality should not impact others.
- **`./scripts/check.sh` must pass.** That is the crate boundaries, fmt,
  clippy `-D warnings`, and `cargo test --workspace`.
- **Fixtures are the contract.** `fixtures/pleco/README.md` pins the Pleco
  grammar, `fixtures/pinyin/NORMALIZATION.md` pins `normalize`, and the
  `golden.rs` tests pin both file formats. Changing a format means changing
  those deliberately, not as a side effect.
- **Manual tests.** Optimally tests for all existing functionality cover them, but for functionality you make or change, please manually test them.

Architecture and “where to change ranking vs UI” live in [DEV.md](DEV.md).
User-facing usage is [README.md](README.md).

## Code notes

- Rust 1.85+ on stable, workspace `edition = 2024`.
- No `unsafe`: the workspace forbids it.
- Rules live in `shouci-core`; the Mac app, the CLI, and the TUI only show
  what it returns. `scripts/check-deps.sh` keeps frontends from reaching past
  it, so business logic is written once and reused.
- The proof of concept started as a TUI and a menu-bar app built in a few
  hours. Now a Mac app, a CLI, and a TUI share one core, and that doesn't
  mean we are limited to those: a new platform is a new frontend over
  `shouci-core`.

## AI Coding
- Feel free to use AI - I did for this project as I just wouldn't have time otherwise
- Do not blindly accept AI design decisions, you can let it write the code but you should make sure you understand why it's building out new features or infra

## Review

CR Descriptions should have a few sections in them. Explain Why, What, Risks, Testing

Thanks!
