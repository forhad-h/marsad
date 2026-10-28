# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

The `web` value covers the planned landing page. marsad itself is a terminal app (a ratatui TUI plus a CLI) that also generates a PNG share card. Neither of those is web, and both are design surfaces too (see Operating Context).

## Stack

Landing page: static HTML/CSS, hand-built in the repo and deployed to GitHub Pages, with no build step (the user's choice). The product itself is Rust: ratatui 0.30 for the TUI, rusqlite for storage, and resvg with bundled Inter fonts for the share card.

## Users

Every audience below is confirmed. One person is often several of them at once.

- **Developers running several Claude Code sessions in parallel.** They need to see at a glance which sessions are waiting on them (approval or a question), what each one is doing, and what it costs in tokens.
- **Usage-conscious users.** They watch context size, the 5-hour and 7-day usage limits, and token spend by skill, project, model and subagent.
- **Career showcasers.** They want to prove their AI fluency to recruiters and clients with a rank, achievements and a LinkedIn share card earned from real usage.
- **Multi-agent coordinators.** They run several agents in the same repo and use marsad's coordination rules and agent board so the agents don't collide.

## Product Purpose

marsad is a live terminal dashboard for every Claude Code session on the machine: what each one is doing, which need the user, which skills and subagents they ran, and how many tokens they used. It also:
- keeps a permanent token history and the reports built on it;
- ships coordination rules and a board for parallel sessions;
- turns real usage into a rank, achievements and a shareable card.

Success is when the user stops tab-hunting for sessions that need them, understands their usage without leaving the terminal, and can show credible evidence of their AI work.

## Positioning

The user stands behind these four claims publicly:

- **Live, hook-accurate status.** Real-time APPROVE, QUESTION, ACTIVE and IDLE states come from Claude Code hooks and transcripts, not from polling or guesses.
- **Local-only, no account.** Everything stays on the machine, in SQLite under `~/.claude/marsad`. Nothing is uploaded except what the user chooses to share. The share flow sends a redacted summary to `claude -p` and shows the post for review first.
- **Permanent history.** A per-day counter that retention never prunes, so all-time totals, streaks and badges survive.
- **Coordination built in.** A bundled protocol and an agent board keep parallel sessions from editing the same files.

## Operating Context

- **The terminal is where the product is used.** The ratatui dashboard runs in the user's terminal next to their Claude Code sessions, often in tmux and sometimes over SSH. Key-driven, no mouse. Status must be readable at a glance.
- **The CLI is for scripts and agents.** Commands such as `marsad total`, `report`, `context`, `badges`, `share --print`, `rules`, `export` and `import` are used by scripts and by Claude itself through the bundled `marsad-usage` skill (`assets/SKILL.md`).
- **The share card is seen in a LinkedIn feed.** It is a 1200×627 PNG with Islamic geometric badge art by tier, the user's name, the rank, token stats and earned achievement medals. Recruiters and business owners see it there.
- **GitHub is the front door today.** The README on GitHub, plus GitHub Releases for installers. A static landing page is planned.

## Capabilities and Constraints

- **Install:** `marsad install` / `uninstall` manage hooks, the status line, the skill and the rules files in `~/.claude/settings.json`. Running sessions pick them up after a restart.
- **Dashboard keys:** sessions list, history, subagents, usage view, agent board (`b`), copy a launch command with the chosen rules (`c`), alerts, sort. Badges and share are one view (`p`): the post beside the badge, with share to LinkedIn, copy text and save image.
- **Platforms:** macOS, Linux and Windows. Installers ship through GitHub Releases.
- **Ranks:** seven tiers, Novice to Grandmaster, from all-time tokens (cache reads included). Achievements are bronze, silver and gold for streak, distinct skills, subagents and busiest day.
- **Terms:** canonical domain terms live in `CONCEPTS.md`, for example session status, day counter, coordination protocol, agent board, claim.

## Brand Commitments

- **Lowercase name:** the name is always written lowercase, `marsad`.
- **The name's meaning:** `marsad` (مرصد) is Arabic for observatory or watchtower, and the brand may draw on that meaning.
- **Islamic geometric heritage:** historical Islamic geometric art is part of marsad's identity beyond the share card. The user rejected badge art that read as "astronomical". They want recognisably historical styles, mixed by rank tier:
  - Mamluk/Seljuk carved roundels for the lower tiers;
  - Moorish zellij for the middle tiers;
  - Persian girih for the top tiers.
  - The card's ivory ground, girih frame and faint star lattice were approved ("background looks great").
- **What ornament may not include:** no figurative imagery, no text or calligraphy, and no religious symbols. That includes the Rub el Hizb mark.
- **Everything else is open:** the user has made no other brand commitments. The palette, typography and voice of the TUI and the site are not decided.

## Evidence on Hand

- **The README:** it has an ASCII mockup of the dashboard.
- **The card and badges:** the code that renders them is in `src/card.rs` and `src/badge_art.rs`. The fonts are Inter, under the OFL, in `assets/fonts/`.
- **Real usage numbers:** the user's own figures, e.g. about 386M all-time tokens at rank Expert.
- **What does not exist:** testimonials, user counts, press, logos or benchmarks. Do not invent them.

## Product Principles

- **The user's attention is the scarce resource:** surface what needs them first, then everything else.
- **Truth over impression:** statuses, totals and badges come from real hook and transcript data and never go backwards. Nothing claims more than marsad knows (for example, nothing says "copied" unless the copy is confirmed).
- **Local and private by default:** nothing leaves the machine without the user seeing it first.
- **Earned, not decorative:** ranks and achievements reward real breadth of work, shown with heritage craft rather than generic gamification.

## Accessibility & Inclusion

- **The TUI must work without color:** statuses, levels and highlights are always readable as text or symbols too, for no-color terminals and color-blind users.
- **The landing page meets WCAG 2.2 AA:** contrast and keyboard use.
