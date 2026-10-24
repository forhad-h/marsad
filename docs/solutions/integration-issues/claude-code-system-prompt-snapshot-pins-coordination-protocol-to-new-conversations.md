---
title: Claude Code snapshots the system prompt per conversation, so a changed or resumed --append-system-prompt-file only reaches new conversations
date: 2026-10-23
category: integration-issues
module: agent coordination (coordination.md, claude launch command)
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - Changing the coordination protocol text or refreshing coordination.md after an upgrade
  - Launching claude with --append-system-prompt-file together with --resume or --continue
  - Calling claude -p from marsad (e.g. to write a share line) and expecting a new prompt file to take effect
symptoms:
  - "An agent ignores a newly added coordination rule after the prompt file changed"
  - "A resumed conversation does not follow the protocol passed on the new launch"
root_cause: inadequate_documentation
resolution_type: documentation_update
retire_when: "Claude Code changes the default of --system-prompt-snapshot or how resume and compaction treat a newly passed prompt file; check claude --help and the Claude Code changelog"
related_components:
  - tooling
tags: [claude-code, system-prompt, append-system-prompt-file, system-prompt-snapshot, resume, compaction, coordination-protocol]
---

# Claude Code snapshots the system prompt per conversation, so a changed or resumed --append-system-prompt-file only reaches new conversations

## Context

marsad ships a coordination protocol (`assets/COORDINATION.md`) and writes it to `<data_dir>/coordination.md`. The `c` key and `marsad coordination` give the user `claude --append-system-prompt-file <abs path>` (`src/coordination.rs:69-71`). The system prompt was chosen over CLAUDE.md or a pasted prompt so the rules survive `/clear` and compaction. This work is on the `feat/agent-coordination` branch, unmerged as of this writing (stacked on PR #21, which is stacked on PR #20).

During planning, `claude --help` was read for how the system prompt behaves across resumes and compaction. A `claude -p` run confirmed a file's text reaches the model, and a run with a missing file failed with `Error: Append system prompt file not found` (session history). Research also found that nothing re-ran `install` after a marsad upgrade, so the file on disk could go stale. marsad now refreshes it at every entry point. That fixes the file, not the conversations already running (session history).

The point that is easy to miss: Claude Code snapshots the system prompt per conversation. Changing the prompt file, or passing the flag again on a resume, does not change a conversation that already exists.

## Guidance

Observed on Claude Code 2.1.309. `claude --help` documents the switch:

> `--system-prompt-snapshot <on|off>` Record the system prompt once per conversation and reuse it verbatim on every request and resume. on (the default, except under --bare outside cloud and background sessions): the prompt is rendered on the conversation's first request — a --system-prompt or --append-system-prompt included — sent, and recorded; every later request and resume sends the record as-is, even when a later launch passes different text, until the conversation is compacted. off: never record; the prompt is rendered fresh every request (for iterating on prompt text).

Rules that follow:

1. The protocol reaches new conversations only. `claude --resume` or `--continue` with `--append-system-prompt-file` does not add it to an existing conversation.
2. Refreshing `coordination.md` does not update conversations that already exist; they keep the text they started with. marsad refreshes the file at dashboard start because `marsad update` only swaps the binary (`src/app.rs:307-308` calls `ensure_protocol`, `src/coordination.rs:17`), but conversations started before the update keep the old rules.
3. To iterate on protocol text, turn snapshotting off so the prompt is rendered fresh on every request:
   ```
   claude --system-prompt-snapshot off --append-system-prompt-file ~/.claude/marsad/coordination.md
   ```
4. Don't promise that compacting a resumed conversation picks up a newly passed file. That is unverified; the help text says only that the record holds "until the conversation is compacted". User-facing text says "new conversations only".
5. `--append-system-prompt-file` has no help line of its own in 2.1.309 (it is only listed under another flag), but it works, and a missing file was observed to fail loudly with `Append system prompt file not found` (session history).
6. Because of (5), `c` and `marsad coordination` refresh the file first and copy or print nothing when the write fails: a command pointing at a missing file is worse than none. See `copy_launch_command` (`src/app.rs:418-425`; the `Err` arm sets `self.error` and returns) and `run` (`src/coordination.rs:81-82`; `ensure_protocol()?` runs before any print).

## Why This Matters

Someone who edits the protocol, or reads the code and assumes a resumed session picks up the new rules, sees "the agent ignores the new rule" and can spend a long time on it. The agent and the file are fine; the conversation is replaying its recorded prompt. Group D plans to call `claude -p` with a prompt and could hit the same thing.

The tree says what happens but not why or how to get around it: `README.md:150`, the footer at `src/app.rs:429` and the CLI line at `src/coordination.rs:93`. None names the snapshot or the `off` switch.

## When to Apply

- Changing `assets/COORDINATION.md` and testing the change.
- Debugging an agent that doesn't follow a rule that is in `coordination.md`.
- Writing user-facing copy about when the rules take effect.
- Adding a feature that relies on a `--system-prompt` or `--append-system-prompt*` flag together with a resume or continue.
- Upgrading marsad while sessions are open.

## Examples

Symptom: you add a rule to `coordination.md`, resume a session with the launch command, and the agent ignores the rule. Cause: the resumed conversation sent its recorded prompt. Check by starting a new conversation with the same command; while iterating, launch with `--system-prompt-snapshot off`.

Right copy: "copied the coordination command · it applies to new conversations" (`src/app.rs:429`) and "Start new Claude Code sessions with this command; resumed conversations keep their old prompt." (`src/coordination.rs:93`).

Wrong copy: "restart or resume your session to pick up the new rules".

This holds for the Claude Code version observed. Re-check `claude --help` for `--system-prompt-snapshot` after a Claude Code upgrade.

## Related

- `docs/solutions/integration-issues/claude-code-hooks-miss-interrupts-and-background-shells.md`: another Claude Code platform behavior marsad works around (different problem).
- Plan: `docs/plans/2026-10-23-0716-feat-agent-coordination-plan.md` (Dependencies / Assumptions, Risks & Dependencies, KTD2).
