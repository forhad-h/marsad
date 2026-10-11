use crate::app::{App, View};
use crate::update::Notice;
use crate::usage::{Pane, Usage};
use crate::model::{ContextInfo, DAY_FMT, Level, fmt_tokens, MAIN_AGENT, RateWindow, Session, SortKey, Status, TokenUsage, Via};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Cell, Paragraph, Row, Scrollbar, ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
};

const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];
const ACCENT: Color = Color::Cyan;

const HIGHLIGHT: Style = Style::new().bg(Color::Indexed(236)).add_modifier(Modifier::BOLD);

/// Background for a row waiting on a permission prompt: a faint amber wash that
/// slowly pulses, so the row stands out even when its text is covered, while
/// staying dark enough that yellow and gray text on it keep their contrast.
/// `lift` brightens it for the selected row.
fn permission_bg(tick: u64, lift: u8) -> Color {
    const PERIOD: u64 = 12; // ticks are ~250ms, so one pulse is ~3s
    let phase = tick % PERIOD;
    let t = (phase.min(PERIOD - phase) * 2) as f32 / PERIOD as f32; // 0 → 1 → 0
    let mix = |lo: u8, hi: u8| (lo as f32 + (hi - lo) as f32 * t).round() as u8 + lift;
    Color::Rgb(mix(30, 58), mix(24, 44), mix(8, 12))
}

/// A table with the dashboard's standard spacing and selection highlight.
fn styled_table<'a>(rows: impl IntoIterator<Item = Row<'a>>, widths: &[Constraint], block: Block<'a>) -> Table<'a> {
    Table::new(rows, widths.to_vec()).column_spacing(1).row_highlight_style(HIGHLIGHT).block(block)
}

fn right_cell(text: impl Into<String>) -> Cell<'static> {
    Cell::from(Line::from(text.into()).alignment(Alignment::Right))
}

fn via_span(via: Via) -> Span<'static> {
    match via {
        Via::Tool => dim("tool"),
        Via::Command => Span::styled("/cmd", Style::new().fg(ACCENT)),
    }
}

const RUN_WIDTHS: [Constraint; 9] = [
    Constraint::Length(3),
    Constraint::Length(12),
    Constraint::Length(18),
    Constraint::Fill(1),
    Constraint::Length(4),
    Constraint::Length(5),
    Constraint::Length(7),
    Constraint::Length(8),
    Constraint::Length(10),
];
const RUN_HEADER: [&str; 9] = ["  #", "TIME", "AGENT", "SKILL", "VIA", "TURNS", "OUT", "TOKENS", ""];

/// One row of a skill-run table (live history and the usage drill-down share the layout).
#[allow(clippy::too_many_arguments)]
fn run_row(i: usize, at: Option<&str>, agent: &str, skill: &str, via: Via, turns: u32, output: u64, total: u64, max: u64) -> Row<'static> {
    let agent_style = if agent == MAIN_AGENT { Style::new() } else { Style::new().fg(ACCENT) };
    Row::new(vec![
        Cell::from(dim(format!("{:>3}", i + 1))),
        Cell::from(Span::raw(fmt_local(at))),
        Cell::from(Span::styled(agent.to_string(), agent_style)),
        Cell::from(Span::styled(skill.to_string(), Style::new().fg(Color::Magenta).bold())),
        Cell::from(via_span(via)),
        right_cell(turns.to_string()),
        right_cell(fmt_tokens(output)),
        right_cell(format!("≈{}", fmt_tokens(total))),
        Cell::from(Span::styled(bar(weight(total as i64, max as i64), 10), Style::new().fg(Color::Magenta))),
    ])
}

fn runs_table(rows: Vec<Row<'static>>, block: Block<'static>) -> Table<'static> {
    styled_table(rows, &RUN_WIDTHS, block).header(Row::new(RUN_HEADER).style(Style::new().dark_gray()))
}

fn status_style(s: Status) -> (Style, &'static str) {
    match s {
        Status::Permission => (Style::new().fg(Color::Yellow).bold(), "PERMISSION"),
        Status::Question => (Style::new().fg(Color::Magenta).bold(), "QUESTION"),
        Status::Error => (Style::new().fg(Color::Red).bold(), "ERROR"),
        Status::Active => (Style::new().fg(Color::Green), "ACTIVE"),
        Status::Idle => (Style::new().fg(Color::Blue), "IDLE"),
        Status::Ended => (Style::new().fg(Color::DarkGray), "ENDED"),
    }
}

fn status_glyph(s: Status, tick: u64) -> &'static str {
    match s {
        Status::Permission => "●",
        Status::Question => "?",
        Status::Error => "✗",
        Status::Active => SPINNER[(tick / 2 % 4) as usize],
        Status::Idle => "○",
        Status::Ended => "·",
    }
}

