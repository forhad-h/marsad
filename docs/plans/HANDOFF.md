Continue the marsad improvement work in /Users/forhad/Projects/marsad (Rust ratatui TUI dashboard for Claude Code sessions). The user asked for 12 improvements, grouped into 4 groups, each run through the compound-engineering flow: plan -> doc review (ce-doc-review mode:non-interactive) -> work (ce-work) -> simplify (ce-simplify-code) -> code review (ce-code-review) -> compound (ce-compound) -> PR. The user authorized the whole flow; don't stop for menu confirmations, but do ask real product questions. Subagents are allowed.

## State now
- Branch `fix/session-status-accuracy` (off `develop`/`main`, both at 8e38da1). Not pushed. Tree clean.
- Group A (status accuracy) is implemented, simplified, code-reviewed (verdict: Ready to merge), and committed: 3bc7b96..4bf6f28. `cargo test` passes 55 tests; `cargo clippy --all-targets -- -D warnings` is clean.
- All 4 requirements docs are committed under docs/plans/:
  - A: 2026-10-23-0657-fix-session-status-accuracy-plan.md (fully planned + reviewed)
  - B: 2026-10-23-0715-feat-token-counter-and-data-plan.md (requirements only)
  - C: 2026-10-23-0716-feat-agent-coordination-plan.md (requirements only)
  - D: 2026-10-23-0717-feat-badges-and-share-plan.md (requirements only)
- Code review run artifacts for A: /tmp/compound-engineering-501/ce-code-review/20261023-073116-9e4f5053

## Next steps, in order
1. Group A follow-ups from the review's residual risks (small; do them before the PR):
   - Add a `stopped` flag on Session (true after a main-agent Stop/SessionStart, false after any other main-agent hook) and skip the interrupt inference in `Session::reconcile` (src/model.rs) when it is set. This closes the ~0.4s race where a refresh lands between Stop and the transcript's turn_duration.
   - After `claude --resume`, background shells from the old process never get a task-notification and pin the row ACTIVE. Record launch time per background shell in src/transcript.rs and the latest SessionStart time on Session; ignore shells launched before it.
   - Test the `idle_prompt` guard in src/events.rs (Stop then idle_prompt keeps IdleCause::Finished).
   - Don't silently ignore `settings::set_alerts` failures on the `m` key in src/app.rs; show them in the footer.
2. Run ce-compound for Group A. The non-obvious learning: Claude Code fires no hook for Esc or interrupts, AskUserQuestion also fires PermissionRequest plus a permission_prompt notification, turn_duration is written about 0.4s after the Stop hook, and background shells are only visible through toolUseResult.backgroundTaskId and task-notifications (user entries, queued_command attachments, queue-operation entries).
3. Open the PR for Group A against `main` with `gh` (squash-merge style, matching history #1-#19). Push with `bash ~/.claude/skills/pr-ship/scripts/gh-git.sh` (the SSH remote hangs). Run `gh pr merge` as its own command, never chained, and without `--delete-branch`. Only merge if the user asks.
4. Group B (token counter, `marsad context` + automatic context warning, export/import): ce-plan on its doc, then the full flow. Stack on Group A, since it reuses src/settings.rs (settings.json) for the warning threshold. Decisions already made: permanent per-day token totals that are never pruned (day one = earliest indexed usage); the header shows `today · all-time`; a context warning goes into the session's next prompt via a new non-async UserPromptSubmit hook (default 80%, once per crossing); export/import is a full move or backup, and import merges without double counting. Consider moving retention from history.json into settings.json as part of export.
5. Group C (agent coordination): generic rules adapted from /Users/forhad/Downloads/coordination-rules.md, written to marsad's data dir by `install`. `c` copies `claude --append-system-prompt-file <path>` (OSC 52 plus pbcopy/xclip/clip.exe, falling back to showing the command). `b` opens a read-only AGENT-BOARD.md view for the session's repo. `marsad coordination` prints the command.
6. Group D (badges + LinkedIn share): 7 rank tiers from all-time tokens, plus bronze/silver/gold achievements (streak, distinct skills, subagents, busiest day) that survive pruning. Islamic geometric badge design. The share key generates a 1-3 sentence line with `claude -p` (no project or client names), shows a preview to edit or confirm, saves a 1200x627 PNG card, opens the LinkedIn composer with the text prefilled, and reveals the PNG. Show the user a rendered sample card before finalizing the design.

## Rules
- Never run `cargo fmt` (the repo is not rustfmt-clean); match surrounding style by hand.
- Tests live in-module (`#[cfg(test)]`). Keep README.md, CHANGELOG.md (## Unreleased) and assets/SKILL.md in sync.
- Commit messages end with: Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
- Don't send code to external review services (cursor-agent) without the user's opt-in.
