//! The share card: a 1200×627 PNG with the rank badge, the rank, the token stats and the earned achievements on an ivory ground framed in girih, drawn as SVG and rasterized by resvg with a bundled font.
// used by the share flow (U5)
#![allow(dead_code)]

use crate::badge_art::{self, Point, Rgb};
use crate::badges::Badges;
use crate::model::fmt_tokens;
use anyhow::{Context, Result};
use resvg::{tiny_skia, usvg};
use std::f64::consts::PI;
use std::fmt::Write;
use std::path::Path;

/// The card's size in pixels: LinkedIn's link-preview ratio.
pub const WIDTH: u32 = 1200;
pub const HEIGHT: u32 = 627;

const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const FONT_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
const FAMILY: &str = "Inter";
/// The SemiBold file names its own family rather than a weight of Inter.
const BOLD: &str = "Inter SemiBold";

/// An achievement's level on the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Medal {
    Bronze,
    Silver,
    Gold,
}

impl Medal {
    /// 1 bronze, 2 silver, 3 gold; anything else earns nothing.
    pub fn from_level(level: usize) -> Option<Medal> {
        match level {
            1 => Some(Medal::Bronze),
            2 => Some(Medal::Silver),
            3 => Some(Medal::Gold),
            _ => None,
        }
    }

    /// Face, edge and highlight colours.
    fn colors(self) -> (Rgb, Rgb, Rgb) {
        match self {
            Medal::Bronze => ((0xc2, 0x7c, 0x48), (0x6e, 0x3e, 0x1a), (0xe8, 0xb6, 0x8a)),
            Medal::Silver => ((0xb9, 0xc1, 0xcb), (0x5c, 0x66, 0x73), (0xee, 0xf1, 0xf5)),
            Medal::Gold => ((0xdc, 0xa9, 0x32), (0x7e, 0x5a, 0x0c), (0xf8, 0xe0, 0x8c)),
        }
    }
}

/// One earned achievement: "Busiest day" "212.5M" gold.
#[derive(Debug, Clone, PartialEq)]
pub struct Achievement {
    pub label: String,
    pub value: String,
    pub medal: Medal,
}

/// What the card shows, frozen once per share.
#[derive(Debug, Clone, PartialEq)]
pub struct CardData {
    /// The rank's tier, Novice (0) to Grandmaster (6).
    pub tier: usize,
    pub rank: String,
    /// Who is sharing; the line is left out when None.
    pub name: Option<String>,
    pub all_time: u64,
    pub week: u64,
    pub month: u64,
    /// The per-day average over the last 30 days.
    pub daily_average: u64,
    /// Earned achievements, best first; the card shows up to four and leaves the row out when empty.
    pub achievements: Vec<Achievement>,
    /// The snapshot day, `YYYY-MM-DD`.
    pub date: String,
}

/// The earned achievements of `b` for the card, gold first, ties in the badge order.
pub fn achievements(b: &Badges) -> Vec<Achievement> {
    let mut out: Vec<Achievement> = b
        .achievements
        .iter()
        .filter_map(|l| {
            let medal = Medal::from_level(l.level)?;
            let n = l.value.max(0) as u64;
            let (label, value) = match l.key {
                "streak" => ("Streak", format!("{n} {}", if n == 1 { "day" } else { "days" })),
                "skills" => ("Skills", fmt_count(n)),
                "subagents" => ("Subagents", fmt_count(n)),
                "busiest_day" => ("Busiest day", fmt_count(n)),
                _ => (l.name, fmt_count(n)),
            };
            Some(Achievement { label: label.into(), value, medal })
        })
        .collect();
    out.sort_by_key(|a| std::cmp::Reverse(a.medal));
    out
}

