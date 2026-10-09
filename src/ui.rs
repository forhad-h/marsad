use crate::app::App;
use crate::model::{Session, Status, TokenUsage};
use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Cell, Paragraph, Row, Table, TableState, Wrap};

const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];
const ACCENT: Color = Color::Cyan;

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

pub fn fmt_tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.1}M", n as f64 / 1e6),
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

fn short_path(p: &str) -> String {
    match dirs::home_dir().and_then(|h| p.strip_prefix(&*h.to_string_lossy()).map(str::to_string)) {
        Some(rest) => format!("~{rest}"),
        None => p.to_string(),
    }
}

pub fn draw(f: &mut Frame, app: &App) {
    let visible = app.visible();
    let selected = app.selected_session();
    let details_height = if selected.is_some() { 9 } else { 0 };
    let [header, body, details, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(4),
        Constraint::Length(details_height),
        Constraint::Length(1),
    ])
    .areas(f.area());

    draw_header(f, header, app, &visible);
    if visible.is_empty() {
        draw_empty(f, body, app);
    } else {
        draw_table(f, body, app, &visible);
    }
    if let Some(s) = selected {
        draw_details(f, details, s, app.tick);
    }
    let ended = if app.show_ended { "hide ended" } else { "show ended" };
    let keys = Line::from(vec![
        " ↑↓".bold(), " select  ".dark_gray(),
        "a".bold(), format!(" {ended}  ").dark_gray(),
        "q".bold(), " quit".dark_gray(),
    ]);
    f.render_widget(Paragraph::new(keys), footer);
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
    let _ = app;
    f.render_widget(Paragraph::new(Line::from(left)), area);
    let right = Line::from(vec![Span::raw("Σ "), Span::styled(fmt_tokens(total), Style::new().bold()), Span::raw(" tokens ")]);
    f.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);
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
        let dim = s.status == Status::Ended;
        let row = Row::new(vec![
            Cell::from(Line::from(vec![
                Span::styled(format!(" {} ", status_glyph(s.status, app.tick)), style),
                Span::styled(label, style),
            ])),
            Cell::from(Span::styled(s.project(), Style::new().bold())),
            Cell::from(Span::styled(agents, Style::new().fg(ACCENT))),
            Cell::from(Line::from(activity)),
            Cell::from(Line::from(fmt_age(now - s.status_since)).alignment(Alignment::Right)),
            Cell::from(Line::from(fmt_tokens(s.total_tokens().total())).alignment(Alignment::Right)),
        ]);
        if dim { row.style(Style::new().dark_gray()) } else { row }
    });
    let widths = [
        Constraint::Length(14),
        Constraint::Length(20),
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(5),
        Constraint::Length(8),
    ];
    let header = Row::new(["  STATUS", "PROJECT", "", "ACTIVITY", "AGE", "TOKENS"])
        .style(Style::new().dark_gray())
        .bottom_margin(0);
    let table = Table::new(rows, widths)
        .header(header)
        .column_spacing(1)
        .row_highlight_style(Style::new().bg(Color::Indexed(236)).add_modifier(Modifier::BOLD))
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::new().dark_gray())
                .title(Line::from(" Sessions ").fg(ACCENT)),
        );
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

fn draw_details(f: &mut Frame, area: Rect, s: &Session, tick: u64) {
    let label = |t: &'static str| Span::styled(format!("{t:<9}"), Style::new().fg(ACCENT));
    let dim = |t: String| Span::styled(t, Style::new().dark_gray());

    let mut meta = vec![label("session"), Span::raw(short_path(&s.cwd))];
    if let Some(m) = s.model.as_ref() {
        meta.extend([dim("   model ".into()), Span::raw(m.clone())]);
    }
    if let Some(m) = s.permission_mode.as_ref() {
        meta.extend([dim("   mode ".into()), Span::raw(m.clone())]);
    }
    meta.extend([dim("   id ".into()), dim(s.id.chars().take(8).collect())]);

    let mut tokens = vec![label("tokens")];
    tokens.extend(usage_spans(&s.total_tokens()));

    let mut agents = vec![label("agents"), Span::raw("main "), Span::styled(fmt_tokens(s.stats.tokens.total()), Style::new().bold())];
    for a in s.subagents.values() {
        let mark = if a.running { SPINNER[(tick / 2 % 4) as usize] } else { "✓" };
        let mark_style = if a.running { Style::new().fg(Color::Green) } else { Style::new().dark_gray() };
        agents.extend([
            dim("  ·  ".into()),
            Span::raw(format!("{} ", a.agent_type)),
            Span::styled(format!("{mark} "), mark_style),
            Span::raw(fmt_tokens(a.stats.tokens.total())),
        ]);
    }

    let mut skills_list: Vec<_> = s.all_skills().into_iter().collect();
    skills_list.sort_by(|a, b| b.1.tokens.total().cmp(&a.1.tokens.total()).then(b.1.calls.cmp(&a.1.calls)));
    let mut skills = vec![label("skills")];
    if skills_list.is_empty() {
        skills.push(dim("none yet".into()));
    }
    for (i, (name, st)) in skills_list.iter().enumerate() {
        if i > 0 {
            skills.push(dim("  ·  ".into()));
        }
        skills.extend([
            Span::styled(name.clone(), Style::new().fg(Color::Magenta)),
            dim(format!(" ×{} ", st.calls)),
            Span::raw(format!("≈{}", fmt_tokens(st.tokens.total()))),
        ]);
    }

    let mut lines = vec![Line::from(meta), Line::from(tokens), Line::from(agents), Line::from(skills)];
    if !s.detail.is_empty() {
        lines.insert(1, Line::from(vec![label("now"), Span::raw(s.detail.clone())]));
    }

    let title = s.title.clone().or_else(|| s.stats.title.clone());
    let mut heading = vec![Span::styled(format!(" {} ", s.project()), Style::new().bold())];
    if let Some(t) = title {
        heading.push(Span::styled(format!("· {t} "), Style::new().dark_gray()));
    }
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().dark_gray())
        .title(Line::from(heading));
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).block(block), area);
}
