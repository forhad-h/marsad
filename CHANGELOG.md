# Changelog

`marsad update` shows the bullets under a version as "What's new", and `dist` copies them into the
GitHub release. Keep each bullet to one short line.

## 0.3.2 - 2026-10-11

- Session status is right-aligned against the table's right edge

## 0.3.1 - 2026-10-11

- Session status shows its label first and the icon after it (`ACTIVE ◐`)

## 0.3.0 - 2026-10-11

- `marsad update` installs the latest release and shows what changed
- Automatic updates: `marsad update --auto on`
- The dashboard shows a yellow notice when a new version is out
- `s` / `S` sort the session list by status, project, context, time or tokens
- Subagents for each session (`tab`): launching skill, tokens used, context left
- Skill windows stay open when a background subagent finishes, so skill token counts are more accurate
- Session status sits at the right edge, so it stays visible when another window covers the left
- TIME shows how long each session has spent working, idle time excluded, across resumes
- Usage history outlives Claude Code's transcripts; sessions are recorded as soon as they end
- `marsad retention` sets how many days of history to keep (default 30)
- `marsad total`: all-time token total, kept after old days are pruned
- Usage view: totals, daily chart and all-time total; `p` switches the period
- 5h / 7d usage limits show the local time they reset (header, status line, `marsad status`)
- Session history (`enter`): prompts, replies, skills, subagents with their reports, questions with answers
- A subagent handing back its report no longer closes the current skill window

## 0.2.0 - 2026-10-09

- Context left before auto-compact, and 5h / 7d usage limits
- Skill history for each session (`enter`)
- 30-day usage history (`u`), `marsad report`, `marsad sql`
- `marsad status` for live sessions

## 0.1.0 - 2026-10-09

- First release: live dashboard of every Claude Code session
