---
artifact_contract: "ce-handoff/v1"
created_at: "2026-10-24T08:39:15Z"
title: "marsad Group D: badges and LinkedIn share, continue in the cloud"
summary: "Badge art rework to historical styles, then U6 (B and p views), U7 docs, simplify, code review, compound and the stacked PR for Group D"
keywords: ["marsad", "group-d", "badges", "linkedin-share", "badge-art", "girih", "zellij", "ce-work", "handoff"]
cwd: "/Users/forhad/Projects/marsad"
resume_focus: "Rework src/badge_art.rs into historical styles by tier and get the user's approval on the card, then U6, U7 and the shipping flow"
repository: "forhad-h/marsad"
repo_root_sha: "ab17d14bd0701549ccffd1c01666771a0bf37864"
branch: "feat/badges-and-share"
head: "f0bd209e88d9f3249a3d6a7144c693c5ac9dd31d"
---

# marsad Group D handoff (badges and LinkedIn share)

marsad is a Rust ratatui TUI dashboard and CLI for Claude Code sessions. The user asked for 12 improvements in 4 groups, each run through the compound-engineering flow: plan, doc review, ce-work, ce-simplify-code, ce-code-review, ce-compound, PR. Groups A-C are done as stacked PRs. This handoff covers the rest of Group D.

## User-given working rules (carry these forward)

These are the user's standing instructions for this whole effort:

- The user authorized the whole flow: don't stop for menu confirmations, but do ask real product questions.
- Subagents are allowed.
- Never run `cargo fmt`; the repo is not rustfmt-clean, so match the surrounding style by hand.
- Tests live in-module (`#[cfg(test)]`). Keep `README.md`, `CHANGELOG.md` (`## Unreleased`) and `assets/SKILL.md` in sync.
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Commit path-limited (`git commit -- <files>`).
- Never point a test or smoke check at the real `~/.claude/marsad` for writes; use `MARSAD_HOME` with a scratch dir.
- Don't send code to external review services (cursor-agent or cross-model passes); the user declined them.
- Run `gh pr merge` only if the user asks, as its own command, never chained, and without `--delete-branch`.
- On the user's Mac, the SSH remote hangs; pushes there go through `bash ~/.claude/skills/pr-ship/scripts/gh-git.sh push`. In the cloud, use whatever push path works.

## PR stack (all open, none merged)

- #20 `fix/session-status-accuracy` -> `main` (Group A)
- #21 `feat/token-counter-and-data` -> `fix/session-status-accuracy` (Group B)
- #22 `feat/agent-coordination` -> `feat/token-counter-and-data` (Group C)
- Group D will be a PR from `feat/badges-and-share` -> `feat/agent-coordination`, squash-merge style like #20-#22 (sections Summary, Changes, Testing, Risk & rollback; body ends with "🤖 Generated with [Claude Code](https://claude.com/claude-code)").

## Authoritative references

- Plan: `docs/plans/2026-10-23-0717-feat-badges-and-share-plan.md`. It is the source of truth: Product Contract R1-R13, AE1-AE4, session-settled Key Decisions, KTD1-KTD13, units U1-U7, Verification Contract, Definition of Done. It went through ce-doc-review (17 fixes applied) and is committed in `1c1e376`.
- Commits on the branch since `feat/agent-coordination`, one per unit: `git log feat/agent-coordination..feat/badges-and-share`.
- Learnings: `docs/solutions/integration-issues/claude-code-system-prompt-snapshot-pins-coordination-protocol-to-new-conversations.md` (run each `claude -p` as a fresh conversation, never resume).

## Current state by unit

| Unit | State | Where |
|---|---|---|
| U1 permanent stores (`skills_seen`, `badge_best`, schema v2) | done | `7d5d1b9`, `src/store.rs`, `src/portable.rs` |
| U2 badge model + `marsad badges` | done | `a5e2c4b`, `src/badges.rs` |
| U3 badge geometry | done, but the user wants it reworked (see below) | `31b06cb`, `src/badge_art.rs` |
| U4 card renderer | done and redesigned once; layout approved in substance, badge art not | `89f6cb2`, `c229a8e`, `src/card.rs`, `assets/fonts/` |
| U5 share core + `marsad share --print` | done | `c42d5d8`, `f0bd209`, `src/share.rs` |
| U6 TUI: `B` badge view, footer announcement, `p` share view | not started | plan `### U6.` |
| U7 docs | not started | plan `### U7.` |

At `f0bd209`: `cargo test --locked` passes 172 tests (2 ignored sample generators), and `cargo clippy --all-targets -- -D warnings` is clean.

## Decisions made with the user during this session

Recorded in the plan's Key Decisions (session-settled): craft tier names Novice..Grandmaster; all tokens count, cache reads included; `p` opens share from the list and `B` opens the badge view; titles are sent to `claude -p` with project/folder names redacted; edit the line inline or the whole post with `E`; the card goes to the Downloads folder.

Made later, after the plan was written (not yet in the plan; mention them in the PR):

