# marsad

Rust TUI dashboard and CLI for Claude Code sessions (ratatui, rusqlite with bundled SQLite, resvg).
User-facing reference: `README.md`. Product and terms: `PRODUCT.md`, `CONCEPTS.md`.

## Commands

```sh
cargo test                                    # all tests; they live in-module under #[cfg(test)]
cargo clippy --all-targets                    # must stay warning-free
MARSAD_HOME=/tmp/marsad-dev cargo run         # dashboard against a scratch data dir
MARSAD_HOME=/tmp/marsad-dev cargo run -- <subcommand>
```

CI runs `cargo test --locked` and `cargo build --release --locked` on Linux, macOS and Windows:
commit `Cargo.lock`, and give every `cfg`-gated platform path a fallback.

## Rules

- Never run `cargo fmt`. The code is not rustfmt-clean, and a format pass rewrites every file.
  Match the surrounding style by hand.
- Anything that writes data during development uses `MARSAD_HOME=<scratch dir>`. Never write to
  `~/.claude/marsad`, and don't run `marsad install`/`uninstall`, which edit
  `~/.claude/settings.json`.
- Tests never touch the clipboard, browser, file manager, network or `claude -p`. Those side effects
  go through injectable functions (`share_view::Env`, `share::Desktop::for_tests()`, `run`
  closures); add new ones the same way.
- Slow work (sync, drafting, image rendering, launching tools) runs on a thread and reports back
  through a channel read in `tick()`. Clipboard copies stay on the main thread, because OSC 52
  writes to the terminal.
- To check a TUI layout, render into `ratatui::backend::TestBackend` at wide (about 182×37) and
  narrow (about 110×33) sizes, and delete the throwaway test afterwards.
- Before changing hooks, rules injection, SVG text or store dedupe, read the matching note in
  `docs/solutions/`.

## Every user-visible change also updates

- `README.md`: the key list near the top and the relevant section.
- `CHANGELOG.md` under `## Unreleased`: one short line per bullet, shown by `marsad update`.
- `assets/SKILL.md`, when a CLI command or its output changes.
- The footer key hints in `ui.rs` and `ShareView::keys()`.

## Git

- Conventional commits (`feat:`, `fix:`, `chore(release):`). Commit with explicit paths:
  `git commit -- <files>`.
- PRs target `main`. Run `gh pr merge` only when asked, as a standalone command, without
  `--delete-branch`.
- To release: bump `Cargo.toml`, rename `## Unreleased` to `## X.Y.Z - YYYY-MM-DD`, commit
  `chore(release): X.Y.Z`, then push tag `vX.Y.Z`. `dist` builds the binaries.

## Output hygiene

Code, comments, docs, commits and PRs describe the work, never the request or the process that
produced it. The full rules are in `assets/OUTPUT-HYGIENE.md`.
