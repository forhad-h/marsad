# marsad

A live terminal dashboard for every Claude Code session on your machine: what each one is
doing, which ones are waiting on you, which skills they ran, and how many tokens they used.

```
 marsad   3 sessions · 1 active · 2 need you                    today 2.4M · all-time 133.5M 
╭ Sessions ───────────────────────────────────────────────────────────────────────────────────╮
│ PROJECT        ACTIVITY                                         TIME   TOKENS        STATUS │
│ marsad         Bash: cargo test --all                            37s     2.0M     APPROVE ● │
│ notes          Which database should we use?                     37s     8.1k    QUESTION ? │
│ web         +2 › add dark mode to the settings page              12s   414.8k      ACTIVE ◐ │
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
marsad install     # registers hooks + status line in ~/.claude/settings.json (backup saved alongside), writes the rules files
marsad install --no-statusline   # hooks only; context size is then estimated from transcripts
marsad             # open the dashboard
marsad uninstall   # removes only marsad's hooks, skill and rules files
marsad update      # install the latest version (shows current → latest and what's new)
```

Restart running Claude Code sessions after `install` so they pick up the hooks.

Keys: `↑↓`/`jk` select · `enter` session history · `tab` subagents · `u` usage history · `B` badges · `p` share to LinkedIn · `b` agent board · `c` copy a launch command with coordination and/or output-hygiene rules · `a` show/hide ended sessions · `m` alerts on/off · `s` cycle sort column (status → project → context → time → tokens) · `S` reverse sort · `q` quit.
In session history and subagents: `↑↓` scroll · `g`/`G` top/bottom · `tab` switch between them · `esc` back.
In session history also: `f` filter by event kind · `J`/`K` scroll the selected event's details.

### Alerts

While the dashboard is open it plays a sound when a session starts waiting on you (QUESTION or
APPROVE) and another when a session finishes its work, and shows a desktop notification naming the
project and what it is waiting on. Pressing Esc or interrupting a turn doesn't count as finishing,
and a session that leaves a background shell running finishes only when that work is done. `m`
turns alerts off and on; the choice is kept in `~/.claude/marsad/settings.json`.

macOS uses its Ping and Glass sounds and shows notifications on the lock screen if Script Editor
(which `osascript` runs as) is allowed to in System Settings → Notifications. Linux uses `paplay`
and `notify-send` when installed. Windows plays system sounds only. Without a sound player the
terminal bell rings instead.

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

### Handoff warning

When a session's context passes **80%** of the auto-compact point, marsad tells Claude once, on
your next prompt, that the context is nearly full and that it should consider writing a handoff
prompt for a fresh session. `install` registers this as a synchronous `UserPromptSubmit` hook
(`marsad context --hook`, a few ms) so the note reaches the prompt you just sent. The warning comes
back after a compact or `/clear` brings the context below the threshold.

```sh
marsad context              # this session: context used, left, and the warning threshold
marsad context --session 1fe6 --format json
marsad context --warn 70    # warn at 70% instead
marsad context --warn off
```

Inside Claude Code (a Bash tool call or a hook), `marsad context` finds its own session; elsewhere
pass `--session`.

### Session history

`enter` on a session opens a scrollable timeline of what happened in it, oldest first:

| Event | Shows |
|---|---|
| prompt | what you typed (including `/commands` and `!` shell commands), and the tokens of the turn it started |
| reply | Claude's reply that ended the turn |
| skill | the skill, whether Claude called it (`tool`) or you typed it (`/cmd`), turns, output and ≈ tokens in its window |
| agent | the subagent's type and task, running or done, its tokens, and its final report |
| question | each AskUserQuestion question and your answer (or the note you left when declining) |

Skills and questions from subagents are included, marked with the subagent's type. The pane
under the list shows the selected event in full; `J`/`K` scroll it, and `f` filters the list to
one kind of event. Prompts, replies and reports are kept up to 4,000 characters each.

### Subagents

`tab` on a session lists the main agent and every subagent it launched, oldest first: agent type,
task description, the skill that launched it, skills it ran itself, running or done, tokens used,
and context left before that agent's own auto-compact. Each subagent has its own context window,
so a subagent's used tokens (summed across all its turns) can be far above its context fill.

### Coordinating several sessions

When several Claude Code sessions work in one repo, marsad's coordination rules keep them from editing the same files. Sessions that load the rules share one `AGENT-BOARD.md` at the root of the repo's main checkout (one board for all its worktrees, kept out of git through `.git/info/exclude`; a bare clone or a submodule keeps it inside its git directory). On it they register, claim files before editing them, record binding decisions, message each other, and log conflicts they can't settle as `needs-human`.