fn fmt_age(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    match s {
        0..5 => "now".into(),
        5..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}

/// Time left until a unix timestamp, e.g. "2h13m" or "3d4h".
fn fmt_until(ts: f64) -> String {
    let s = (ts - crate::now()).max(0.0) as u64;
    match s {
        0..3600 => format!("{}m", s / 60),
        3600..86400 => format!("{}h{:02}m", s / 3600, s % 3600 / 60),
        _ => format!("{}d{}h", s / 86400, s % 86400 / 3600),
    }
}

/// Local wall-clock time for an ISO-8601 transcript timestamp.
fn fmt_local(iso: Option<&str>) -> String {
    let Some(t) = iso.and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()) else { return "—".into() };
    let t = t.with_timezone(&chrono::Local);
    if t.date_naive() == chrono::Local::now().date_naive() {
        t.format("%H:%M:%S").to_string()
    } else {
        t.format("%b %d %H:%M").to_string()
    }
}

fn short_path(p: &str) -> String {
    match dirs::home_dir().and_then(|h| p.strip_prefix(&*h.to_string_lossy()).map(str::to_string)) {
        Some(rest) => format!("~{rest}"),
        None => p.to_string(),
    }
}

/// Green while there is room, yellow past 60%, red past 85%.
fn ratio_color(r: f64) -> Color {
    match Level::of(r) {
        Level::Critical => Color::Red,
        Level::Warn => Color::Yellow,
        Level::Ok => Color::Green,
    }
}

fn bar(ratio: f64, width: usize) -> String {
    let filled = ((ratio.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    "▰".repeat(filled) + &"▱".repeat(width - filled)
}

/// `▰▰▰▱▱▱ 42% · 412k left before auto-compact` with the whole gauge colored by fill.
fn context_spans(ctx: &ContextInfo, width: usize, long: bool) -> Vec<Span<'static>> {
    let r = ctx.used_ratio();
    let style = Style::new().fg(ratio_color(r));
    let approx = if ctx.estimated { "≈" } else { "" };
    let mut spans = vec![
        Span::styled(bar(r, width), style),
        Span::styled(format!(" {approx}{:.0}%", r * 100.0), style.bold()),
    ];
    if long {
        spans.push(Span::raw(format!("   {} left", fmt_tokens(ctx.left()))));
        spans.push(Span::styled(
            format!(" before auto-compact at {} · window {}", fmt_tokens(ctx.limit), fmt_tokens(ctx.window)),
            Style::new().dark_gray(),
        ));
    }
    spans
}

fn rate_spans(label: &str, w: RateWindow) -> Vec<Span<'static>> {
    let style = Style::new().fg(ratio_color(w.used_pct / 100.0));
    let mut spans = vec![Span::raw(format!("{label} ")), Span::styled(format!("{:.0}%", w.used_pct), style.bold())];
    if let Some(reset) = w.resets_at {
        spans.push(Span::styled(format!(" ↻{}", fmt_until(reset)), Style::new().dark_gray()));
    }
    spans
}