/// The patterned frame's width at each edge.
const FRAME: f64 = 22.0;
/// The badge: centre and radius.
const BADGE_X: f64 = 316.0;
const BADGE_Y: f64 = 313.5;
const BADGE_R: f64 = 248.0;
/// The text column: its left edge and width; four stat columns and two achievement columns share it.
const TEXT_X: f64 = 622.0;
const TEXT_W: f64 = 516.0;
const STATS: usize = 4;
const AWARD_COLS: usize = 2;
/// The background lattice's spacing.
const TILE: f64 = 58.0;

const IVORY: Rgb = (0xf8, 0xf3, 0xe6);
const INK: Rgb = (0x1b, 0x21, 0x2c);

/// A token count for the card: the dashboard's format, with billions and trillions so big totals stay short, and no `.0`.
pub fn fmt_count(n: u64) -> String {
    let s = match n {
        0..1_000_000_000 => fmt_tokens(n),
        1_000_000_000..1_000_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        _ => format!("{:.1}T", n as f64 / 1e12),
    };
    match s.find(".0") {
        Some(i) if i + 3 == s.len() => format!("{}{}", &s[..i], &s[i + 2..]),
        _ => s,
    }
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

/// `a` moved towards `b` by `t`.
fn mix(a: Rgb, b: Rgb, t: f64) -> Rgb {
    let m = |x: u8, y: u8| (x as f64 + (y as f64 - x as f64) * t).round() as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// `2026-10-24` as `24 October 2026`; anything else as given.
fn fmt_date(day: &str) -> String {
    chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").map_or_else(|_| day.to_string(), |d| d.format("%-d %B %Y").to_string())
}

/// A closed polygon's path, two decimals, no negative zero.
fn poly(points: &[Point]) -> String {
    let num = |v: f64| {
        let s = format!("{v:.2}");
        if s == "-0.00" { "0.00".into() } else { s }
    };
    let mut d = String::new();
    for (i, p) in points.iter().enumerate() {
        let _ = write!(d, "{}{} {}", if i == 0 { "M" } else { "L" }, num(p.0), num(p.1));
    }
    d.push('Z');
    d
}

/// An eight-point star centred on (x, y): {8/k} with k 2 (two squares) or 3 (the sharper star).
fn star8(x: f64, y: f64, r: f64, k: usize, turn: f64) -> String {
    let outline = badge_art::hankin_star(8, r, turn, PI * k as f64 / 8.0).expect("contact in bounds");
    poly(&outline.iter().map(|p| (x + p.0, y - p.1)).collect::<Vec<_>>())
}

/// A text run's width in the bundled fonts.
fn text_width(family: &str, size: f64, spacing: f64, text: &str) -> f64 {
    let s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4000\" height=\"400\"><text id=\"t\" x=\"0\" y=\"200\" font-size=\"{size}\" font-family=\"{family}\" letter-spacing=\"{spacing}\">{}</text></svg>", escape(text));
    tree(&s).ok().and_then(|t| t.node_by_id("t").map(|n| n.abs_bounding_box().width() as f64)).unwrap_or(0.0)
}

/// The font size, at most `size`, at which the text fits `max` pixels; letter spacing shrinks with it.
fn fitted(family: &str, size: f64, spacing: f64, text: &str, max: f64) -> (f64, f64) {
    let w = text_width(family, size, spacing, text);
    if w <= max {
        return (size, spacing);
    }
    let k = (max / w * 0.98 * 10.0).floor() / 10.0;
    ((size * k * 10.0).floor() / 10.0, (spacing * k * 100.0).round() / 100.0)
}

/// One line of text, shrunk to fit `max`.
#[allow(clippy::too_many_arguments)]
fn line(s: &mut String, id: &str, x: f64, y: f64, family: &str, size: f64, spacing: f64, color: Rgb, text: &str, max: f64, anchor: &str) {
    let (size, spacing) = fitted(family, size, spacing, text, max);
    let _ = write!(s, "<text id=\"{id}\" x=\"{x}\" y=\"{y}\" font-size=\"{size}\" font-family=\"{family}\" letter-spacing=\"{spacing}\" text-anchor=\"{anchor}\" fill=\"{}\">{}</text>", hex(color), escape(text));
}

/// The ground: ivory with a faint star-and-cross lattice, strongest behind the badge.
fn ground(s: &mut String, tint: Rgb) {
    let (w, h) = (WIDTH as f64, HEIGHT as f64);
    let _ = write!(s, "<rect width=\"{w}\" height=\"{h}\" fill=\"{}\"/>", hex(IVORY));
    let _ = write!(s, "<defs><linearGradient id=\"fade\" x1=\"0\" y1=\"0\" x2=\"{w}\" y2=\"0\" gradientUnits=\"userSpaceOnUse\"><stop offset=\"0\" stop-color=\"#fff\"/><stop offset=\"0.42\" stop-color=\"#fff\" stop-opacity=\"0.75\"/><stop offset=\"0.62\" stop-color=\"#fff\" stop-opacity=\"0.22\"/><stop offset=\"1\" stop-color=\"#fff\" stop-opacity=\"0.22\"/></linearGradient>");
    let _ = write!(s, "<mask id=\"lattice-mask\" maskUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"{w}\" height=\"{h}\"><rect x=\"{FRAME}\" y=\"{FRAME}\" width=\"{}\" height=\"{}\" fill=\"url(#fade)\"/></mask></defs>", w - 2.0 * FRAME, h - 2.0 * FRAME);
    // Eight-point stars on a square lattice, tips touching along the rows; the gaps between them read as crosses. Centred on the badge.
    let _ = write!(s, "<g mask=\"url(#lattice-mask)\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.3\" stroke-opacity=\"0.30\" stroke-linejoin=\"round\">", hex(tint));
    let (x0, y0) = (BADGE_X - TILE * (BADGE_X / TILE).ceil(), BADGE_Y - TILE * (BADGE_Y / TILE).ceil());
    let (cols, rows) = (((w - x0) / TILE).ceil() as usize, ((h - y0) / TILE).ceil() as usize);
    let mut d = String::new();
    for j in 0..=rows {
        for i in 0..=cols {
            let (x, y) = (x0 + TILE * i as f64, y0 + TILE * j as f64);
            d.push_str(&star8(x, y, TILE / 2.0, 2, 0.0));
            // A small {8/3} star in each cross.
            d.push_str(&star8(x + TILE / 2.0, y + TILE / 2.0, TILE * 0.2, 3, PI / 8.0));
        }
    }
    let _ = write!(s, "<path d=\"{d}\"/></g>");
}

/// The frame: a band in the tier's ground colour with a chain of stars and lozenges in its line colour.
fn frame(s: &mut String, band: Rgb, line: Rgb, inner: Rgb) {
    let (w, h) = (WIDTH as f64, HEIGHT as f64);
    let _ = write!(s, "<path fill-rule=\"evenodd\" fill=\"{}\" d=\"M0 0H{w}V{h}H0Z M{FRAME} {FRAME}V{}H{}V{FRAME}Z\"/>", hex(band), h - FRAME, w - FRAME);
    let mid = FRAME / 2.0;
    // Stars at an even spacing that lands on every corner.
    let step = |len: f64| len / (len / 34.0).round();
    let (sx, sy) = (step(w - FRAME), step(h - FRAME));
    let mut stars = String::new();
    let mut links = String::new();
    let mut put = |x: f64, y: f64, along_x: bool, step: f64| {
        stars.push_str(&star8(x, y, 7.2, 3, PI / 8.0));
        // A lozenge between each pair.
        let (cx, cy) = if along_x { (x + step / 2.0, y) } else { (x, y + step / 2.0) };
        let (a, b) = if along_x { (6.5, 3.2) } else { (3.2, 6.5) };
        let _ = write!(links, "M{} {cy}L{cx} {}L{} {cy}L{cx} {}Z", cx - a, cy - b, cx + a, cy + b);
    };
    let n = ((w - FRAME) / sx).round() as usize;
    for i in 0..n {
        put(mid + sx * i as f64, mid, true, sx);
        put(w - mid - sx * i as f64 - sx, h - mid, true, sx);
    }
    let m = ((h - FRAME) / sy).round() as usize;
    for i in 0..m {
        put(w - mid, mid + sy * i as f64, false, sy);
        put(mid, h - mid - sy * i as f64 - sy, false, sy);
    }
    let _ = write!(s, "<path d=\"{stars}\" fill=\"{}\" fill-opacity=\"0.9\"/><path d=\"{links}\" fill=\"{}\" fill-opacity=\"0.55\"/>", hex(line), hex(line));
    // A fine rule inside the band.
    let _ = write!(s, "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"1.5\"/>", FRAME + 7.0, FRAME + 7.0, w - 2.0 * FRAME - 14.0, h - 2.0 * FRAME - 14.0, hex(inner));
}

/// A small medal: a coin in the metal's colours with an eight-point star struck into it.
fn medal(s: &mut String, x: f64, y: f64, r: f64, m: Medal) {
    let (face, edge, shine) = m.colors();
    let _ = write!(s, "<circle cx=\"{x}\" cy=\"{y}\" r=\"{r}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"1.5\"/>", hex(face), hex(edge));
    let _ = write!(s, "<circle cx=\"{x}\" cy=\"{y}\" r=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"0.8\" stroke-opacity=\"0.7\"/>", r - 3.0, hex(edge));
    let _ = write!(s, "<path d=\"{}\" fill=\"{}\" stroke=\"{}\" stroke-width=\"0.9\" stroke-linejoin=\"round\"/>", star8(x, y, r * 0.66, 2, PI / 8.0), hex(shine), hex(edge));
}

/// The card as an SVG document.
pub fn svg(d: &CardData) -> String {
    let p = badge_art::badge(d.tier).palette;
    let accent = mix(p.field, INK, 0.15);
    let muted = mix(INK, IVORY, 0.36);
    let rule = mix(p.field, IVORY, 0.72);
    let mut s = String::new();
    let _ = write!(s, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{HEIGHT}\" viewBox=\"0 0 {WIDTH} {HEIGHT}\" font-family=\"{FAMILY}\">");
    ground(&mut s, p.field);
    frame(&mut s, p.ground, mix(p.line, p.rosette, 0.25), rule);

    // The badge on a cleared ivory disc, with a soft shadow and a halo ring.
    let _ = write!(s, "<defs><filter id=\"shadow\" x=\"-20%\" y=\"-20%\" width=\"140%\" height=\"140%\"><feGaussianBlur stdDeviation=\"9\"/></filter></defs>");
    let _ = write!(s, "<circle cx=\"{BADGE_X}\" cy=\"{BADGE_Y}\" r=\"{}\" fill=\"{}\"/>", BADGE_R + 22.0, hex(IVORY));
    let _ = write!(s, "<circle cx=\"{BADGE_X}\" cy=\"{BADGE_Y}\" r=\"{}\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\" stroke-opacity=\"0.55\"/>", BADGE_R + 13.0, hex(p.field));
    let _ = write!(s, "<circle cx=\"{BADGE_X}\" cy=\"{}\" r=\"{BADGE_R}\" fill=\"{}\" fill-opacity=\"0.35\" filter=\"url(#shadow)\"/>", BADGE_Y + 7.0, hex(p.band));
    s.push_str(&badge_art::svg(d.tier, BADGE_X, BADGE_Y, BADGE_R));

    // The text block, centred between the top of the frame and the footer rule.
    let rows = d.achievements.len().min(STATS).div_ceil(AWARD_COLS);
    let name = d.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let name_gap = if name.is_some() { 52.0 } else { 0.0 };
    let rank_gap = if name.is_some() { 86.0 } else { 94.0 };
    let award_gap = if rows > 0 { 64.0 + 42.0 * (rows - 1) as f64 } else { 0.0 };
    let height = 12.0 + name_gap + rank_gap + 64.0 + 40.0 + award_gap;
    let (top, bottom) = (FRAME + 58.0, 486.0);
    let y = top + (bottom - top - height) / 2.0 + 12.0;

    let tier = d.tier.min(badge_art::TIERS - 1) + 1;
    line(&mut s, "kicker", TEXT_X, y, BOLD, 15.0, 2.2, accent, &format!("AI-ASSISTED ENGINEERING \u{b7} RANK {tier} OF {}", badge_art::TIERS), TEXT_W, "start");
    if let Some(name) = name {
        line(&mut s, "name", TEXT_X, y + name_gap, BOLD, 30.0, -0.2, INK, name, TEXT_W, "start");
    }
    let rank_y = y + name_gap + rank_gap;
    line(&mut s, "rank", TEXT_X - 3.0, rank_y, BOLD, 86.0, -2.0, INK, &d.rank, TEXT_W, "start");

    let col = TEXT_W / STATS as f64;
    let label_y = rank_y + 64.0;
    let stats = [("all-time", "All time", d.all_time), ("week", "7 days", d.week), ("month", "30 days", d.month), ("daily", "Per day", d.daily_average)];
    for (i, (id, name, value)) in stats.into_iter().enumerate() {
        let x = TEXT_X + col * i as f64;
        line(&mut s, &format!("{id}-label"), x, label_y, BOLD, 13.0, 1.8, muted, &name.to_uppercase(), col - 14.0, "start");
        line(&mut s, id, x, label_y + 40.0, BOLD, 30.0, -0.6, if i == 0 { accent } else { INK }, &fmt_count(value), col - 14.0, "start");
    }

    let award_y = label_y + 40.0 + 64.0;
    let cell = TEXT_W / AWARD_COLS as f64;
    for (i, a) in d.achievements.iter().take(STATS).enumerate() {
        let (x, y) = (TEXT_X + cell * (i % AWARD_COLS) as f64, award_y + 42.0 * (i / AWARD_COLS) as f64);
        medal(&mut s, x + 13.0, y - 6.5, 14.0, a.medal);
        let text = format!("{} \u{b7} {}", a.label, a.value);
        let (size, spacing) = fitted(FAMILY, 18.0, 0.0, &text, cell - 52.0);
        let _ = write!(s, "<text id=\"award-{i}\" x=\"{}\" y=\"{y}\" font-size=\"{size}\" letter-spacing=\"{spacing}\" fill=\"{}\">{} \u{b7} <tspan font-family=\"{BOLD}\" fill=\"{}\">{}</tspan></text>", x + 34.0, hex(muted), escape(&a.label), hex(INK), escape(&a.value));
    }

    let _ = write!(s, "<rect x=\"{TEXT_X}\" y=\"504\" width=\"{TEXT_W}\" height=\"1.5\" fill=\"{}\"/>", hex(rule));
    line(&mut s, "date", TEXT_X, 545.0, FAMILY, 17.0, 0.0, muted, &fmt_date(&d.date), TEXT_W / 2.0, "start");
    let _ = write!(s, "<text id=\"footer\" x=\"{}\" y=\"545\" font-size=\"17\" text-anchor=\"end\" fill=\"{}\">via <tspan font-family=\"{BOLD}\" fill=\"{}\">marsad</tspan></text>", TEXT_X + TEXT_W, hex(muted), hex(accent));
    s.push_str("</svg>");
    s
}

/// The parser options: only the bundled font, never the system's.
fn options() -> usvg::Options<'static> {
    let mut opt = usvg::Options { font_family: FAMILY.into(), ..Default::default() };
    let db = opt.fontdb_mut();
    db.load_font_data(FONT_REGULAR.to_vec());
    db.load_font_data(FONT_SEMIBOLD.to_vec());
    db.set_sans_serif_family(FAMILY);
    opt
}

