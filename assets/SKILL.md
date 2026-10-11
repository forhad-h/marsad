---
name: marsad-usage
description: Analyze Claude Code token usage over any period, including past and closed sessions (by day, session, skill, project, model or subagent) and check live sessions (status, context left before auto-compact, 5h/7d usage limits) using marsad. Use when the user asks which sessions, skills, projects or models used the most tokens, how usage changed over days, how much context or usage limit is left, which sessions need attention, or wants a usage report.
---

# Claude Code usage history (marsad)

`marsad` keeps a SQLite history of every Claude Code session on this machine, built from
transcripts and kept after Claude Code deletes them (forever, unless `marsad retention` sets a limit). Each command below indexes new transcript data first (add `--no-sync` to skip).
If `marsad` is not on PATH, run it by its full path: `"{{MARSAD_EXE}}"`.

## Live sessions (right now)

```sh
marsad status --format json        # running sessions: status, activity, context left, usage limits
marsad status --all                # include sessions that have ended
```

Each session has `status` (`permission`, `question`, `error`, `active`, `idle`, `ended`),
`activity`, `context_used` / `context_limit` / `context_left` / `context_pct` (tokens until
auto-compact; `context_estimated: true` means the window size was inferred from the model id), and
`tokens_total`. The top-level `rate_limits.five_hour` / `seven_day` give `used_pct` and `resets_at`
(Unix seconds), from the most recent status line report (claude.ai Pro/Max only; null otherwise).
Live data is not in the usage database; use `marsad status` for it.

## Ready-made reports

```sh
marsad report --by session            # top sessions by tokens (default: all history)
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
  `marsad retention` shows whether older rows are being deleted.
- Sessions that ran inside subagents appear under `agent_type` other than `main`.

When reporting, lead with the answer (the top items and their share of the total), then
show a compact table. Mention that skill figures are approximate when they matter.
