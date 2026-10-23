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
