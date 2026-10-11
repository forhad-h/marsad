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
marsad install     # registers async hooks + status line in ~/.claude/settings.json (backup saved alongside)
marsad install --no-statusline   # hooks only; context size is then estimated from transcripts
marsad             # open the dashboard
marsad uninstall   # removes only marsad's hooks
marsad update      # install the latest version (shows current → latest and what's new)
```

Restart running Claude Code sessions after `install` so they pick up the hooks.

Keys: `↑↓`/`jk` select · `enter` skill history · `u` 30-day usage · `a` show/hide ended sessions · `s` cycle sort column (status → project → context → age → tokens) · `S` reverse sort · `q` quit.
In skill history: `↑↓` scroll · `g`/`G` top/bottom · `esc` back.

### Updates

The dashboard checks GitHub for a new release once a day and shows a yellow
``v0.3.0 available · run `marsad update` `` notice in the bottom right. `marsad update` shows your
current and the latest version, a short "What's new", and installs it with the release's installer
(into the same directory as the running binary). `marsad update --check` only shows the versions.

```sh
marsad update --auto on    # the dashboard installs new versions in the background
marsad update --auto off
```

With automatic updates on, the notice changes to `updated to v0.3.0 · restart marsad`. Set
`MARSAD_NO_UPDATE_CHECK=1` to turn off the daily check. Builds from source (`cargo install`, or a
binary outside a `bin` directory) can't update themselves; reinstall them the same way instead.

### Context and usage limits

The **CONTEXT LEFT** column and the details pane show how many tokens remain before
auto-compact, colored green → yellow (60%) → red (85%). The header shows your 5-hour and 7-day
usage limits with time to reset (claude.ai Pro/Max only).

These exact figures come from Claude Code's status line feed, so `marsad install` sets
`statusLine` to `marsad statusline`, which prints a compact `ctx 42% · 412k left · 5h 23% · 7d 41%`.
If you already had a status line, marsad wraps it: your status line keeps rendering exactly as
before, and `marsad uninstall` restores it. Without the status line, the window size is inferred from the
model id (marked `≈`) and usage limits are not shown.

Auto-compact point: 967K on native 1M-context models, otherwise the full window, or your
`autoCompactWindow` / `CLAUDE_CODE_AUTO_COMPACT_WINDOW` if set.

### Skill history

`enter` on a session opens a scrollable timeline of every skill invocation (main agent and
subagents): time, agent, skill, whether Claude called it (`tool`) or you typed it (`/cmd`), turns,
output tokens and ≈ total tokens in its window.

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

### 30-day usage history

`u` opens the Usage view: days on the left, and the selected day's sessions and skills ranked by
tokens on the right (`tab` switches pane, `enter` on a session shows its per-day totals and every
skill run). "All 30 days" aggregates the whole window.

The history is a SQLite database at `~/.claude/marsad/usage.db`, indexed from Claude Code
transcripts (`~/.claude/projects`). It includes sessions from before marsad was installed.
Tokens are bucketed by the local day each API call happened, so long sessions split across days.
Rows older than 30 days are deleted automatically. The dashboard re-indexes every minute while open, and
`marsad sync` / `report` / `sql` index on demand (only changed files are re-read).

```sh
marsad report --by skill --days 7          # skills by ≈tokens this week
marsad report --by session --date 2026-10-04
marsad report --by day --format json       # also: project, model, agent; csv
marsad sql "SELECT skill, SUM(total) FROM skill_runs GROUP BY skill" --format json
marsad schema
marsad status                              # live: status, activity, context left, usage limits
marsad status --format json --all          # include ended sessions
```

**For Claude:** `marsad install` adds a personal `marsad-usage` skill
(`~/.claude/skills/marsad-usage/SKILL.md`). Ask any Claude Code session things like "which
skills used the most tokens this week?" or "how much context is left in my other sessions?" and
it will use `marsad report` / `marsad sql` / `marsad status`. `marsad sql` accepts only read-only
statements.

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
Bump `version` in `Cargo.toml`, rename `## Unreleased` in `CHANGELOG.md` to `## X.Y.Z - date`
(dist puts that section in the release notes, and `marsad update` shows it as "What's new"),
commit, then:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

After changing `dist-workspace.toml`, run `dist generate` to refresh the workflow.
