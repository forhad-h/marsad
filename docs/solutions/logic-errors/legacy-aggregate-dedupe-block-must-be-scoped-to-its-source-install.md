---
title: A dedupe block on a legacy aggregate must be scoped to the install that produced it
date: 2026-10-23
category: logic-errors
module: token counter (store, portable export/import)
problem_type: logic_error
component: data_model
symptoms:
  - "Importing an export from a machine that ran pre-0.4 marsad lowered the importing machine's all-time total"
  - "The importing machine's own token_days rows on dates inside the other machine's legacy range were deleted"
  - "Those days never came back on later syncs, because indexing skipped every day inside any legacy range"
  - "cargo test passed: the only test covered the copied-transcript (restore) case"
root_cause: data_integrity
resolution_type: code_fix
severity: high
tags: [token-counter, legacy-totals, import, export, dedupe, idempotent, provenance, data-loss]
---

# A dedupe block on a legacy aggregate must be scoped to the install that produced it

## Problem

Versions before 0.4 kept pruned days only as one all-time sum. On upgrade that sum became a
`legacy_totals` row keyed by the install that pruned it, with a `[since, through)` day range, and
every such range blocked the permanent day counter (`token_days`) on those days. Because the block
ignored whose range it was, importing an export from any other machine deleted the importing
machine's own, unrelated sessions on those dates and kept them from being re-indexed.

## Symptoms

- After `marsad import <file>` of an export from a machine that ran pre-0.4 marsad, the importing
  machine's all-time total went down instead of up.
- Its own `token_days` rows on dates inside the other machine's pruned range were gone, and the
  next sync didn't restore them: `write_unit` inserts a day only when `!in_legacy(legacy, day)`
  (`src/store.rs`), and the ranges it was given included the other machine's.
- Nothing warned the user; the loss showed only as a smaller number.
- `cargo test` passed. The only test imported into a machine holding a *copy* of the exporter's
  transcripts, where dropping the day is correct.

## What Didn't Work

The first design (plan KTD4) made every legacy range from any source block the day counter, both
when indexing transcripts and on import, which also deleted local `token_days` rows inside any
imported range. The reasoning held for one case: after a restore or move with transcripts copied
along, the same sessions would count once in the legacy sum and again in `token_days`. The plan, as
first written, priced the cost as a rare undercount that needed two machines that had both pruned
(KTD4 has since been corrected).

That estimate was wrong. A legacy sum is one number covering only the sessions of the machine that
pruned it, and every pre-0.4 user has one, so one pruned exporter is enough to trigger the loss.
Three independent reviewers (data-migration, correctness, adversarial) found it in code review, and
an independent validator confirmed it as data loss.

How the global block came about (session history): a doc review of the plan narrowed the block from
"everything before the legacy floor" to `[since, through)`, but never asked *whose* range it was.
Import was then written to merge legacy rows first "so their day ranges can block and clean
token_days", and the only test written was the restore case.

Alternatives the user rejected: scoping the block per session (needs session ids carried with an
aggregate that has none), and blocking only the local range (double counts copied transcripts after
a reinstall).

## Solution

A legacy row blocks the day counter only when it is this install's own, or when the user imported
that source with `marsad import --restore`, recorded as a `restored:<source>` key in `meta`.

```rust
// src/store.rs
pub(crate) const BLOCKING_LEGACY: &str = "l.since IS NOT NULL AND l.through IS NOT NULL
    AND (l.source = (SELECT value FROM meta WHERE key = 'install_id')
         OR EXISTS (SELECT 1 FROM meta WHERE key = 'restored:' || l.source))";
```

- `legacy_ranges` selects only rows matching `BLOCKING_LEGACY`. Both consumers use it: `sync_dir`
  passes it to `write_unit`, and import's `merge_table` uses it to skip incoming `token_days` rows.
- `merge_table` takes `restore: bool` (`src/portable.rs`). Only then does it mark each imported
  source restored and run the `DELETE FROM token_days` inside blocking ranges. A plain import never
  deletes local rows.
- `marsad import --restore` exposes it (`src/main.rs`), documented as "this machine's own earlier
  data, after a reinstall" in README.md.

Unchanged: `legacy_totals` merges per `source` with `MIN(since)` / `MAX(through)` / `MAX(total)`,
and `token_days` with per-column `MAX`, so importing the same file twice still adds nothing.

Remaining accepted cost: rows imported from another machine on days inside *this* install's own
pre-0.4 pruned range are dropped, because the local sum might include them and can't be split.

## Why This Works

The parts of a legacy sum are gone, so the code can't know which days' tokens it holds; it can only
know who produced it. Blocking is correct exactly where the producer's sessions are the ones the day
counter would index: the producing install itself, or a source the user declares to be the same data.
Another machine's sum covers other sessions and has no claim on this machine's days. The global block
traded double counting for silent loss of unrelated rows; scoping by provenance keeps the dedupe
where it is right and makes the broad behavior an explicit, visible choice.

Verified on the branch: `cargo test` (76 tests) and `cargo clippy --all-targets -- -D warnings`
pass. On a copy of a real database, export then import twice into an empty data dir gave the same
all-time total (+0 the second time), and `import --restore` of a machine's own export added +0.

## Prevention

- When an older format kept an aggregate that can't be split into its parts, scope any dedupe rule
  keyed on it to the provenance it covers, and make "treat this as mine" an explicit opt-in.
- Write the pair of tests before the rule: one where the blocked data is a copy (must be dropped)
  and one where different data shares the same window (must survive). In `src/portable.rs`:

  ```rust
  // B's own session on a day A pruned before 0.4 survives a plain import.
  let mut b = db("INSERT INTO token_days VALUES ('own', 'main', '2026-07-15', 0, 300, 0, 0, 300, 1);");
  import_value(&mut b, &export, false).unwrap();
  assert_eq!(all_time(&b), 1000 + 10 + 300);
  assert!(store::legacy_ranges(&b).unwrap().is_empty());
  ```

  `restore_after_reindexing_copied_transcripts_counts_legacy_days_once` is the copy half: with
  `restore = true` the copied day counts once (`1000 + 10`) and stays blocked.
- In fixtures, create a legacy row as the install's own with
  `SELECT value ... FROM meta WHERE key = 'install_id'`; a made-up source no longer blocks anything.
- When pricing a design's "rare case", check which state is the default for existing users.

## Related Issues

- Plan: `docs/plans/2026-10-23-0715-feat-token-counter-and-data-plan.md` (KTD2 legacy rows per
  install, KTD4 corrected for this fix).
- Fix on branch `feat/token-counter-and-data`, pending its PR.
