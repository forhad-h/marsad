---
title: Multi-Session Agent Coordination - Plan
type: feat
date: 2026-10-23
topic: agent-coordination
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Multi-Session Agent Coordination - Plan

## Goal Capsule

- **Objective:** Several Claude Code sessions working in the same repo know what each other are doing and avoid editing the same files. Users can start such a session with one paste and watch the coordination from the dashboard.
- **Product authority:** This plan covers group C of four. Status accuracy (A), the token counter (B), and badges and sharing (D) are separate plans and not active scope.
- **Open blockers:** None.

---

## Product Contract

### Summary

marsad ships a generic coordination protocol that sessions follow through a shared `AGENT-BOARD.md` at the repo root.
`c` in the dashboard copies a `claude --append-system-prompt-file <path>` command that loads the protocol into a new session's system prompt, so it survives `/clear` and compaction.
`b` opens a read-only view of the selected session's board.

### Problem Frame

The user often runs several sessions on one project, each on a different improvement. When two touch the same file they overwrite or conflict with each other.
They wrote a coordination protocol (`coordination-rules.md`) that solves this, but it has stack-specific examples, lives in their Downloads folder, and must be loaded into every session by hand. Instructions given in a prompt are lost on `/clear` and weakened by compaction.

### Requirements

**The protocol**

- R1. marsad bundles a coordination protocol derived from the user's `coordination-rules.md`: register on the board, claim files before editing, binding decisions, messages, conflicts, handoff notes, and keeping the board small.
- R2. The protocol is project-agnostic. Stack-specific examples become generic categories (lockfiles, dependency manifests, schemas, migrations, CI config, generated files), and nothing assumes a particular language, framework or branch name beyond "the default branch".
- R3. The protocol works in a repo with git worktrees (one board per repo) and degrades sensibly outside a git repo (board in the current directory).
- R4. `marsad install` writes the protocol to a stable path under marsad's data directory, and upgrading marsad refreshes it. `marsad uninstall` removes it.

**Launching sessions with it**

- R5. `c` in the dashboard copies `claude --append-system-prompt-file <absolute path>` to the clipboard, and the footer confirms it. The user does not need to know where the file lives.
- R6. When the clipboard cannot be reached, the dashboard shows the full command so the user can copy it by hand.
- R7. A `marsad coordination` command prints the same launch command and the protocol's path, for scripts and terminals without the dashboard.

**Board view**

- R8. `b` on a session opens a view of the `AGENT-BOARD.md` for that session's repo, showing agents with last-seen times, claims, decisions, messages, open conflicts and needs-human items.
- R9. Agents and claims whose last-seen time is over 2 hours old are shown as stale. Needs-human conflicts are highlighted.
- R10. The board view is read-only and refreshes while open. When the repo has no board, it says so and shows the launch command.

### Key Decisions

- **System prompt file, not CLAUDE.md.** Appending to the system prompt survives `/clear` and compaction and does not touch the user's repo files. Governs R5.
- **Show the board in the dashboard.** Users spot collisions without opening the file. (session-settled: user-directed — chosen over rules plus copy key only, and over conflict flags only.) Governs R8.
- **Its own view, not the details pane.** The details pane has room for about nine lines; a board needs more. Governs R8.
- **marsad never edits the board.** Sessions own it under the protocol. A second writer would break the protocol's one-line edit rules. Governs R10.

### Acceptance Examples

- AE1. **Covers R5, R6.** Given the dashboard is open on macOS, when the user presses `c`, then the clipboard holds the launch command and the footer says it was copied; over SSH where no clipboard is reachable, the footer shows the command itself.
- AE2. **Covers R3, R8.** Given two sessions in different worktrees of one repo, when the user presses `b` on either, then both show the same board from the main checkout.
- AE3. **Covers R9.** Given an agent whose last-seen time is 3 hours ago, when the board view opens, then that agent and its claims are marked stale.

### Scope Boundaries

- Editing, sweeping or resolving board entries from marsad.
- Enforcing claims (for example blocking edits through hooks).
- Coordination across machines.

### Dependencies / Assumptions

- `claude --append-system-prompt-file` is listed in Claude Code 2.1.309's help next to `--append-system-prompt`. Assumed to work in interactive sessions.
- Board entries follow the protocol's template, so the view can parse its sections and tables. Lines it cannot parse are still shown as text.

### Outstanding Questions

**Deferred to Planning**

- Clipboard mechanism per platform (terminal escape sequence and/or system clipboard tools).

### Sources / Research

- Source protocol: the user's `coordination-rules.md` (outside the repo; its content is to be bundled with the edits in R2).
- Bundled assets pattern: `assets/SKILL.md` is embedded and written by `install` in `src/install.rs`.