pub fn draw(f: &mut Frame, app: &App) {
    let visible = app.visible();
    let [header, body, footer] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(4), Constraint::Length(1)]).areas(f.area());
    draw_header(f, header, app, &visible);

    let keys: Vec<(&str, String)> = match (app.view, app.selected_session()) {
        (View::Usage, _) if app.usage.drill.is_some() => {
            draw_drill(f, body, &app.usage);
            vec![("↑↓", "scroll".into()), ("esc", "back".into()), ("q", "quit".into())]
        }
        (View::Usage, _) => {
            draw_usage(f, body, &app.usage);
            vec![
                ("tab", "pane".into()),
                ("↑↓", "move".into()),
                ("enter", "open session".into()),
                ("r", "resync".into()),
                ("esc", "back".into()),
                ("q", "quit".into()),
            ]
        }
        (View::History, Some(s)) => {
            draw_history(f, body, app, s);
            vec![
                ("↑↓", "scroll".into()),
                ("g/G", "top/bottom".into()),
                ("tab", "subagents".into()),
                ("esc", "back".into()),
                ("q", "quit".into()),
            ]
        }
        (View::Agents, Some(s)) => {
            draw_agents(f, body, app, s);
            vec![
                ("↑↓", "scroll".into()),
                ("g/G", "top/bottom".into()),
                ("tab", "skill history".into()),
                ("esc", "back".into()),
                ("q", "quit".into()),
            ]
        }
        _ => {
            draw_list(f, body, app, &visible);
            let ended = if app.show_ended { "hide ended" } else { "show ended" };
            vec![
                ("↑↓", "select".into()),
                ("enter", "history".into()),
                ("tab", "subagents".into()),
                ("u", "30-day usage".into()),
                ("a", ended.into()),
                ("s/S", format!("sort: {}", app.sort.label())),
                ("q", "quit".into()),
            ]
        }
    };
    let mut spans = vec![Span::raw(" ")];
    for (k, label) in keys {
        spans.push(k.to_string().bold());
        spans.push(format!(" {label}  ").dark_gray());
    }
    let footer = match &app.update {
        Some(notice) => {
            let text = match notice {
                Notice::Available(v) => format!("v{v} available · run `marsad update` "),
                Notice::Installed(v) => format!("updated to v{v} · restart marsad "),
            };
            let [keys, note] =
                Layout::horizontal([Constraint::Min(0), Constraint::Length(text.chars().count() as u16)]).areas(footer);
            f.render_widget(Paragraph::new(Span::styled(text, Style::new().fg(Color::Yellow))), note);
            keys
        }
        None => footer,
    };
    f.render_widget(Paragraph::new(Line::from(spans)), footer);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App, visible: &[&Session]) {
    let waiting = visible.iter().filter(|s| s.status.needs_user()).count();
    let active = visible.iter().filter(|s| s.status == Status::Active).count();
    let total: u64 = visible.iter().map(|s| s.total_tokens().total()).sum();
    let mut left = vec![
        Span::styled(" marsad ", Style::new().fg(Color::Black).bg(ACCENT).bold()),
        Span::raw(format!("  {} sessions", visible.len())),
        Span::styled(format!(" · {active} active"), Style::new().fg(Color::Green)),
    ];
    if waiting > 0 {
        left.push(Span::styled(format!(" · {waiting} need you"), Style::new().fg(Color::Yellow).bold()));
    }
    f.render_widget(Paragraph::new(Line::from(left)), area);

    let mut right = Vec::new();
    if let Some((five, seven)) = app.rate_limits() {
        for (label, w) in [("5h", five), ("7d", seven)] {
            if let Some(w) = w {
                right.extend(rate_spans(label, w));
                right.push(Span::styled("  ·  ", Style::new().dark_gray()));
            }
        }
    }
    right.extend([Span::raw("Σ "), Span::styled(fmt_tokens(total), Style::new().bold()), Span::raw(" tokens ")]);
    f.render_widget(Paragraph::new(Line::from(right)).alignment(Alignment::Right), area);
}

fn draw_list(f: &mut Frame, area: Rect, app: &App, visible: &[&Session]) {
    let selected = app.selected_session();
    let details_height = if selected.is_some() { 9 } else { 0 };
    let [table, details] =
        Layout::vertical([Constraint::Min(4), Constraint::Length(details_height)]).areas(area);
    if visible.is_empty() {
        draw_empty(f, table, app);
    } else {
        draw_table(f, table, app, visible);
    }
    if let Some(s) = selected {
        draw_details(f, details, s, app);
    }
}

fn draw_empty(f: &mut Frame, area: Rect, app: &App) {
    let msg = if app.hooks_installed {
        vec![
            Line::from("No sessions yet.".bold()),
            Line::from(""),
            Line::from("Start (or restart) a Claude Code session and it will appear here.".dark_gray()),
        ]
    } else {
        vec![
            Line::from("marsad hooks are not installed.".bold()),
            Line::from(""),
            Line::from(vec!["Run ".dark_gray(), "marsad install".fg(ACCENT).bold(), ", then restart your Claude Code sessions.".dark_gray()]),
        ]
    };
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().dark_gray());
    let inner = block.inner(area);
    f.render_widget(block, area);
    let [_, mid, _] = Layout::vertical([Constraint::Fill(1), Constraint::Length(3), Constraint::Fill(1)]).areas(inner);
    f.render_widget(Paragraph::new(msg).alignment(Alignment::Center), mid);
}