- **Card content (user's choice):** a light ivory card, with the badge as the large hero; Islamic ornament on the whole card (a girih frame band and a faint star lattice); no "today" figure on the card; the headline "AI-ASSISTED ENGINEERING · RANK n OF 7"; the earned achievements as bronze/silver/gold medals; and the user's name, from `git config user.name`. All of this is implemented in `src/card.rs`, and the post text still includes today, per R8.
- **Card background (user's verdict):** "background looks great anyway". Keep the ivory ground, frame and lattice as they are.
- **Badge art (user's verdict):** the current badges "look like astronomical things". The user wants historical Islamic design styles ("Islam has very rich historical design and concept"), and chose **mix by tier**:
  - Tiers 0-1 (Novice, Apprentice): Mamluk / Seljuk carved roundel. Deep-relief geometric interlace in one material (ivory or sandstone, then cedar), read through light and shadow, not color.
  - Tiers 2-4 (Practitioner, Specialist, Expert): Moorish zellij (Alhambra / Fez). An 8-fold khatam rosette in a tiled field of small cut-tile pieces in cobalt, turquoise, green, ochre, black and white, with broad interlaced bands that weave over and under.
  - Tiers 5-6 (Master, Grandmaster): Persian girih (Isfahan / Timurid). 10-fold geometry with decagon rosettes and long interlaced strapwork, turquoise and lapis with gold; Grandmaster the richest.
  - Still bound by R7: no figurative imagery, no text, no religious symbols or calligraphy. Avoid the Rub el Hizb mark.
- **My call (accepted by me, not raised with the user):** KTD9 redaction ignores a fixed list of generic container folder names (users, home, projects, code, src, dev, ...) so ordinary words don't trip the post-check; project names themselves are still redacted. See `src/share.rs`.

## What is next

1. **Badge art rework (redo U3's look).** A worker started this and hit the usage limit; its work was lost with the scratch dir, so it starts from `f0bd209`.
   - Keep the API `src/card.rs` uses: `svg(tier, cx, cy, r)`, `badge(tier).palette`, `Badge::segments()` for the future Canvas, `TIERS`, and `hankin_star` (the card uses it for medals and the frame).
   - Keep the card tests passing.
   - Render samples with the ignored tests:
     - `BADGE_PREVIEW=<file>` with the `badge_art` preview test.
     - `CARD_SAMPLES=<dir> cargo test card::tests::sample -- --include-ignored`. It takes about 35 s in debug. `card-real.png` is Expert tier 4, named "Forhad Hosain", with 386.2M all time, 198M, 340M, 11.3M/day, and the medals gold Busiest day 212.5M, silver Skills 13, silver Subagents 102.
   - Iterate until the badges read as those historical traditions.
2. **Card approval: the plan's stop condition.** Show the user the new card PNGs, all seven tiers, and get approval before U6. This is a real product question; ask it.
3. **U6** per the plan (`### U6.`). The share-core API it needs is in `src/share.rs`: `snapshot`, `draft` (with a cancel `AtomicBool`), `post`, `template_line`, `linkedin_url`, `default_card_path`, `confirm` / `save_card` / `open_linkedin` / `reveal`, and `Desktop::current()`.
   - The clipboard step must run on the main thread (OSC 52 interleaves with drawing).
   - Remove the `#[allow(dead_code)] // ... (U6)` markers as items get used: `src/share.rs`, `src/badges.rs` (`pending_announcement`, `mark_announced`), `src/badge_art.rs` and `src/card.rs` module-level allows.
4. **U7 docs.** The README must explain what leaves the machine, including that a client named only in a title is caught by the user's review, not by redaction.
5. **The shipping flow:**
   - ce-simplify-code on `git diff feat/agent-coordination...HEAD`.
   - ce-code-review with `base:feat/agent-coordination`, applying fixes.
   - ce-compound, one learning. Candidates:
     - usvg drops a text node whose font family isn't loaded, and Inter's SemiBold subset names its family "Inter SemiBold", so `font-weight` alone falls back silently.
     - `claude -p --safe-mode --no-session-persistence` is the isolation that keeps a helper call out of marsad's own session list, and `--bare` breaks subscription logins.
   - Push and open the PR.

## Verification that needs the user's Mac (not doable in the cloud)

The Verification Contract's smoke checks:

- **Real `claude` run:** `marsad share --print` with the real `claude`, checking that no new session appears in the dashboard. U5 already ran this once successfully on the Mac; it should be re-run after U6.
- **Dashboard smoke test:** the pyte pseudo-terminal run of `B`, `p`, cancel and confirm into a scratch Downloads dir, saving and restoring the clipboard with `pbpaste`/`pbcopy`.

Leave these for a local session and say so in the PR's Testing section rather than claiming them.

## Pitfalls already hit

- **SemiBold font family:** the subset's family is "Inter SemiBold". An unknown family drops the text silently; a test guards this (`semibold_resolves_to_its_own_face`).
- **`fmt_count`:** in `src/card.rs` it drops a trailing ".0" ("198M"), and the post uses it too.
- **No `--max-turns`:** `claude --help` 2.1.309/2.1.310 has no `--max-turns`, so the flag set doesn't use it.
- **Migration is stepwise:** re-running the v1 copy on a v1 database would double count legacy-range days. A test guards this.
