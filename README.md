# marsad

A live terminal dashboard for every Claude Code session on your machine: what each one is
doing, which ones are waiting on you, which skills they ran, and how many tokens they used.

```
 marsad   3 sessions · 1 active · 2 need you                                    Σ 2.0M tokens
╭ Sessions ───────────────────────────────────────────────────────────────────────────────────╮
│  STATUS       PROJECT        ACTIVITY                                          AGE   TOKENS │
│ ● PERMISSION  marsad         Bash: cargo test --all                            37s     2.0M │
│ ? QUESTION    notes          Which database should we use?                     37s     8.1k │
│ ◐ ACTIVE      web         +2 › add dark mode to the settings page              12s   414.8k │
╰─────────────────────────────────────────────────────────────────────────────────────────────╯
╭ web · Dark mode settings ───────────────────────────────────────────────────────────────────╮
│session  ~/Projects/web   model claude-opus-5-5   mode auto   id 1fe612e4                    │
│tokens   in 54   out 12.8k   cache read 343.9k   cache write 58.0k   total 414.8k            │
│agents   main 312.1k  ·  Explore ✓ 15.8k  ·  general-purpose ◐ 41.3k                         │
│skills   code-review ×1 ≈140.9k                                                              │
╰─────────────────────────────────────────────────────────────────────────────────────────────╯
```

## Install

macOS / Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/forhad-h/marsad/releases/latest/download/marsad-installer.sh | sh
```

Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/forhad-h/marsad/releases/latest/download/marsad-installer.ps1 | iex"
```

Or download a binary from [Releases](https://github.com/forhad-h/marsad/releases), or build from
source with `cargo install --git https://github.com/forhad-h/marsad`.

## Usage

```sh
marsad install     # registers async hooks in ~/.claude/settings.json (backup saved alongside)
marsad             # open the dashboard
marsad uninstall   # removes only marsad's hooks
```

Restart running Claude Code sessions after `install` so they pick up the hooks.

Keys: `↑↓`/`jk` select · `a` show/hide ended sessions · `q` quit.

## How it works

- **Status** comes from [Claude Code hooks](https://code.claude.com/docs/en/hooks). Each event runs
  `marsad hook` (async, a few ms), which appends a trimmed line to `~/.claude/marsad/events.jsonl`.

  | Status | Events |
  |---|---|
  | Permission | `PermissionRequest`, `Notification[permission_prompt]` |
  | Question | `AskUserQuestion` tool, `Elicitation`, `Notification[elicitation_dialog]` |
  | Active | `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `SubagentStart`, `PreCompact` |
  | Idle | `Stop`, `Notification[idle_prompt]` |
  | Error | `StopFailure` |
  | Ended | `SessionEnd`, or the session's Claude Code process is gone |

- **Tokens** are read from each session's transcript (`message.usage`, deduped by message id),
  including subagent transcripts under `<session>/subagents/`. Hooks don't carry usage data.
- **Per-skill tokens (≈)** are approximate: after a skill is invoked (Skill tool or a typed
  `/skill`), each assistant turn is charged to it until the next user prompt or the next skill.

### Known limitations

- After you approve a permission prompt, the row stays on PERMISSION until the tool finishes
  (Claude Code fires no hook on approval).
- Sessions started before `marsad install` show up only after they are restarted.
- The event log rotates at 20 MB (`events.1.jsonl` keeps the previous one).

## Development

```sh
cargo test
MARSAD_HOME=/tmp/marsad-dev cargo run          # use a separate event log
```

## Releasing

Releases are built by [`dist`](https://opensource.axo.dev/cargo-dist/) in
`.github/workflows/release.yml` for macOS (arm64, x64), Linux (arm64, x64) and Windows (x64).
Bump `version` in `Cargo.toml`, commit, then:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

After changing `dist-workspace.toml`, run `dist generate` to refresh the workflow.