fn draw_table(f: &mut Frame, area: Rect, app: &App, visible: &[&Session]) {
    let now = crate::now();
    let rows = visible.iter().map(|s| {
        let (style, label) = status_style(s.status);
        let activity = if !s.detail.is_empty() {
            Span::raw(s.detail.clone())
        } else {
            let title = s.title.clone().or_else(|| s.stats.title.clone()).unwrap_or_default();
            Span::styled(title, Style::new().dark_gray())
        };
        let running = s.subagents.values().filter(|a| a.running).count();
        let agents = if running > 0 { format!("+{running}") } else { String::new() };
        let context = match s.context(app.compact_window) {
            Some(ctx) => {
                let style = Style::new().fg(ratio_color(ctx.used_ratio()));
                Line::from(vec![
                    Span::styled(bar(ctx.used_ratio(), 5), style),
                    Span::styled(format!(" {:>6}", fmt_tokens(ctx.left())), style),
                ])
            }
            None => Line::from(""),
        };
        let row = Row::new(vec![
            Cell::from(Line::from(vec![
                Span::styled(format!(" {} ", status_glyph(s.status, app.tick)), style),
                Span::styled(label, style),
            ])),
            Cell::from(Span::styled(s.project(), Style::new().bold())),
            Cell::from(Span::styled(agents, Style::new().fg(ACCENT))),
            Cell::from(Line::from(activity)),
            Cell::from(context),
            right_cell(fmt_age(now - s.status_since)),
            right_cell(fmt_tokens(s.total_tokens().total())),
        ]);
        match s.status {
            Status::Ended => row.style(Style::new().dark_gray()),
            Status::Permission => row.style(Style::new().bg(permission_bg(app.tick, 0))),
            _ => row,
        }
    });
    let widths = [
        Constraint::Length(14),
        Constraint::Length(20),
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(13),
        Constraint::Length(5),
        Constraint::Length(8),
    ];
    let sorted_col = match app.sort {
        SortKey::Status => 0,
        SortKey::Project => 1,
        SortKey::Context => 4,
        SortKey::Age => 5,
        SortKey::Tokens => 6,
    };
    let arrow = if app.sort_reversed { "▴" } else { "▾" };
    let header = Row::new(["  STATUS", "PROJECT", "", "ACTIVITY", "CONTEXT LEFT", "AGE", "TOKENS"].into_iter().enumerate().map(
        |(i, h)| {
            if i == sorted_col { Cell::from(Span::styled(format!("{h}{arrow}"), Style::new().fg(ACCENT))) } else { Cell::from(h) }
        },
    ))
    .style(Style::new().dark_gray());
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dark_gray())
        .title(Line::from(" Sessions ").fg(ACCENT));
    let mut table = styled_table(rows, &widths, block).header(header);
    // Keep the warning wash on a selected permission row instead of the plain gray highlight.
    if app.selected_session().is_some_and(|s| s.status == Status::Permission) {
        table = table.row_highlight_style(HIGHLIGHT.bg(permission_bg(app.tick, 18)));
    }
    let mut state = TableState::default().with_selected(app.selected_index());
    f.render_stateful_widget(table, area, &mut state);
}

fn usage_spans(u: &TokenUsage) -> Vec<Span<'static>> {
    let kv = |k: &'static str, v: u64| [Span::styled(format!("{k} "), Style::new().dark_gray()), Span::raw(format!("{}   ", fmt_tokens(v)))];
    let mut spans = Vec::new();
    spans.extend(kv("in", u.input));
    spans.extend(kv("out", u.output));
    spans.extend(kv("cache read", u.cache_read));
    spans.extend(kv("cache write", u.cache_write));
    spans.push(Span::styled("total ", Style::new().dark_gray()));
    spans.push(Span::styled(fmt_tokens(u.total()), Style::new().bold()));
    spans
}

fn label(t: &'static str) -> Span<'static> {
    Span::styled(format!("{t:<9}"), Style::new().fg(ACCENT))
}

fn dim(t: impl Into<String>) -> Span<'static> {
    Span::styled(t.into(), Style::new().dark_gray())
}

fn session_heading(s: &Session) -> Line<'static> {
    let mut heading = vec![Span::styled(format!(" {} ", s.project()), Style::new().bold())];
    if let Some(t) = s.title.clone().or_else(|| s.stats.title.clone()) {
        heading.push(dim(format!("· {t} ")));
    }
    Line::from(heading)
}

fn context_line(s: &Session, app: &App) -> Line<'static> {
    let mut spans = vec![label("context")];
    match s.context(app.compact_window) {
        Some(ctx) => spans.extend(context_spans(&ctx, 20, true)),
        None => spans.push(dim("no API calls yet")),
    }
    Line::from(spans)
}

