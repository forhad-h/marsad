---
title: Token Counter, Handoff Context and Data Portability - Plan
type: feat
date: 2026-10-23
topic: token-counter-and-data
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Token Counter, Handoff Context and Data Portability - Plan

## Goal Capsule

- **Objective:** Users can see and keep their real token usage from their first day onward: today and all-time at a glance, any week or month later, on any machine. Sessions can find out when their context is nearly full so they hand off before auto-compact.
- **Product authority:** This plan covers group B of four. Status accuracy (A), agent coordination (C), and badges and sharing (D) are separate plans. D consumes the counter defined here.
- **Open blockers:** None.

---

## Product Contract

### Summary

A permanent day-by-day token counter that outlives the 30-day detail history feeds the header (`today · all-time`) and later the share and badge features.
`marsad context` tells a session its own token use and context left, and an automatic one-line warning reaches the session when its context crosses a threshold.
`marsad export` and `marsad import` move all marsad data between machines.

### Problem Frame

The header's `Σ` figure sums only the sessions currently on screen, so it shrinks when sessions end and says nothing about the day or the user's history.
Per-day detail is pruned after 30 days, and pruned days survive only as one all-time sum, so weekly or monthly figures older than a month are lost.
The user wants these numbers to show recruiters and clients how much AI-assisted work they do, which only works if the history is permanent.

Long sessions also lose output quality when Claude Code auto-compacts. A session could write a handoff prompt for a fresh session first, but it has no way to know how close it is to the compaction point.

Finally, all marsad data lives on one machine with no way to back it up or move it.

### Requirements

**Permanent token counter**

- R1. marsad keeps token totals per local calendar day (input, output, cache read, cache write, total), and retention pruning never deletes them.
- R2. The counter starts at the earliest usage marsad can index, including transcripts that predate installing marsad, and grows as new usage is indexed.
- R3. Existing all-time totals from earlier versions are preserved on upgrade, so the all-time figure never drops.
- R4. The header shows today's tokens (local timezone) and the all-time total in place of the visible-sessions sum. Today's figure includes usage from live sessions not yet indexed.
- R5. Usage history, `marsad total` and `marsad report --by day` read daily totals from the counter, so periods longer than the retention still have per-day numbers.
- R6. The counter answers totals and per-day averages for today, the last 7 days, the last 30 days and all time, for use by the share feature.

**Handoff context**

- R7. `marsad context` prints the calling session's tokens used, context used, context left before auto-compact and percent full, as text or JSON. With no arguments it identifies the session it runs in; a session id can be passed explicitly.
- R8. When a session's context crosses the warning threshold, the session's next prompt carries a one-line note telling Claude how full the context is and suggesting it write a handoff now.
- R9. The warning fires once per crossing. It fires again only after the context drops below the threshold (for example after a compact or `/clear`) and rises past it again.
- R10. The threshold defaults to 80% of the auto-compact point and can be changed or turned off with a marsad command.
- R11. The bundled marsad-usage skill documents `marsad context` and the handoff pattern.

**Export and import**

- R12. `marsad export [file]` writes one file holding the permanent counter, the kept session and skill history, and marsad settings (retention, alert mute, context warning threshold).
- R13. `marsad import <file>` merges an export into the local data. Importing the same file twice, or an export taken from this machine, never double counts tokens.
- R14. Import reports what it added and skips data it cannot read without failing the rest.

### Key Decisions

- **Day totals are permanent; detail stays 30 days.** Tokens per day are tiny (about 15 KB a year), while session and skill detail is what grows. (session-settled: user-directed — chosen over keeping only an all-time sum: recruiters need weekly, monthly and per-day figures.) Governs R1, R5.
- **Day one is the earliest indexed usage, not the install date.** Pre-install transcripts are real work the user did. (session-settled: user-approved — chosen over counting only from install.) Governs R2.
- **Context warnings are pushed, not only pulled.** Sessions forget to check, so marsad adds the note itself, in addition to the `marsad context` command. (session-settled: user-approved — chosen over command-only and command plus a handoff skill.) Governs R7, R8.
- **Export means a full move or backup.** (session-settled: user-approved — chosen over counter-only export and analysis-only CSV dumps.) Governs R12, R13.
- **Imports merge rather than replace.** Whichever source has the larger figure for a session-day wins. That keeps re-import idempotent and safe when both machines saw the same session. Governs R13.

### Acceptance Examples

- AE1. **Covers R1, R5.** Given retention is 30 days and the user used 3M tokens 45 days ago, when they open Usage with the "all" period, then that day still shows 3M even though its sessions are gone.
- AE2. **Covers R3.** Given an existing install whose pruned all-time sum is 50M, when the user upgrades, then `marsad total` still reports at least 50M.
- AE3. **Covers R4.** Given a session that used 200k tokens in the last minute and has not been indexed yet, when the header refreshes, then today's figure includes those 200k.
- AE4. **Covers R8, R9.** Given the threshold is 80%, when a session's context goes from 78% to 83%, then the next prompt carries the note once; at 90% no second note appears; after `/compact` drops it to 20% and it later passes 80% again, the note appears again.
- AE5. **Covers R13.** Given an export from machine A, when it is imported on machine B twice, then B's totals increase once.

### Scope Boundaries

- Cloud sync or automatic backup.
- Exporting raw transcripts (Claude Code owns those).
- Spending or cost estimates in money.
- Per-project permanent counters; the permanent counter is per day only.

### Dependencies / Assumptions

- Claude Code sets `CLAUDE_CODE_SESSION_ID` in Bash tool and hook environments, which lets `marsad context` identify its own session.
- A synchronous (non-async) hook's output on prompt submit is added to the session's context. marsad's existing hooks are async and cannot do this, so this needs one additional hook registration.
- Days pruned by earlier versions only kept an all-time sum, so they cannot be split back into days. They count toward all-time totals but not toward any day.

### Outstanding Questions

**Deferred to Planning**

- Export file format and versioning.
- How the header obtains live, not-yet-indexed usage cheaply.

### Sources / Research

- Header total: `draw_header` in `src/ui.rs`. Lifetime and pruning: `prune` and `lifetime` in `src/store.rs`. Retention: `store::retention`.
- Context math: `context_info` and `auto_compact_window` in `src/model.rs`; status line snapshots in `src/statusline.rs`.
- Hook registration: `EVENTS` and `install` in `src/install.rs`.