`c` opens a picker of the rules marsad ships: **Coordination** (above) and **Output hygiene** (keeps the request, who asked and how the work was done out of code, commits, PRs and docs). Tick one or both with `space` or its number, then press `enter` to copy the command. Your choice is remembered, so next time `c` then `enter` is enough.

```sh
claude --append-system-prompt-file ~/.claude/marsad/coordination.md                  # coordination
claude --append-system-prompt-file ~/.claude/marsad/output-hygiene.md                # output hygiene
claude --append-system-prompt-file ~/.claude/marsad/coordination+output-hygiene.md   # both
```

Claude Code reads only the last `--append-system-prompt-file` it is given, so a set of rules is merged into one file per combination rather than passed as several flags.

The rules are added to the session's system prompt, so they survive `/clear` and compaction. They apply to new conversations only: a resumed conversation keeps the system prompt it started with. The coordination rules add about 2k tokens to each request of those sessions.

Over SSH, `c` sends the command through your terminal (OSC 52, which kitty, WezTerm, Alacritty and iTerm2 with clipboard access allow; tmux needs `set-clipboard on`) and also shows it so you can copy it by hand. `marsad rules` lists the rules and `marsad rules coordination output-hygiene` prints the command (`--format json` for scripts); `marsad coordination` is short for `marsad rules coordination`.

`b` on a session opens its repo's board, read-only and live: agents with their last-seen times, claims, decisions, messages, conflicts and handoff notes. Agents and their claims not seen for more than 2 hours are marked `[stale]`, and `needs-human` conflicts are tagged in yellow. marsad never edits the board; the sessions own it. The rules live in marsad's data dir and are rewritten by each version, so copy the file elsewhere if you want to change them.

### Badges and sharing

`B` shows your rank and four achievements, each with its level in words and how far it is to the
next, beside a drawing of your badge. The rank counts every token, cache reads included, so it
matches the header's all-time figure.

| Rank | All-time tokens | Badge |
|---|---|---|
| Novice | 0 | carved sandstone |
| Apprentice | 1B | carved cedar |
| Practitioner | 10B | bronze, Moorish tilework |
| Specialist | 40B | silver, Moorish tilework |
| Expert | 100B | gold, Moorish tilework |
| Master | 200B | gold, Persian tilework |
| Grandmaster | 400B | jewelled gold, Persian tilework |

The ladder is paced for years, not weeks: at about 8B tokens a month, a heavy daily user reaches
Practitioner in the first weeks, then about one rank a year, and Grandmaster in about four years.
Gold achievements take a similar effort.

| Achievement | Bronze | Silver | Gold |
|---|---|---|---|
| Longest streak (days in a row with usage) | 14 | 60 | 180 |
| Distinct skills | 25 | 75 | 150 |
| Subagents run | 1,000 | 10,000 | 50,000 |
| Busiest day (tokens) | 2B | 5B | 10B |

marsad keeps the best value each one ever reached, so retention pruning or an import never takes a
badge away. When a rank or level rises, the footer says so once (Esc dismisses it). `marsad badges`
prints the same (`--format json` or `csv` for scripts).

In the badge view, `s` saves the view as an image, `Downloads/marsad-badge-YYYY-MM-DD.png`, and
shows it in your file manager. `L` does the same and opens LinkedIn's post box with a one-line
caption, so you only attach the image. `marsad badges --png [PATH]` saves it from the command line.
The image is drawn from the view itself (same art, colours and text), so it looks the same in any
terminal.

`p`, from the list or the badge view, prepares a LinkedIn post: a line about today's work drafted by
Claude Code, then today's, the last 7 and 30 days', the daily average and all-time tokens, and your
rank. You see the whole post first: `enter` confirms, `e` edits the line, `E` opens the whole post
in `$VISUAL`/`$EDITOR`, `r` drafts a new line, and `q`/`esc` cancels without writing or opening
anything. Confirming:

1. saves a 1200×627 card with your badge, rank, stats, achievements and `git config user.name` to
   `Downloads/marsad-YYYY-MM-DD.png` (or `~/.claude/marsad/cards/`); sharing again the same day
   replaces it,
2. copies the post to your clipboard,
3. opens LinkedIn's post composer with the text filled in,
4. shows the card in your file manager, so you can attach it.

LinkedIn's prefill link is undocumented, so if the composer opens empty, paste the post; a post
whose link would be too long opens the plain feed instead. Over SSH or without a display, steps 3
and 4 are skipped and the link and the card's path are shown.

