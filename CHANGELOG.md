# Changelog

`marsad update` shows the bullets under a version as "What's new", and `dist` copies them into the
GitHub release. Keep each bullet to one short line.

## Unreleased

- `c` opens a picker: start Claude Code with coordination, output-hygiene rules, or both
- `marsad rules` lists the rules files and prints the command for any set of them
- Footer notes clear themselves after a few seconds

## 0.4.0 - 2026-10-24

- Questions show QUESTION instead of PERMISSION, and PERMISSION is now APPROVE
- Pressing Esc on a prompt, or interrupting a turn, sets the session back to IDLE
- Sessions stay ACTIVE while a background shell Claude started is still running
- Sessions whose process exited show ENDED reliably
- Sounds and desktop notifications when a session needs you or finishes; `m` toggles them
- The header shows today's and all-time tokens
- A permanent per-day token counter: retention now prunes session details only
- `marsad total` adds today, 7 and 30 days with daily averages
- `marsad context` shows how full a session's context is
- Claude is told once when a session's context passes 80%, so it can write a handoff (`marsad context --warn`)
- `marsad export` and `marsad import` back up or move all data; importing twice never double counts
- `marsad import --restore` brings back this machine's own data after a reinstall
- Coordination rules let several sessions in one repo share an `AGENT-BOARD.md` and avoid editing the same files
- `c` copies the command that starts a session with them (`marsad coordination` prints it)
- `b` shows a repo's agent board: who is working on what, stale claims and `needs-human` conflicts
- `B` shows your rank, Novice to Grandmaster by all-time tokens, and bronze, silver and gold achievements
- Badges never go down: pruning and imports can't take one away; `marsad badges` prints them
- The footer announces a new rank or achievement level once
- `p` prepares a LinkedIn post and a 1200×627 card, with a line Claude drafts from today's redacted work
- The share preview lets you edit the line or the whole post, redraft it, or cancel before anything is saved
- `marsad share --print` prints the post and writes the card without opening anything
- Skills pruned before this version whose transcripts are gone don't count toward Distinct skills

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
