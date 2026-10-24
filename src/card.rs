//! The share card: a 1200×627 PNG with the rank badge, the rank and the token stats, drawn as SVG and rasterized by resvg with a bundled font.
// used by the share flow (U5)
#![allow(dead_code)]

use crate::badge_art::{self, Rgb};
use crate::model::fmt_tokens;
use anyhow::{Context, Result};
use resvg::{tiny_skia, usvg};
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

/// What the card shows, frozen once per share.
#[derive(Debug, Clone, PartialEq)]
pub struct CardData {
    /// The rank's tier, Novice (0) to Grandmaster (6).
    pub tier: usize,
    pub rank: String,
    pub today: u64,
    pub week: u64,
    pub month: u64,
    pub all_time: u64,
    /// The per-day average over the last 30 days.
    pub daily_average: u64,
    /// The snapshot day, `YYYY-MM-DD`.
    pub date: String,
}

/// The text column on the right: its left edge, its width, and the width of each of its three stat columns.
const TEXT_X: f64 = 620.0;
const TEXT_W: f64 = 520.0;
const COL_W: f64 = 180.0;

/// A token count for the card: the dashboard's format, with billions and trillions so big totals stay short.
pub fn fmt_count(n: u64) -> String {
    match n {
        0..1_000_000_000 => fmt_tokens(n),
        1_000_000_000..1_000_000_000_000 => format!("{:.1}B", n as f64 / 1e9),
        _ => format!("{:.1}T", n as f64 / 1e12),
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

/// The card as an SVG document.
pub fn svg(d: &CardData) -> String {
    let p = badge_art::badge(d.tier).palette;
    let white = (0xff, 0xff, 0xff);
    let bg = mix(p.band, (0, 0, 0), 0.25);
    let glow = p.ground;
    let accent = mix(p.line, white, 0.15);
    let text = (0xf4, 0xf6, 0xf8);
    let muted = mix(p.line, bg, 0.38);
    let rule = mix(p.line, bg, 0.82);
    let mut s = String::new();
    let _ = write!(s, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{HEIGHT}\" viewBox=\"0 0 {WIDTH} {HEIGHT}\" font-family=\"{FAMILY}\">");
    let _ = write!(s, "<defs><radialGradient id=\"glow\" cx=\"300\" cy=\"313\" r=\"420\" gradientUnits=\"userSpaceOnUse\"><stop offset=\"0\" stop-color=\"{}\" stop-opacity=\"0.9\"/><stop offset=\"1\" stop-color=\"{}\" stop-opacity=\"0\"/></radialGradient></defs>", hex(glow), hex(glow));
    let _ = write!(s, "<rect width=\"{WIDTH}\" height=\"{HEIGHT}\" fill=\"{}\"/><rect width=\"{WIDTH}\" height=\"{HEIGHT}\" fill=\"url(#glow)\"/>", hex(bg));
    // A thin bar in the tier's hue along the top edge.
    let _ = write!(s, "<rect width=\"{WIDTH}\" height=\"6\" fill=\"{}\"/>", hex(mix(p.rosette, p.line, 0.35)));
    // The badge, with a faint halo ring.
    let _ = write!(s, "<circle cx=\"300\" cy=\"313\" r=\"226\" fill=\"none\" stroke=\"{}\" stroke-width=\"2\" stroke-opacity=\"0.35\"/>", hex(p.line));
    s.push_str(&badge_art::svg(d.tier, 300.0, 313.0, 210.0));
    let label = |s: &mut String, id: &str, x: f64, y: f64, t: &str| {
        let _ = write!(s, "<text id=\"{id}\" x=\"{x}\" y=\"{y}\" font-size=\"17\" font-family=\"{BOLD}\" letter-spacing=\"2.4\" fill=\"{}\">{}</text>", hex(muted), escape(&t.to_uppercase()));
    };
    let tier = d.tier.min(badge_art::TIERS - 1) + 1;
    label(&mut s, "kicker", TEXT_X, 150.0, &format!("Claude Code rank \u{b7} {tier} of {}", badge_art::TIERS));
    let _ = write!(s, "<text id=\"rank\" x=\"{TEXT_X}\" y=\"234\" font-size=\"80\" font-family=\"{BOLD}\" letter-spacing=\"-1.5\" fill=\"{}\">{}</text>", hex(text), escape(&d.rank));
    let _ = write!(s, "<text id=\"all-time\" x=\"{TEXT_X}\" y=\"286\" font-size=\"28\" fill=\"{}\"><tspan font-family=\"{BOLD}\" fill=\"{}\">{}</tspan> tokens all time</text>", hex(muted), hex(accent), fmt_count(d.all_time));
    let _ = write!(s, "<rect x=\"{TEXT_X}\" y=\"330\" width=\"{TEXT_W}\" height=\"1.5\" fill=\"{}\"/>", hex(rule));
    let cols = [("today", "Today", d.today, None), ("week", "Last 7 days", d.week, None), ("month", "Last 30 days", d.month, Some(d.daily_average))];
    for (i, (id, name, value, avg)) in cols.into_iter().enumerate() {
        let x = TEXT_X + COL_W * i as f64;
        label(&mut s, &format!("{id}-label"), x, 380.0, name);
        let _ = write!(s, "<text id=\"{id}\" x=\"{x}\" y=\"432\" font-size=\"40\" font-family=\"{BOLD}\" letter-spacing=\"-0.5\" fill=\"{}\">{}</text>", hex(text), fmt_count(value));
        if let Some(avg) = avg {
            let _ = write!(s, "<text id=\"{id}-avg\" x=\"{x}\" y=\"466\" font-size=\"19\" fill=\"{}\">{} / day</text>", hex(muted), fmt_count(avg));
        }
    }
    let _ = write!(s, "<rect x=\"{TEXT_X}\" y=\"520\" width=\"{TEXT_W}\" height=\"1.5\" fill=\"{}\"/>", hex(rule));
    let _ = write!(s, "<text id=\"date\" x=\"{TEXT_X}\" y=\"560\" font-size=\"18\" fill=\"{}\">{}</text>", hex(muted), escape(&fmt_date(&d.date)));
    let _ = write!(s, "<text id=\"footer\" x=\"{}\" y=\"560\" font-size=\"18\" text-anchor=\"end\" fill=\"{}\">via <tspan font-family=\"{BOLD}\">marsad</tspan></text>", TEXT_X + TEXT_W, hex(muted));
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

    fn real() -> CardData {
        CardData { tier: 4, rank: "Expert".into(), today: 197_000_000, week: 198_000_000, month: 340_000_000, all_time: 344_000_000, daily_average: 11_300_000, date: "2026-10-24".into() }
    }

    fn uniform(tier: usize, n: u64) -> CardData {
        CardData { tier, rank: NAMES[tier].into(), today: n, week: n, month: n, all_time: n, daily_average: n, date: "2026-10-24".into() }
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
        let mut bright = 0;
        for y in y0..y1 {
            for x in x0..x1 {
                let (a, b) = (with.pixel(x, y).unwrap(), without.pixel(x, y).unwrap());
                if a != b {
                    changed += 1;
                }
                if a.red() > 200 && a.green() > 200 && a.blue() > 200 {
                    bright += 1;
                }
            }
        }
        assert!(changed > 1000, "only {changed} pixels changed under the rank name");
        assert!(bright > 500, "only {bright} light pixels in the rank name");
    }

    #[test]
    fn semibold_resolves_to_its_own_face() {
        // The heavier face shows as a wider run of the same text.
        let width = |family: &str| {
            let s = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"800\" height=\"100\"><text id=\"t\" x=\"0\" y=\"80\" font-size=\"60\" font-family=\"{family}\">Grandmaster</text></svg>");
            tree(&s).unwrap().node_by_id("t").unwrap().abs_bounding_box().width()
        };
        assert!(width(BOLD) > width(FAMILY) + 5.0, "SemiBold {} vs Regular {}", width(BOLD), width(FAMILY));
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
                let fits = |id: &str, left: f64, width: f64| {
                    let b = t.node_by_id(id).unwrap_or_else(|| panic!("no {id}")).abs_bounding_box();
                    assert!(b.left() as f64 >= left - 8.0 && b.right() as f64 <= left + width, "tier {tier} n {n}: {id} {b:?} outside {left}+{width}");
                };
                for id in ["kicker", "rank", "all-time", "date", "footer"] {
                    fits(id, TEXT_X, TEXT_W);
                }
                for (i, id) in ["today", "week", "month"].iter().enumerate() {
                    let left = TEXT_X + COL_W * i as f64;
                    // A column leaves a gutter before the next one; the last may run to the right edge.
                    let width = if i == 2 { TEXT_W - COL_W * 2.0 } else { COL_W - 16.0 };
                    fits(id, left, width);
                    fits(&format!("{id}-label"), left, width);
                }
                fits("month-avg", TEXT_X + COL_W * 2.0, TEXT_W - COL_W * 2.0);
                let date = t.node_by_id("date").unwrap().abs_bounding_box();
                let footer = t.node_by_id("footer").unwrap().abs_bounding_box();
                assert!(date.right() < footer.left(), "date and footer overlap");
            }
        }
    }

    #[test]
    fn counts_switch_to_billions_and_trillions() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(11_300_000), "11.3M");
        assert_eq!(fmt_count(344_000_000), "344.0M");
        assert_eq!(fmt_count(12_300_000_000), "12.3B");
        assert_eq!(fmt_count(4_200_000_000_000), "4.2T");
    }

    #[test]
    fn card_shows_every_stat_and_the_date() {
        let s = svg(&real());
        for want in ["Expert", "197.0M", "198.0M", "340.0M", "11.3M / day", "344.0M", "24 October 2026", "5 OF 7", "marsad"] {
            assert!(s.contains(want), "card lacks {want}");
        }
    }

    #[test]
    fn rank_text_is_escaped() {
        let s = svg(&CardData { rank: "A<b>&\"c".into(), ..real() });
        assert!(s.contains("A&lt;b&gt;&amp;&quot;c"));
        assert!(tree(&s).is_ok());
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
        let samples: [(u64, u64, u64, u64); 7] = [(120_000, 480_000, 610_000, 640_000), (1_800_000, 4_100_000, 6_900_000, 7_400_000), (3_200_000, 14_500_000, 31_000_000, 38_000_000), (9_400_000, 41_000_000, 120_000_000, 162_000_000), (22_000_000, 98_000_000, 310_000_000, 480_000_000), (64_000_000, 380_000_000, 1_250_000_000, 2_600_000_000), (210_000_000, 1_400_000_000, 5_800_000_000, 12_300_000_000)];
        for (tier, (today, week, month, all_time)) in samples.into_iter().enumerate() {
            cards.push(CardData { tier, rank: NAMES[tier].into(), today, week, month, all_time, daily_average: month / 30, date: "2026-10-24".into() });
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
    }
}