fn draw_details(f: &mut Frame, area: Rect, s: &Session, app: &App) {
    let mut meta = vec![label("session"), Span::raw(short_path(&s.cwd))];
    let model = s.snapshot.as_ref().and_then(|x| x.model.clone()).or(s.model.clone());
    if let Some(m) = model {
        meta.extend([dim("   model "), Span::raw(m)]);
    }
    if let Some(m) = s.permission_mode.as_ref() {
        meta.extend([dim("   mode "), Span::raw(m.clone())]);
    }
    meta.extend([dim("   id "), dim(s.id.chars().take(8).collect::<String>())]);

    let mut tokens = vec![label("tokens")];
    tokens.extend(usage_spans(&s.total_tokens()));

    let mut agents = vec![label("agents"), Span::raw("main "), Span::styled(fmt_tokens(s.stats.tokens.total()), Style::new().bold())];
    // Grouped by type so sessions with many subagents still fit on one line.
    let mut by_type: std::collections::BTreeMap<&str, (usize, usize, u64)> = Default::default();
    for a in s.subagents.values() {
        let e = by_type.entry(a.agent_type.as_str()).or_default();
        e.0 += 1;
        e.1 += usize::from(a.running);
        e.2 += a.stats.tokens.total();
    }
    for (agent_type, (count, running, tokens)) in by_type {
        let (mark, mark_style) = if running > 0 {
            (format!("{} {running} running", SPINNER[(app.tick / 2 % 4) as usize]), Style::new().fg(Color::Green))
        } else {
            ("✓".to_string(), Style::new().dark_gray())
        };
        let count = if count > 1 { format!(" ×{count}") } else { String::new() };
        agents.extend([
            dim("  ·  "),
            Span::raw(format!("{agent_type}{count} ")),
            Span::styled(format!("{mark} "), mark_style),
            Span::raw(fmt_tokens(tokens)),
        ]);
    }

    let mut skills_list: Vec<_> = s.all_skills().into_iter().collect();
    skills_list.sort_by(|a, b| b.1.tokens.total().cmp(&a.1.tokens.total()).then(b.1.calls.cmp(&a.1.calls)));
    let mut skills = vec![label("skills")];
    if skills_list.is_empty() {
        skills.push(dim("none yet"));
    }
    for (i, (name, st)) in skills_list.iter().enumerate() {
        if i > 0 {
            skills.push(dim("  ·  "));
        }
        skills.extend([
            Span::styled(name.clone(), Style::new().fg(Color::Magenta)),
            dim(format!(" ×{} ", st.calls)),
            Span::raw(format!("≈{}", fmt_tokens(st.tokens.total()))),
        ]);
    }

    let mut lines = vec![Line::from(meta), context_line(s, app), Line::from(tokens), Line::from(agents), Line::from(skills)];
    if !s.detail.is_empty() {
        let width = area.width.saturating_sub(12) as usize;
        lines.insert(1, Line::from(vec![label("now"), Span::raw(crate::hook::truncate(&s.detail, width))]));
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dark_gray())
        .title(session_heading(s))
        .title_bottom(
            Line::from(vec![dim(" enter "), Span::raw("skill history "), dim(" tab "), Span::raw("subagents ")])
                .alignment(Alignment::Right),
        );
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), area);
}

/// Full-screen, scrollable timeline of every skill invocation in a session.
fn draw_history(f: &mut Frame, area: Rect, app: &App, s: &Session) {
    let runs = s.skill_runs();
    let in_skills: u64 = runs.iter().map(|(_, r)| r.tokens.total()).sum();
    let total = s.total_tokens();
    let [summary, list] = Layout::vertical([Constraint::Length(5), Constraint::Min(3)]).areas(area);

    let share = if total.total() > 0 { in_skills as f64 / total.total() as f64 * 100.0 } else { 0.0 };
    let mut tokens = vec![label("tokens")];
    tokens.extend(usage_spans(&total));
    let lines = vec![
        context_line(s, app),
        Line::from(tokens),
        Line::from(vec![
            label("skills"),
            Span::styled(
                format!("{} invocation{}", runs.len(), if runs.len() == 1 { "" } else { "s" }),
                Style::new().bold(),
            ),
            dim("   ≈"),
            Span::raw(fmt_tokens(in_skills)),
            dim(format!(" tokens inside skill windows ({share:.0}% of session)")),
        ]),
    ];
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dark_gray())
        .title(session_heading(s));
    f.render_widget(Paragraph::new(lines).block(block), summary);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dark_gray())
        .title(Line::from(" Skill history ").fg(ACCENT))
        .title(Line::from(dim(" ≈ tokens of the turns run inside each skill's window ")).alignment(Alignment::Right));
    if runs.is_empty() {
        let msg = Paragraph::new(dim("No skills invoked in this session yet.")).alignment(Alignment::Center).block(block);
        f.render_widget(msg, list);
        return;
    }

    let max = runs.iter().map(|(_, r)| r.tokens.total()).max().unwrap_or(0);
    let rows = runs
        .iter()
        .enumerate()
        .map(|(i, (agent, r))| {
            run_row(i, r.at.as_deref(), agent, &r.name, r.via, r.turns, r.tokens.output, r.tokens.total(), max)
        })
        .collect();
    let table = runs_table(rows, block);
    render_scrolled(f, table, list, runs.len(), app.history_scroll);
}

/// Renders a bordered table with a header, selecting row `scroll`, plus a scrollbar
/// when the rows overflow.
fn render_scrolled(f: &mut Frame, table: Table<'static>, area: Rect, len: usize, scroll: usize) {
    let scroll = scroll.min(len.saturating_sub(1));
    let mut state = TableState::default().with_selected(Some(scroll));
    f.render_stateful_widget(table, area, &mut state);

    // Borders and header take three rows.
    if len <= area.height.saturating_sub(3) as usize {
        return;
    }
    let mut sb = ScrollbarState::new(len).position(scroll);
    f.render_stateful_widget(
        Scrollbar::new(ScrollbarOrientation::VerticalRight).style(Style::new().dark_gray()),
        area.inner(Margin { vertical: 1, horizontal: 0 }),
        &mut sb,
    );
}

