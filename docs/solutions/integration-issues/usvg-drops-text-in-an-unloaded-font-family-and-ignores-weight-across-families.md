---
title: usvg drops text whose font family isn't loaded, and font-weight can't reach a face that names its own family
date: 2026-10-24
category: integration-issues
module: share card (src/card.rs, assets/fonts)
problem_type: best_practice
component: development_workflow
severity: medium
applies_when:
  - Adding or changing text on the share card, or any SVG rendered with resvg/usvg and a custom font database
  - Bundling a font subset or a static weight file instead of a variable font
  - A card renders with text missing, or with bold text that looks regular
symptoms:
  - "A text element is missing from the rendered PNG, with no error"
  - "Text set with font-weight 600 or bold renders in the regular face"
root_cause: inadequate_documentation
resolution_type: documentation_update
retire_when: "usvg falls back to its default font family for an unknown font-family, or matches weights across families; check by rendering a text node in an unloaded family"
related_components:
  - tooling
tags: [resvg, usvg, fontdb, fonts, svg, share-card, text-rendering]
---

# usvg drops text whose font family isn't loaded, and font-weight can't reach a face that names its own family

## Context

The share card (`src/card.rs`) is an SVG string rendered by `resvg` 0.48.1 with only the bundled Inter fonts loaded, never the system's, so the PNG is identical on every machine (KTD7 in `docs/plans/2026-10-23-0717-feat-badges-and-share-plan.md`). Two static files ship: `Inter-Regular.ttf` and `Inter-SemiBold.ttf` (`src/card.rs:16-17`), loaded once in `options()` (`src/card.rs:282-291`).

The SemiBold file names its own family "Inter SemiBold" rather than declaring itself a 600 weight of "Inter". That, together with how usvg resolves families, gives two silent failures.

## Guidance

Measured with a throwaway test that rendered "Grandmaster" at 60 px through `card::tree` and read the text node's bounding box:

| `<text>` attributes | Result |
|---|---|
| `font-family="Inter"` | 367.8 px wide (Regular) |
| `font-family="Inter" font-weight="600"` | 367.8 px wide: still Regular, no warning |
| `font-family="Inter SemiBold"` | 378.1 px wide (SemiBold) |
| `font-family="Helvetica"` (not loaded) | no node at all: the text is dropped |
| `font-family="sans-serif"` | 367.8 px wide (`set_sans_serif_family` maps it to Inter) |

Rules that follow:

1. Select the heavier face by its family name. The card uses `BOLD = "Inter SemiBold"` (`src/card.rs:20`) as a `font-family`, never `font-weight`.
2. Only name families that are loaded. `usvg::Options::font_family` is the default for text with no `font-family` at all; it is not a fallback for an unknown one. Text in an unloaded family vanishes from the tree, so `node_by_id` returns None and the PNG simply lacks it.
3. Keep the guard tests:
   - `semibold_resolves_to_its_own_face` (`src/card.rs:385`) fails if the bold family stops resolving, because the two widths become equal.
   - `rank_name_is_drawn_so_the_font_loaded` (`src/card.rs:360`) fails if the rank text is dropped, because no pixels change under it.
4. When swapping in other font files, check each file's family name (for example with `fc-scan` or `fontdb::Database::faces`) before writing it into the SVG.

## Why This Matters

Neither failure raises an error. A dropped stat or a "bold" headline that renders regular looks like a layout bug, and the natural fixes (adjusting `font-weight`, or adding a system-font fallback) either do nothing or break the identical-on-every-machine guarantee. The width test catches both in milliseconds.

## When to Apply

- Adding a text element to `card::svg`, or a new font file to `assets/fonts/`.
- Rendering any other SVG with resvg and a hand-built font database.
