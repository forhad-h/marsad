---
title: Isolate a helper `claude -p` call with --safe-mode and --no-session-persistence, not --bare
date: 2026-10-24
category: integration-issues
module: share flow (share.rs claude_command, the drafted LinkedIn line)
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - marsad (or any tool that watches Claude Code sessions) shells out to `claude -p` for a helper task
  - Changing the flag set in `share::claude_command`, or adding a new helper call to Claude Code
  - Tempted to add --bare to make a helper call faster or quieter
symptoms:
  - "A helper call shows up in marsad's own session list as a ghost session and its tokens are counted"
  - "The user's hooks, CLAUDE.md or MCP servers run inside a call that should only see one summary"
  - "With --bare the call fails for claude.ai subscription logins"
root_cause: inadequate_documentation
resolution_type: code_fix
retire_when: "claude --help changes what --bare authenticates with, or drops --safe-mode or --no-session-persistence; check `claude --help` on the installed version"
related_components:
  - tooling
tags: [claude-code, claude-p, safe-mode, no-session-persistence, bare, ghost-session, isolation, share]
---

# Isolate a helper `claude -p` call with --safe-mode and --no-session-persistence, not --bare

## Context

The share flow drafts its LinkedIn line with one `claude -p` call over a redacted summary of
today's work (`src/share.rs`, `claude_command` and `ask`). marsad reads every Claude Code
transcript on the machine, so a careless helper call pollutes the very data it reports on. It
also runs with the user's whole Claude Code configuration unless told otherwise. Three obvious
flag choices each break something:

- A plain `claude -p` writes a transcript. marsad then indexes it as a new session: a ghost row in
  the dashboard, plus tokens counted toward the user's totals and rank.
- A plain call also loads the user's hooks, CLAUDE.md, plugins and MCP servers. marsad's own
  hooks would then log the helper call, and the user's servers would see the summary.
- `--bare` looks like the fix, since it skips hooks, plugins, CLAUDE.md and auto-memory. But
  `claude --help` (2.1.310) says that under `--bare` "Anthropic auth is strictly
  ANTHROPIC_API_KEY", with keychain reads skipped. A user who signs in with a claude.ai
  subscription has no API key, so the call fails for exactly the users most likely to share.

## Guidance

Use this flag set, as `claude_command` does:

```rust
cmd.args(["-p", "--safe-mode", "--no-session-persistence", "--tools", "", "--model", "haiku", "--output-format", "json"])
    .args(["--system-prompt", SYSTEM_PROMPT])
    .current_dir(cwd); // a fresh, empty temp dir
```

- `--safe-mode` turns off CLAUDE.md, skills, plugins, hooks, MCP servers, custom commands and
  agents. Auth still works, so subscription logins are fine.
- `--no-session-persistence` (print mode only) saves no session to disk. No transcript means no
  ghost session and no extra counted tokens.
- `--tools ""` keeps the reply to one turn with no side effects. `claude --help` (2.1.309 and
  2.1.310) has no `--max-turns`, so there is no turn cap to lean on.
- `--system-prompt` is passed inline on every call, and the call is never resumed or continued.
  See `claude-code-system-prompt-snapshot-pins-coordination-protocol-to-new-conversations.md`.
- Run it in a fresh temp dir, so no project directory is anywhere near the call.
- Treat an unknown-option error as "Claude Code is too old" (`Note::TooOld`) and fall back to the
  template line. Never retry without the isolation flags.

`a_clean_reply_is_the_line_and_the_call_is_isolated` in `src/share.rs` pins the argument list, so
an edit that drops a flag or adds `--resume` / `--continue` fails a test.

## Applicability

This applies to any helper call into Claude Code from a tool that also observes Claude Code
sessions. Re-check `claude --help` after a Claude Code upgrade: the isolation depends on what
`--safe-mode` disables and on what `--bare` accepts for auth, and both are version-specific. The
"no ghost session" behaviour was checked once with the real `claude` on the user's Mac during U5
(the plan's Verification Contract). It should be re-run after any change to this flag set.