const AGENT_WIDTHS: [Constraint; 10] = [
    Constraint::Length(3),
    Constraint::Length(12),
    Constraint::Length(16),
    Constraint::Fill(2),
    Constraint::Length(18),
    Constraint::Fill(1),
    Constraint::Length(9),
    Constraint::Length(7),
    Constraint::Length(13),
    Constraint::Length(7),
];
const AGENT_HEADER: [&str; 10] =
    ["  #", "STARTED", "AGENT", "TASK", "LAUNCHED BY", "SKILLS RUN", "STATUS", "USED", "CONTEXT", "LEFT"];

/// Skill name without its plugin prefix, so `compound-engineering:ce-plan` fits as `ce-plan`.
fn short_skill(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// `ce-plan, ce-work ×2` for the skills one agent invoked itself.
fn skill_names(stats: &crate::model::TranscriptStats) -> String {
    let names: Vec<String> = stats
        .skills
        .iter()
        .map(|(name, st)| {
            let name = short_skill(name);
            if st.calls > 1 { format!("{name} ×{}", st.calls) } else { name.to_string() }
        })
        .collect();
    if names.is_empty() { "—".into() } else { names.join(", ") }
}

/// Gauge and tokens-left cells; context is measured against each agent's own auto-compact point.
fn context_cells(ctx: Option<ContextInfo>) -> [Cell<'static>; 2] {
    match ctx {
        Some(ctx) => {
            let style = Style::new().fg(ratio_color(ctx.used_ratio()));
            let approx = if ctx.estimated { "≈" } else { "" };
            [
                Cell::from(Line::from(vec![
                    Span::styled(bar(ctx.used_ratio(), 6), style),
                    Span::styled(format!(" {approx}{:.0}%", ctx.used_ratio() * 100.0), style.bold()),
                ])),
                right_cell(fmt_tokens(ctx.left())),
            ]
        }
        None => [Cell::from(dim("—")), right_cell("—")],
    }
}

/// Full-screen list of a session's subagents: what launched each one, what it ran,
/// how many tokens it used and how much context it has left.
fn draw_agents(f: &mut Frame, area: Rect, app: &App, s: &Session) {
    let agents = s.subagents_by_start();
    let total = s.total_tokens().total();
    let in_agents: u64 = agents.iter().map(|(a, _)| a.stats.tokens.total()).sum();
    let running = agents.iter().filter(|(a, _)| a.running).count();
    let [summary, list] = Layout::vertical([Constraint::Length(5), Constraint::Min(3)]).areas(area);

    let share = if total > 0 { in_agents as f64 / total as f64 * 100.0 } else { 0.0 };
    let mut tokens = vec![label("tokens")];
    tokens.extend(usage_spans(&s.total_tokens()));
    let mut count = vec![
        label("agents"),
        Span::styled(format!("{} launched", agents.len()), Style::new().bold()),
    ];
    if running > 0 {
        count.push(Span::styled(format!(" · {running} running"), Style::new().fg(Color::Green)));
    }
    count.extend([dim("   "), Span::raw(fmt_tokens(in_agents)), dim(format!(" tokens ({share:.0}% of session)"))]);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dark_gray())
        .title(session_heading(s));
    f.render_widget(Paragraph::new(vec![context_line(s, app), Line::from(tokens), Line::from(count)]).block(block), summary);

    // The main agent comes first so each subagent can be compared against it.
    let (main_style, _) = status_style(s.status);
    let mut rows = vec![Row::new(
        [
            Cell::from(dim(format!("{:>3}", 0))),
            Cell::from(Span::raw(fmt_local(s.stats.started.as_deref()))),
            Cell::from(Span::styled(MAIN_AGENT, Style::new().bold())),
            Cell::from(dim(s.title.clone().or_else(|| s.stats.title.clone()).unwrap_or_default())),
            Cell::from(dim("—")),
            Cell::from(Span::styled(skill_names(&s.stats), Style::new().fg(Color::Magenta))),
            Cell::from(Span::styled(format!("{} {}", status_glyph(s.status, app.tick), s.status.as_str()), main_style)),
            right_cell(fmt_tokens(s.stats.tokens.total())),
        ]
        .into_iter()
        .chain(context_cells(s.context(app.compact_window)))
        .collect::<Vec<_>>(),
    )];
    for (i, (a, launched_by)) in agents.iter().enumerate() {
        let status = if a.running {
            Span::styled(format!("{} running", SPINNER[(app.tick / 2 % 4) as usize]), Style::new().fg(Color::Green))
        } else {
            dim("✓ done")
        };
        let launched_by = match launched_by {
            Some(skill) => Span::styled(short_skill(skill).to_string(), Style::new().fg(Color::Magenta).bold()),
            None => dim("—"),
        };
        let cells = [
            Cell::from(dim(format!("{:>3}", i + 1))),
            Cell::from(Span::raw(fmt_local(a.stats.started.as_deref()))),
            Cell::from(Span::styled(a.agent_type.clone(), Style::new().fg(ACCENT))),
            Cell::from(Span::raw(a.description.clone())),
            Cell::from(launched_by),
            Cell::from(Span::styled(skill_names(&a.stats), Style::new().fg(Color::Magenta))),
            Cell::from(status),
            right_cell(fmt_tokens(a.stats.tokens.total())),
        ];
        rows.push(Row::new(cells.into_iter().chain(context_cells(a.context(app.compact_window))).collect::<Vec<_>>()));
    }

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dark_gray())
        .title(Line::from(" Subagents ").fg(ACCENT))
        .title(Line::from(dim(" context left = tokens before each agent's auto-compact ")).alignment(Alignment::Right));
    let len = rows.len();
    let table = styled_table(rows, &AGENT_WIDTHS, block).header(Row::new(AGENT_HEADER).style(Style::new().dark_gray()));
    render_scrolled(f, table, list, len, app.history_scroll);
}

