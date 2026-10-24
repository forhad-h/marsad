---
title: Claude Code hooks alone can't tell interrupts, questions and background shells apart
date: 2026-10-23
category: integration-issues
module: session status (events, model, transcript)
problem_type: integration_issue
component: data_model
symptoms:
  - A session stays ACTIVE or APPROVE after the user presses Esc or interrupts the turn
  - An AskUserQuestion prompt shows as APPROVE instead of QUESTION
  - A session reads IDLE (and plays the done sound) while a background shell Claude started is still running
  - After `claude --resume`, a session stays ACTIVE forever on a background shell from the old process
  - Subagent work right after Stop is briefly misread as an interrupt
root_cause: wrong_api
resolution_type: code_fix
severity: medium
retire_when: Claude Code fires a hook for Esc/interrupts and for background shell completion; check the hooks reference at code.claude.com/docs for an interrupt or task-completion event
tags: [claude-code-hooks, transcript, interrupt, background-shell, askuserquestion, status, resume]
---

# Claude Code hooks alone can't tell interrupts, questions and background shells apart

## Problem

marsad derives each session's status from Claude Code hook events. Several state changes the user cares about produce no hook at all, or produce hooks that look like something else, so a hook-only status machine shows the wrong state. The missing facts are only in the session transcript, and the transcript lags the hooks.

## Symptoms

- Esc on a prompt, or interrupting a turn, leaves the row ACTIVE / APPROVE / QUESTION indefinitely.
- AskUserQuestion reads as APPROVE.
- A turn that ends with a background shell still running reads as finished.
- A resumed session is pinned ACTIVE by a shell that died with the old process.

## What Didn't Work

- **Treating any transcript turn end newer than the last hook as an interrupt.** `turn_duration` is written about 0.4s *after* the Stop hook, so every normal finish looked like an interrupt for that window, and a background agent's work or approval after Stop was undone.
- **Comparing only against `last_hook` while letting subagent hooks bump it.** Subagent hooks carry `agent_id`; if they update the main agent's last-hook time, a real interrupt is hidden by a subagent's later tool call.
- **Mapping every `PermissionRequest` / `permission_prompt` notification to APPROVE.** AskUserQuestion fires those too.

## Solution

Facts about Claude Code's behavior, observed while building this (October 2026):

1. **Esc and interrupts fire no hook.** The only trace is in the transcript: the turn gets a `system` entry with `subtype: "turn_duration"` (an interrupted turn usually gets one too, after a `[Request interrupted…` user entry) and no Stop hook follows.
2. **`turn_duration` lands about 0.4s after the Stop hook** for a normal finish.
3. **AskUserQuestion fires three things:** `PreToolUse` with `tool_name: "AskUserQuestion"`, a `PermissionRequest` with the same tool name, and a `Notification` with `notification_type: "permission_prompt"`. All three must read as QUESTION.
4. **Background shells are visible only in the transcript.** A Bash `tool_result` entry whose `toolUseResult.backgroundTaskId` is set means the command went to the background (also when a foreground command was moved there on timeout). It finishes only via a `<task-notification>` carrying `<task-id>` and `<status>`, which arrives as a user entry, an `attachment` of type `queued_command`, or a `queue-operation` entry.
5. **After `claude --resume`, shells from the old process never get a notification.** A `SessionStart` with `source: "compact"` is a compaction inside the same process, so its shells are still live.
6. **Subagent hooks carry `agent_id`.**

How marsad uses them:

- `events::apply` updates `last_hook` only for hooks without `agent_id`, and sets `Session::stopped` to true on the main agent's Stop or SessionStart and false on any other main-agent hook (`src/events.rs:14-17`).
- `Session::reconcile` (`src/model.rs:414`) infers an interrupt only when the session is not `stopped`, the transcript's turn end is newer than `last_hook`, the current status was entered before that turn end, and a grace period (`INTERRUPT_GRACE_SECS`, `src/model.rs:46`) has passed for a late Stop. A status entered after the turn end (a background agent working or asking approval after Stop) is left alone.
- AskUserQuestion's `PermissionRequest` maps to QUESTION, and a `permission_prompt` notification is ignored while the status is QUESTION (`src/events.rs:75`, `src/events.rs:84`).
- The transcript reader records each background shell with its launch time (`src/transcript.rs:356`) and drops it on any of the three notification shapes (`close_task`, `src/transcript.rs:314`). `reconcile` keeps an IDLE session ACTIVE while a shell launched since `Session::run_started` is running; `run_started` is the time of the latest SessionStart whose source is not `compact` (`src/events.rs:48`).

## Why This Works

The hook stream is authoritative for what it reports and is fast; the transcript is authoritative for what hooks never report but is late. Reconciling on every refresh lets each source fill the other's gaps. The ordering rules (status entered before the turn end; `stopped` until the next main-agent hook) stop the late transcript write from overriding a newer hook-derived state, and the launch-time cutoff stops a dead process's shells from counting.

## Prevention

- When adding a status rule that reads the transcript, assume it lags the hooks by up to half a second and test the "refresh lands in between" case. `subagent_work_between_stop_and_turn_end_is_not_an_interrupt` in `src/events.rs` is the template.
- Keep subagent hooks (those with `agent_id`) out of anything that measures the main agent's activity.
- When a new tool can prompt the user, check which of PreToolUse / PermissionRequest / Notification it fires before mapping it to a status.
- Background-shell tracking depends on undocumented transcript shapes. If Claude Code changes them, `background_shells_stay_open_until_notified` in `src/transcript.rs` is where to start.

## Related Issues

- Plan: `docs/plans/2026-10-23-0657-fix-session-status-accuracy-plan.md`
