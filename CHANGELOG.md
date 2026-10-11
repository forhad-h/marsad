# Changelog

`marsad update` shows the bullets under a version as "What's new", and `dist` copies them into the
GitHub release. Keep each bullet to one short line.

## Unreleased

- `marsad update` installs the latest release and shows what changed
- Automatic updates: `marsad update --auto on`
- The dashboard shows a yellow notice when a new version is out
- `s` / `S` sort the session list by status, project, context, age or tokens
- Subagents for each session (`tab`): launching skill, tokens used, context left
- Skill windows stay open when a background subagent finishes, so skill token counts are more accurate
- Sessions waiting on a permission prompt get a slowly pulsing amber row background
- Usage history outlives Claude Code's transcripts; sessions are recorded as soon as they end
- `marsad retention` sets how many days of history to keep (default 30)
- `marsad total`: all-time token total, kept after old days are pruned
- Usage view: totals, daily chart and all-time total; `p` switches the period

## 0.2.0 - 2026-10-09

- Context left before auto-compact, and 5h / 7d usage limits
- Skill history for each session (`enter`)
- 30-day usage history (`u`), `marsad report`, `marsad sql`
- `marsad status` for live sessions

## 0.1.0 - 2026-10-09

- First release: live dashboard of every Claude Code session