fn pane_block(title: &'static str, focused: bool) -> Block<'static> {
    let border = if focused { Style::new().fg(ACCENT) } else { Style::new().dark_gray() };
    let title_style = if focused { Style::new().fg(ACCENT).bold() } else { Style::new() };
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(Span::styled(format!(" {title} "), title_style)))
}

fn fmt_day(day: &str) -> String {
    match chrono::NaiveDate::parse_from_str(day, DAY_FMT) {
        Ok(d) if d == chrono::Local::now().date_naive() => "Today".into(),
        Ok(d) => d.format("%a %b %d").to_string(),
        Err(_) => day.to_string(),
    }
}

fn weight(v: i64, max: i64) -> f64 {
    if max <= 0 { 0.0 } else { v as f64 / max as f64 }
}

/// 30-day browser: days on the left, that day's sessions and skills on the right.
fn draw_usage(f: &mut Frame, area: Rect, u: &Usage) {
    let [left, right] = Layout::horizontal([Constraint::Length(34), Constraint::Fill(1)]).areas(area);
    let [sessions_area, skills_area] = Layout::vertical([Constraint::Fill(3), Constraint::Fill(2)]).areas(right);

    // Days
    let max_day = u.days.iter().map(|d| d.total).max().unwrap_or(0);
    let all_total: i64 = u.days.iter().map(|d| d.total).sum();
    let mut rows = vec![Row::new(vec![
        Cell::from(Span::styled(format!("All {} days", crate::store::RETENTION_DAYS), Style::new().bold())),
        Cell::from(""),
        right_cell(fmt_tokens(all_total as u64)),
    ])];
    rows.extend(u.days.iter().map(|d| {
        Row::new(vec![
            Cell::from(fmt_day(&d.day)),
            Cell::from(Span::styled(bar(weight(d.total, max_day), 8), Style::new().fg(ACCENT))),
            right_cell(fmt_tokens(d.total as u64)),
        ])
    }));
    let mut title = pane_block("Days", u.focus == Pane::Days);
    if u.syncing {
        title = title.title_bottom(Line::from(dim(" indexing… ")).alignment(Alignment::Right));
    } else if let Some(e) = &u.error {
        title = title.title_bottom(Line::from(Span::styled(format!(" {} ", crate::hook::truncate(e, 28)), Style::new().fg(Color::Red))));
    }
    let widths = [Constraint::Length(11), Constraint::Length(8), Constraint::Fill(1)];
    let table = styled_table(rows, &widths, title);
    f.render_stateful_widget(table, left, &mut TableState::default().with_selected(Some(u.day_index())));

    let scope = u.day.as_deref().map(fmt_day).unwrap_or_else(|| format!("last {} days", crate::store::RETENTION_DAYS));

    // Sessions
    let max_s = u.sessions.first().map(|s| s.total).unwrap_or(0);
    let rows = u.sessions.iter().map(|s| {
        Row::new(vec![
            Cell::from(Span::styled(s.project.clone(), Style::new().bold())),
            Cell::from(dim(s.title.clone())),
            Cell::from(Span::raw(s.model.trim_start_matches("claude-").to_string())),
            right_cell(s.turns.to_string()),
            right_cell(if s.skill_runs > 0 { s.skill_runs.to_string() } else { String::new() }),
            right_cell(fmt_tokens(s.output as u64)),
            right_cell(fmt_tokens(s.total as u64)),
            Cell::from(Span::styled(bar(weight(s.total, max_s), 8), Style::new().fg(ACCENT))),
        ])
    });
    let widths = [
        Constraint::Length(18),
        Constraint::Fill(1),
        Constraint::Length(12),
        Constraint::Length(5),
        Constraint::Length(6),
        Constraint::Length(7),
        Constraint::Length(7),
        Constraint::Length(8),
    ];
    let header = Row::new(["PROJECT", "TITLE", "MODEL", "TURNS", "SKILLS", "OUT", "TOKENS", ""]).style(Style::new().dark_gray());
    let block = pane_block("Sessions", u.focus == Pane::Sessions)
        .title(Line::from(dim(format!(" {scope} · {} ", u.sessions.len()))).alignment(Alignment::Right));
    let sel = (u.focus == Pane::Sessions && !u.sessions.is_empty()).then_some(u.session_idx);
    let table = styled_table(rows, &widths, block).header(header);
    f.render_stateful_widget(table, sessions_area, &mut TableState::default().with_selected(sel));

    // Skills
    let max_k = u.skills.first().map(|s| s.total).unwrap_or(0);
    let rows = u.skills.iter().map(|s| {
        Row::new(vec![
            Cell::from(Span::styled(s.skill.clone(), Style::new().fg(Color::Magenta).bold())),
            right_cell(s.calls.to_string()),
            right_cell(s.sessions.to_string()),
            right_cell(fmt_tokens(s.output as u64)),
            right_cell(format!("≈{}", fmt_tokens(s.total as u64))),
            Cell::from(Span::styled(bar(weight(s.total, max_k), 8), Style::new().fg(Color::Magenta))),
        ])
    });
    let widths = [
        Constraint::Fill(1),
        Constraint::Length(5),
        Constraint::Length(8),
        Constraint::Length(7),
        Constraint::Length(8),
        Constraint::Length(8),
    ];
    let header = Row::new(["SKILL", "CALLS", "SESSIONS", "OUT", "TOKENS", ""]).style(Style::new().dark_gray());
    let block = pane_block("Skills", u.focus == Pane::Skills)
        .title(Line::from(dim(format!(" {scope} · ≈ tokens inside skill windows "))).alignment(Alignment::Right));
    let sel = (u.focus == Pane::Skills && !u.skills.is_empty()).then_some(u.skill_idx);
    let table = styled_table(rows, &widths, block).header(header);
    f.render_stateful_widget(table, skills_area, &mut TableState::default().with_selected(sel));
}

