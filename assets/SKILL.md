---
name: marsad-usage
description: Analyze Claude Code token usage over any period, including past and closed sessions (by day, session, skill, project, model or subagent) and check live sessions (status, context left before auto-compact, 5h/7d usage limits) using marsad. Use when the user asks which sessions, skills, projects or models used the most tokens, how usage changed over days, how much context or usage limit is left, which sessions need attention, or wants a usage report.
---

# Claude Code usage history (marsad)

`marsad` keeps a SQLite history of every Claude Code session on this machine, built from
transcripts and kept after Claude Code deletes them. Session details are kept for the last 30 days by
default (`marsad retention`); daily token counts are kept forever. `marsad total` gives the all-time
token total plus today, 7 and 30 days with daily averages (`--format json` adds a `periods` list). Each command below indexes new transcript data first (add `--no-sync` to skip).
If `marsad` is not on PATH, run it by its full path: `"{{MARSAD_EXE}}"`.

## Live sessions (right now)

```sh
marsad status --format json        # running sessions: status, activity, context left, usage limits
marsad status --all                # include sessions that have ended
```

Each session has `status` (`approve`, `question`, `error`, `active`, `idle`, `ended`),
`activity`, `context_used` / `context_limit` / `context_left` / `context_pct` (tokens until
auto-compact; `context_estimated: true` means the window size was inferred from the model id), and
`tokens_total`. The top-level `rate_limits.five_hour` / `seven_day` give `used_pct` and `resets_at`
(Unix seconds), from the most recent status line report (claude.ai Pro/Max only; null otherwise).
Live data is not in the usage database; use `marsad status` for it.

## This session's context and handoffs

```sh
marsad context --format json       # the session this runs in: context_used / context_left / context_pct, warn_pct
marsad context --session <id-prefix>
```

Run inside Claude Code, `marsad context` finds its own session. marsad also adds a note to the
prompt once the context passes `warn_pct` (default 80) of the auto-compact point. When you see that
note, or `context_pct` is high, offer to write a handoff prompt: the goal, what is done, decisions
made, open questions, and the next steps, with file paths, so a fresh session can continue.
`marsad context --warn <pct|off>` changes the threshold.

## Several sessions in one repo

```sh
marsad coordination --format json   # {command, protocol_path}: starts a session with the coordination rules
```

Sessions started with that command coordinate on `AGENT-BOARD.md` at the root of the repo's main checkout: beside the absolute `git rev-parse --git-common-dir` when that is a `.git` dir, inside it otherwise (a bare clone or a submodule). Read it to see which sessions are active and which files they have claimed; the dashboard's `b` key shows it too.

## Ready-made reports

```sh
marsad report --by session            # top sessions by tokens (default: all kept history)
marsad report --by skill --days 7     # skills ranked by ≈tokens this week
marsad report --by day                # daily totals, newest first
marsad report --by project --from 2026-10-01 --to 2026-10-07
marsad report --by model | --by agent # per model / per subagent type
marsad report --by skill --session <id-prefix>
```

Filters: `--days N` (1 = today), `--date YYYY-MM-DD`, `--from/--to`, `--session <id prefix>`,
`--project <substring>`, `--limit N` (default 25). Output: `--format table|json|csv`
(use `--format json` when you will process the result).

## Ad-hoc SQL (read-only)

```sh
marsad schema                                   # tables and column meanings
marsad sql "SELECT ..." --format json
```

Tables:
- `sessions(session_id, project, cwd, title, model, first_at, last_at, transcript_path)`
- `usage_daily(session_id, agent_id, agent_type, day, input, output, cache_read, cache_write, total, turns)` —
  one row per session × agent (`main` or a subagent) × local day.
- `token_days(session_id, agent_id, day, input, output, cache_read, cache_write, total, turns)` —
  the permanent counter: same grain as `usage_daily` but never pruned. Use it for totals and daily
  trends older than the retention window.
- `legacy_totals(source, since, through, input, output, cache_read, cache_write, total)` — sums of
  days pruned by marsad before 0.4, one row per install; those days have no `token_days` rows.
- `lifetime` — frozen pre-0.4 data, already copied into `legacy_totals`; don't add it.
- `skill_runs(session_id, agent_id, agent_type, seq, skill, via, at, day, turns, input, output, cache_read, cache_write, total)` —
  one row per skill invocation; `via` is `tool` (Claude invoked it) or `command` (user typed `/skill`).

Example: skills with the highest average cost per call this week

```sh
marsad sql "SELECT skill, COUNT(*) calls, SUM(total)/COUNT(*) avg_tokens
            FROM skill_runs WHERE day >= date('now','localtime','-6 days')
            GROUP BY skill ORDER BY avg_tokens DESC LIMIT 10" --format json
```

## Interpreting the numbers

- `total` = input + output + cache_read + cache_write. Cache reads usually dominate, but they are
  billed at about 1/10 of the input price. To compare cost rather than raw volume, also look at
  `output`, `cache_write`, and `input`.
- Skill tokens are **approximate**: each run counts the turns between the skill's invocation and the
  next user prompt or next skill, so it includes work Claude did while following that skill.
- Days are the machine's local dates. History goes back to the first indexed transcript;
  `marsad retention` shows how many days of details are kept. For totals beyond that, use
  `marsad total` or SQL (all time = `SUM(token_days.total)` + `SUM(legacy_totals.total)`).
- Sessions that ran inside subagents appear under `agent_type` other than `main`.

When reporting, lead with the answer (the top items and their share of the total), then
show a compact table. Mention that skill figures are approximate when they matter.