fn tree(svg: &str) -> Result<usvg::Tree> {
    usvg::Tree::from_str(svg, &options()).context("card svg")
}

/// The card as PNG bytes, 1200×627.
pub fn png(d: &CardData) -> Result<Vec<u8>> {
    let tree = tree(&svg(d))?;
    let mut pixmap = tiny_skia::Pixmap::new(WIDTH, HEIGHT).context("card pixmap")?;
    resvg::render(&tree, tiny_skia::Transform::default(), &mut pixmap.as_mut());
    pixmap.encode_png().context("card png")
}

/// Renders the card and writes it to `path`, creating its folder.
pub fn write_at(d: &CardData, path: &Path) -> Result<()> {
    let bytes = png(d)?;
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    std::fs::write(path, bytes).with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NAMES: [&str; 7] = ["Novice", "Apprentice", "Practitioner", "Specialist", "Expert", "Master", "Grandmaster"];
    const LONG_NAME: &str = "Maximiliana Alexandrova-Featherstonehaugh";

    fn award(label: &str, value: &str, medal: Medal) -> Achievement {
        Achievement { label: label.into(), value: value.into(), medal }
    }

    fn real() -> CardData {
        CardData {
            tier: 4,
            rank: "Expert".into(),
            name: Some("Forhad Hosain".into()),
            all_time: 386_200_000,
            week: 198_000_000,
            month: 340_000_000,
            daily_average: 11_300_000,
            achievements: vec![award("Busiest day", "212.5M", Medal::Gold), award("Skills", "13", Medal::Silver), award("Subagents", "102", Medal::Silver)],
            date: "2026-10-24".into(),
        }
    }

    fn uniform(tier: usize, n: u64) -> CardData {
        let v = fmt_count(n);
        let achievements = vec![award("Busiest day", &v, Medal::Gold), award("Skills", &v, Medal::Silver), award("Subagents", &v, Medal::Bronze), award("Streak", &format!("{v} days"), Medal::Gold)];
        CardData { tier, rank: NAMES[tier].into(), name: Some(LONG_NAME.into()), all_time: n, week: n, month: n, daily_average: n, achievements, date: "2026-10-24".into() }
    }

    fn decode(png: &[u8]) -> tiny_skia::Pixmap {
        tiny_skia::Pixmap::decode_png(png).unwrap()
    }

    #[test]
    fn png_header_reports_1200_by_627() {
        let png = png(&real()).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 1200);
        assert_eq!(u32::from_be_bytes(png[20..24].try_into().unwrap()), 627);
    }

    #[test]
    fn rank_name_is_drawn_so_the_font_loaded() {
        let d = real();
        let with = decode(&png(&d).unwrap());
        let without = decode(&png(&CardData { rank: String::new(), ..d.clone() }).unwrap());
        let bbox = tree(&svg(&d)).unwrap().node_by_id("rank").unwrap().abs_bounding_box();
        assert!(bbox.width() > 100.0, "rank text has no width: {bbox:?}");
        let (x0, x1, y0, y1) = (bbox.left() as u32, bbox.right() as u32, bbox.top() as u32, bbox.bottom() as u32);
        let mut changed = 0;
        let mut ink = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let (a, b) = (with.pixel(x, y).unwrap(), without.pixel(x, y).unwrap());
                if a != b {
                    changed += 1;
                }
                if a.red() < 60 && a.green() < 60 && a.blue() < 70 {
                    ink += 1;
                }
            }
        }
        assert!(changed > 1000, "only {changed} pixels changed under the rank name");
        assert!(ink > 500, "only {ink} dark pixels in the rank name");
    }

    #[test]
    fn semibold_resolves_to_its_own_face() {
        // The heavier face shows as a wider run of the same text.
        assert!(text_width(BOLD, 60.0, 0.0, "Grandmaster") > text_width(FAMILY, 60.0, 0.0, "Grandmaster") + 5.0);
    }

    #[test]
    fn same_snapshot_renders_identical_bytes() {
        assert_eq!(png(&real()).unwrap(), png(&real()).unwrap());
    }

    #[test]
    fn every_text_fits_its_box_for_huge_and_zero_values() {
        for tier in 0..7 {
            for n in [0, 999, 12_300_000_000, 999_900_000_000_000] {
                let t = tree(&svg(&uniform(tier, n))).unwrap();
                let bounds = |id: &str| t.node_by_id(id).unwrap_or_else(|| panic!("no {id}")).abs_bounding_box();
                let fits = |id: &str, left: f64, width: f64| {
                    let b = bounds(id);
                    assert!(b.left() as f64 >= left - 8.0 && b.right() as f64 <= left + width, "tier {tier} n {n}: {id} {b:?} outside {left}+{width}");
                };
                for id in ["kicker", "name", "rank", "date", "footer"] {
                    fits(id, TEXT_X, TEXT_W);
                }
                let col = TEXT_W / STATS as f64;
                for (i, id) in ["all-time", "week", "month", "daily"].iter().enumerate() {
                    let left = TEXT_X + col * i as f64;
                    // A column leaves a gutter before the next one.
                    fits(id, left, col - 10.0);
                    fits(&format!("{id}-label"), left, col - 10.0);
                }
                let cell = TEXT_W / AWARD_COLS as f64;
                for i in 0..4 {
                    fits(&format!("award-{i}"), TEXT_X + cell * (i % 2) as f64, cell - 12.0);
                }
                assert!(bounds("name").width() > 300.0, "the long name shrank too far");
                assert!(bounds("award-3").bottom() < 500.0, "achievements run into the footer");
                assert!(bounds("date").right() < bounds("footer").left(), "date and footer overlap");
                assert!(bounds("kicker").top() > (FRAME + 20.0) as f32, "kicker runs into the frame");
            }
        }
    }

    #[test]
    fn missing_name_and_achievements_leave_no_trace() {
        let d = CardData { name: None, achievements: vec![], ..real() };
        let s = svg(&d);
        assert!(!s.contains("id=\"name\"") && !s.contains("award-"));
        let blank = svg(&CardData { name: Some("  ".into()), ..d.clone() });
        assert!(!blank.contains("id=\"name\""), "a blank name is no name");
        assert!(tree(&s).is_ok());
    }

    #[test]
    fn counts_switch_to_billions_and_trillions() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(11_300_000), "11.3M");
        assert_eq!(fmt_count(344_000_000), "344M");
        assert_eq!(fmt_count(2_000), "2k");
        assert_eq!(fmt_count(10_050_000), "10.1M");
        assert_eq!(fmt_count(12_300_000_000), "12.3B");
        assert_eq!(fmt_count(4_200_000_000_000), "4.2T");
    }

    #[test]
    fn card_shows_every_stat_and_the_date() {
        let s = svg(&real());
        for want in ["Expert", "Forhad Hosain", "386.2M", "198M", "340M", "11.3M", "PER DAY", "212.5M", "Subagents", "24 October 2026", "AI-ASSISTED ENGINEERING \u{b7} RANK 5 OF 7", "marsad"] {
            assert!(s.contains(want), "card lacks {want}");
        }
        assert!(!s.contains("TODAY"), "today is gone from the card");
    }

    #[test]
    fn rank_and_name_are_escaped() {
        let s = svg(&CardData { rank: "A<b>&\"c".into(), name: Some("<Zoë & \"Co\">".into()), ..real() });
        assert!(s.contains("A&lt;b&gt;&amp;&quot;c"));
        assert!(s.contains("&lt;Zoë &amp; &quot;Co&quot;&gt;"));
        assert!(tree(&s).is_ok());
    }

    #[test]
    fn achievements_from_badges_keep_only_earned_ones_gold_first() {
        let mut stats = crate::badges::Stats::default();
        for (k, v) in [("rank", 386_200_000), ("streak", 2), ("skills", 13), ("subagents", 102), ("busiest_day", 212_500_000)] {
            stats.0.insert(k, v);
        }
        let got = achievements(&crate::badges::badges(&stats));
        assert_eq!(got, vec![award("Busiest day", "212.5M", Medal::Gold), award("Skills", "13", Medal::Silver), award("Subagents", "102", Medal::Silver)]);
        stats.0.insert("streak", 7);
        assert_eq!(achievements(&crate::badges::badges(&stats))[1], award("Streak", "7 days", Medal::Silver), "ties keep the badge order");
    }

    #[test]
    fn write_at_creates_the_folder_and_the_file() {
        let dir = std::env::temp_dir().join(format!("marsad-card-{}", std::process::id()));
        let path = dir.join("nested").join("card.png");
        write_at(&real(), &path).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), png(&real()).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "writes sample cards to $CARD_SAMPLES"]
    fn sample_cards() {
        let dir = std::path::PathBuf::from(std::env::var("CARD_SAMPLES").expect("set CARD_SAMPLES"));
        let mut cards = vec![real()];
        let samples: [(u64, u64, u64); 7] = [(480_000, 610_000, 640_000), (4_100_000, 6_900_000, 7_400_000), (14_500_000, 31_000_000, 38_000_000), (41_000_000, 120_000_000, 162_000_000), (98_000_000, 310_000_000, 480_000_000), (380_000_000, 1_250_000_000, 2_600_000_000), (1_400_000_000, 5_800_000_000, 12_300_000_000)];
        let awards = [
            vec![],
            vec![award("Streak", "3 days", Medal::Bronze)],
            vec![award("Skills", "4", Medal::Bronze), award("Streak", "3 days", Medal::Bronze)],
            vec![award("Busiest day", "18.4M", Medal::Bronze), award("Skills", "11", Medal::Silver), award("Subagents", "36", Medal::Bronze)],
            vec![award("Busiest day", "64M", Medal::Silver), award("Skills", "13", Medal::Silver), award("Subagents", "102", Medal::Silver), award("Streak", "9 days", Medal::Silver)],
            vec![award("Busiest day", "230M", Medal::Gold), award("Skills", "27", Medal::Gold), award("Subagents", "640", Medal::Silver), award("Streak", "12 days", Medal::Silver)],
            vec![award("Busiest day", "1.1B", Medal::Gold), award("Skills", "41", Medal::Gold), award("Subagents", "2.4k", Medal::Gold), award("Streak", "48 days", Medal::Gold)],
        ];
        for (tier, ((week, month, all_time), achievements)) in samples.into_iter().zip(awards).enumerate() {
            let name = (tier != 0).then(|| "Forhad Hosain".to_string());
            cards.push(CardData { tier, rank: NAMES[tier].into(), name, all_time, week, month, daily_average: month / 30, achievements, date: "2026-10-24".into() });
        }
        // Every card, and a contact sheet of them at half size, two across.
        let mut sheet = tiny_skia::Pixmap::new(WIDTH, HEIGHT * 2).unwrap();
        for (i, d) in cards.iter().enumerate() {
            let name = if i == 0 { "card-real.png".into() } else { format!("card-tier{}.png", d.tier) };
            write_at(d, &dir.join(name)).unwrap();
            let card = decode(&png(d).unwrap());
            let at = tiny_skia::Transform::from_row(0.5, 0.0, 0.0, 0.5, (i % 2) as f32 * WIDTH as f32 / 2.0, (i / 2) as f32 * HEIGHT as f32 / 2.0);
            sheet.draw_pixmap(0, 0, card.as_ref(), &tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() }, at, None);
        }
        sheet.save_png(dir.join("contact-sheet.png")).unwrap();
        write_at(&uniform(5, 999_900_000_000_000), &dir.join("card-extreme.png")).unwrap();
    }
}