/// One session from history: per-day totals and every skill run.
fn draw_drill(f: &mut Frame, area: Rect, u: &Usage) {
    let Some(d) = u.drill.as_ref() else { return };
    let s = &d.session;
    let [summary, list] = Layout::vertical([Constraint::Length(5), Constraint::Min(3)]).areas(area);

    let mut days = vec![label("days")];
    for (i, (day, total)) in d.days.iter().enumerate() {
        if i > 0 {
            days.push(dim("  ·  "));
        }
        days.extend([dim(format!("{} ", fmt_day(day))), Span::raw(fmt_tokens(*total as u64))]);
    }
    let lines = vec![
        Line::from(vec![
            label("session"),
            dim(s.session_id.clone()),
            dim("   model "),
            Span::raw(s.model.clone()),
        ]),
        Line::from(vec![
            label("tokens"),
            Span::styled(fmt_tokens(s.total as u64), Style::new().bold()),
            dim(format!("   out {}   turns {}   skill runs {}", fmt_tokens(s.output as u64), s.turns, d.runs.len())),
        ]),
        Line::from(days),
    ];
    let mut heading = vec![Span::styled(format!(" {} ", s.project), Style::new().bold())];
    if !s.title.is_empty() {
        heading.push(dim(format!("· {} ", s.title)));
    }
    let block = Block::bordered().border_type(BorderType::Rounded).border_style(Style::new().dark_gray()).title(Line::from(heading));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), summary);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dark_gray())
        .title(Line::from(" Skill runs ").fg(ACCENT));
    if d.runs.is_empty() {
        f.render_widget(Paragraph::new(dim("No skills invoked in this session.")).alignment(Alignment::Center).block(block), list);
        return;
    }
    let max = d.runs.iter().map(|r| r.total).max().unwrap_or(0) as u64;
    let rows = d
        .runs
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let turns = r.turns as u32;
            run_row(i, Some(&r.at), &r.agent_type, &r.skill, r.via, turns, r.output as u64, r.total as u64, max)
        })
        .collect();
    let table = runs_table(rows, block);
    f.render_stateful_widget(table, list, &mut TableState::default().with_selected(Some(d.scroll)));
}
