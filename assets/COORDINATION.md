# Multi-session coordination

These rules apply to this conversation because it was started with them. The user's direct instructions in the conversation override them.

Several Claude Code sessions may work in this repo at the same time. They coordinate only through one shared board file, `AGENT-BOARD.md`, at the root of the repo's main checkout. Resolve its absolute path once, at the start, from the directory you started in:

```bash
d=$(git rev-parse --git-common-dir 2>/dev/null) && d=$(cd "$d" && pwd -P) && echo "$(dirname "$d")/AGENT-BOARD.md" || echo "$PWD/AGENT-BOARD.md"
```

Use that absolute path for every read and write. Git worktrees each have their own root, so a relative path would create separate boards. Outside a git repo the board lives in the absolute directory you started in. If the file does not exist, create it from the template below. In a git repo, also add the line `AGENT-BOARD.md` to `<git-common-dir>/info/exclude` (the directory printed by the `git rev-parse` above) so it is never committed; skip this outside git. These rules apply to every task, however small.

## 1. Start (before any other work)
1. Read the board in full.
2. Register in **Agents**: name `agent-<task-in-2-words>` (add a number if the name is taken), task, branch/worktree, status `active`, started and last-seen times. Get times from `date '+%Y-%m-%d %H:%M'`, never from memory.
3. Read **Decisions**, **Messages** and **Conflicts**. Reply to anything addressed to you or `all`. Decisions are binding.
4. Work on your own branch or worktree, named after your agent name. Never commit to the default branch.
5. Subagents you spawn act under your name and your claims. They do not receive these rules, so put the board path, your agent name and the paths they may touch in their prompt.

## 2. Claims
- Claim before you create or edit any file: add the path or glob to **Claims** with your name and time. Claim narrowly and only what you need now.
- Never touch a file another agent has claimed. Message them, then do other parts of your task. While blocked, wait with `sleep 60` and re-read the board, repeating as needed (one wait per command, so it stays under the tool's timeout). After 30 minutes with no reply, log it under Conflicts and keep working elsewhere.
- Hot shared files (lockfiles, dependency manifests, schemas, migrations, CI config, generated files, root configs): announce in Messages first, keep the change minimal, commit fast, release the claim at once.
- A claim is stale when its owner's last-seen is over 2 hours old. Post a note in Messages, then you may take it.

## 3. Shared resources
Before resetting or migrating a shared database, starting a server on a default port, or regenerating shared generated files, announce it in Messages. Then wait for objections: re-check the board after each `sleep 60`, five times, and go ahead if none came.

## 4. Writing to the board
- Re-read the board immediately before every write.
- Change one row or one line per edit, with the Edit tool anchored on a unique line. Never rewrite the whole file. If the edit fails because the file changed, re-read and retry.
- Never reorder or reword another agent's entries. Delete them only as section 9 allows.
- Update your last-seen time on every board write, and at least every 30 minutes.

## 5. Rhythm
Re-check the board at the start of every step, before every commit, and before you finish.

## 6. Git
- Stage only files you claimed, by explicit path. Never `git add -A` or `git add .`.
- No repo-wide formatting, lint-fix or renames unless a Decision allows it.
- Never force-push, rebase, reset or check out another agent's branch. Never resolve conflicts in another agent's files: message them.
- Before relying on a changed convention, API, schema or shared contract, record it under **Decisions**.

## 7. Disagreements
Never undo another agent's change. Log it under **Conflicts** and message them. After one unresolved round, mark it `needs-human`, stop on that point, and continue with the rest of your task.

## 8. Finish
1. Delete your own Claims rows and your own Agents row.
2. Delete every Message you wrote, and every Message addressed to you that you have acted on.
3. Delete Conflicts you were part of that are resolved. Leave `needs-human` ones.
4. Add one **Handoff notes** entry, at most 5 lines: what is done, what is left, branch name and whether it is pushed, gotchas others need. Skip it if nothing is left and nothing is surprising.
5. Tell the user about any `needs-human` items.
6. If no other agent is active (every remaining Agents row has a last-seen over 2 hours old), run the sweep in section 9.

## 9. Keeping the board small
The board is a live scratchpad, not a log. Only Decisions and `needs-human` Conflicts are allowed to persist.
- A Message to a specific agent is deleted by its recipient after acting on it. A Message to `all` is deleted by its author when it no longer applies, or when the author finishes.
- A Handoff note is deleted by whoever picks up that work, right after reading it. Any agent may delete a Handoff note older than 7 days.
- A Decision stays until it is written into CLAUDE.md or the project docs, then the agent who wrote it there deletes it from the board. Keep each Decision to one line.
- `needs-human` Conflicts are deleted only by the human or on their instruction.
- **Sweep**, run by the last agent out (section 8, step 6) or by any agent at start when the board is over 120 lines: delete Agents rows and Claims whose last-seen is over 2 hours old, all Messages older than 24 hours, all resolved Conflicts, and Handoff notes older than 7 days. Never touch Decisions or `needs-human` items. Post one Message to `all` saying a sweep was done.

## Board template
```markdown
# AGENT BOARD
Shared by all Claude Code sessions in this repo. One-line anchored edits only. Never edit another agent's entries.

## Agents
| agent | task | branch/worktree | status | started | last seen |
|---|---|---|---|---|---|

## Claims
| path/glob | agent | since | note |
|---|---|---|---|

## Decisions
<!-- - YYYY-MM-DD HH:MM agent: decision (binding for all) -->

## Messages
<!-- - YYYY-MM-DD HH:MM from -> to: text   (to = agent name or all; reply as a new line) -->

## Conflicts
<!-- - YYYY-MM-DD HH:MM agent-a vs agent-b: issue | open / resolved / needs-human -->

## Handoff notes
<!-- - YYYY-MM-DD HH:MM agent: done / left / others must know -->
```
