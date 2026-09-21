# Contributing to Shouci

You do not need to be a expert on Chinese to help.
I want to make an app that works for us all will be open to any kind of support!

## How to start

1. [Open an issue](https://github.com/ZacharyZampa/Shouci/issues) for anything
   non-trivial (ranking miss, UX change, new source). Could help prevent
   duplicate work!
2. Fork, branch from `main`, keep the change focused.

```sh
./scripts/setup-hooks.sh    # pre-commit runs the same gate as CI to help us keep things working
./scripts/check.sh          # fmt + clippy + all tests (fetches the dict if needed)
```

CI is macOS. VocabBar (`vocab-mac`) will not build on Linux.

## What “good” testing looks like (to me)

- **I/O focused.** Query, file, or other input in, headword, saved item, or other output out.
  Can add a row to `crates/vocab-cli/tests/e2e.rs` or a probe in
  `fixtures/search/top1000.tsv` when search behavior changes.
- **Do not remove a failing test to make the CR pass.** If a test is wrong, then we must fix it. But new or modified functionality should not impact others.
- **`./scripts/check.sh` must pass.** That is fmt, clippy `-D warnings`, and
  `cargo test --workspace`.
- **No `dictionary.db` / `user.db` in git.** If things needing secrets are added, don't commit them.
- **Manual Tests** Optimally tests for all existing functionality covers them, but for functionality you make or change, please manually test them.

Architecture and “where to change ranking vs UI” live in [DEV.md](DEV.md).
User-facing usage is [README.md](README.md).

## Code notes

- Rust 1.85+, workspace `edition = 2024`.
- Try to minimize `unsafe` usage
- Unless good reason, let's keep frontends thin so business logic can be reused.
- Started with a TUI and Mac menu bar as I worked on a fast POC in a few hours. That doesn't mean we are limited to only that platform.

## AI Coding
- Feel free to use AI - I did for this POC as I just wouldn't have time otherwise
- Do not blindly accept AI design decisions, you can let it write the code but you should make sure you understand why it's buidling out new features or infra

## Review

CR Descriptions should have a few sections in them. Explain Why, What, Risks, Testing

Thanks!
