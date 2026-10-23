# Concepts

Shared domain vocabulary for this project — entities, named processes, and status concepts with project-specific meaning. Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Session state

### Session status
What a Claude Code session is doing right now, as the dashboard shows it: Approve, Question, Error, Active, Idle or Ended, ordered by how urgently the user is needed.

Approve means a tool call is waiting for permission; Question means Claude asked the user something (including AskUserQuestion, even though Claude Code also raises a permission request for it). Status comes from hook events first and is then corrected by reconciliation.

### Reconciliation
The correction applied on every refresh, where facts only the transcript knows override the hook-derived status: an inferred interrupt makes the session Idle, and a running background shell makes an Idle session Active.

The transcript is written slightly after the hooks, so reconciliation never overrides a status entered after the transcript's turn end, and never infers an interrupt while the main agent's last hook was a Stop or session start.

### Interrupt
A turn the user ended with Esc or an interrupt. Claude Code fires no hook for it, so it is inferred: the transcript records a turn end, no newer hook from the main agent arrived, and a short grace period for a late Stop has passed.

### Idle cause
Why a session last went Idle: it finished its turn, it was interrupted, or something else (session start, idle notification). Only a finished turn counts as "done" for alerts.

### Background shell
A shell command Claude left running after its tool call returned, either because it asked to or because the command timed out and was moved to the background. It keeps its session Active until a task notification reports it finished. Shells launched by an earlier process of a resumed session are ignored, since they never report.

## Token history

### Day counter
The permanent record of tokens used per session, agent and local calendar day. Retention never prunes it, so totals for any day, week or all time come from it.
*Avoid:* lifetime total

Retention prunes only the detail history (sessions, per-agent days, skill runs); the day counter outlives it. Re-indexing a transcript replaces that transcript's days rather than adding to them.

### Legacy total
One all-time sum of days that a version before the day counter had already pruned, kept per install together with the day range it covers. Its parts are gone, so it counts toward all-time totals but toward no single day.

Days inside a legacy total's range never enter the day counter, but only when the legacy total is this install's own or came from a restore import. Another install's legacy total covers only that install's sessions and blocks nothing here.

### Restore import
An import the user marks as this machine's own earlier data, for a reinstall with the old transcripts still on disk. It makes the file's legacy totals block the day counter here too, and removes days already counted inside them. A plain import never removes this machine's days.

## Coordination

### Coordination protocol
The rules marsad ships for Claude Code sessions that work in the same repo at once: register on the agent board, claim files before editing them, and record decisions, messages, conflicts and handoff notes there. A session follows them only when it was started with them in its system prompt.

Claude Code records a conversation's system prompt on its first request and replays it on resume, so the protocol reaches new conversations only; refreshing the rules after an upgrade does not change conversations already started.

### Agent board
The one shared markdown file per repo through which coordinating sessions see each other, kept beside the repo's main checkout so every worktree of the repo shares it (inside the git directory when there is no checkout of its own). Sessions own and edit it under the coordination protocol; marsad only reads and shows it.
*Avoid:* coordination file

### Claim
A session's entry on the agent board saying it is editing a path or glob, so other sessions leave it alone. A claim is stale when its owner has not been seen within the staleness window, and orphaned, and so stale too, when no agent row names its owner.

### Needs-human conflict
A conflict on the agent board that the sessions could not settle between themselves and left for the user. Only the conflict's final status cell decides it, so a message that merely mentions needs-human is not one.