**What leaves your machine.** Only the drafting does, and only through your own Claude Code login:
one `claude -p` call (Haiku, no tools, no hooks, CLAUDE.md, plugins or MCP, and no transcript, so it
doesn't appear as a session) gets today's session titles, the skills and subagent types you used
with their counts, and the token figures. Every project and folder name from today's sessions is
removed from them first, including the folders above a project outside your home folder (system
folders such as `/srv`, `/opt` or `/Volumes` aside), and project and folder fields are never sent. If the reply still names one,
marsad uses a template line instead and says why. A client or company named only inside a session
title, and not in any project or folder name, is **not** removed: read the line in the preview
before you confirm. Without Claude Code, or if drafting fails or takes over a minute, the post uses
a template line built from the stats, with a note saying so.

```sh
marsad share --print               # print the post and write the card; opens nothing
marsad share --print --no-ai --out card.png
```

## How it works

- **Status** comes from [Claude Code hooks](https://code.claude.com/docs/en/hooks). Each event runs
  `marsad hook` (async, a few ms), which appends a trimmed line to `~/.claude/marsad/events.jsonl`.

  | Status | Events |
  |---|---|
  | Approve | `PermissionRequest`, `Notification[permission_prompt]` (except for questions) |
  | Question | `AskUserQuestion` tool (including its permission request), `Elicitation`, `Notification[elicitation_dialog]` |
  | Active | `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `SubagentStart`, `PreCompact`; or a background shell Claude started since the session last launched or resumed is still running |
  | Idle | `Stop`, `Notification[idle_prompt]`; or the transcript shows the turn ended with no `Stop` (Esc on a prompt, or an interrupted turn) |
  | Error | `StopFailure` |
  | Ended | `SessionEnd`, or the session's Claude Code process is gone (every hook event records its pid) |

- **Tokens** are read from each session's transcript (`message.usage`, deduped by message id),
  including subagent transcripts under `<session>/subagents/`. Hooks don't carry usage data.
- **Per-skill tokens (≈)** are approximate: after a skill is invoked (Skill tool or a typed
  `/skill`), each assistant turn is charged to it until the next user prompt or the next skill.

### Usage history

`u` opens the Usage view: totals for the period on top (tokens, sessions, average per day, busiest
day, a daily chart, and your all-time total), days on the left, and the selected day's sessions and
skills ranked by tokens on the right. `p` / `P` switch the period (the last 7 days, or everything
kept; with a longer retention also 30, 90 and 365 days). `tab` switches pane, and `enter` on a
session shows its per-day totals and every skill run. "Total" aggregates the whole period.

The history is a SQLite database at `~/.claude/marsad/usage.db`, indexed from Claude Code
transcripts (`~/.claude/projects`). It includes sessions from before marsad was installed.
Tokens are bucketed by the local day each API call happened, so long sessions split across days.

Session details (sessions, per-agent days, skill runs) are kept for the **last 30 days** by
default; `marsad retention` changes that. Rows stay even after Claude Code deletes their
transcripts (it does after `cleanupPeriodDays`, 30 days by default). Each session is indexed when it ends (the `SessionEnd` hook starts a background
`marsad sync`), so it is recorded even if the dashboard isn't open. The dashboard also re-indexes
every minute while open, and `marsad sync` / `report` / `sql` / `total` index on demand (only
changed files are re-read).

Retention never touches the token counter: every session's tokens per agent and day are kept
for good, so the header's today and all-time figures, `report --by day` and the Usage view's daily
chart reach back past the retention window. `marsad total` prints the all-time total, ready to
share, with today, the last 7 and 30 days and their daily averages:

```sh
marsad total                # Claude Code tokens since Sep 21, 2026: 133,533,863 (133.5M) + breakdown
marsad total --format json  # also a `periods` list: today, 7d, 30d, all
marsad retention            # show how many days of details are kept
marsad retention 90         # keep 90 days; older details are deleted on the next sync
marsad retention forever
```

The all-time total starts from the oldest day marsad indexed, so usage that was already gone from
Claude Code's transcripts when marsad first ran isn't included. Days pruned by versions before 0.4
survive only as one sum, so they count toward all time but have no daily rows.

### Export and import

```sh
marsad export                     # writes marsad-export-YYYY-MM-DD.json in the current directory
marsad export ~/marsad-backup.json
marsad import ~/marsad-backup.json
marsad import --restore ~/marsad-backup.json   # this machine's own earlier data, after a reinstall
```

The export holds the token counter, the kept history and your settings. `import` merges it into
this machine's data and prints what it added: days and sessions already present keep the larger
count, so importing the same file twice, or importing a machine's own export back, never double
counts. Settings from the file fill only those not set here.

Use `--restore` when the file came from this same machine (a reinstall, or a new data folder) and
Claude Code's old transcripts are still on disk. Days that a pre-0.4 version had pruned into one
sum are then kept out of the day counter here, so they aren't counted twice. Without it, an
import never removes this machine's own days.

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

- `/btw` side questions and `!` shell commands fire no hook and leave nothing in the transcript, so
  they don't change a session's status.

- After you approve a permission prompt, the row stays on APPROVE until the tool finishes
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
