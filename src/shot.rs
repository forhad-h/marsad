//! The badge view as a PNG: drawn off screen at a fixed size, then each cell painted the way a dark
//! terminal shows it. Braille cells become dots; text uses a bundled monospace font, so the picture
//! looks the same on every machine and in every terminal.

use crate::badges::Badges;
use anyhow::{Context, Result};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;
use resvg::{tiny_skia, usvg};
use std::fmt::Write;
use std::path::{Path, PathBuf};

const FONT_MONO: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");
const FONT_MONO_BOLD: &[u8] = include_bytes!("../assets/fonts/DejaVuSansMono-Bold.ttf");
const FAMILY: &str = "DejaVu Sans Mono";

/// The view's size in cells: wide and tall enough for the art beside the text.
pub const COLS: u16 = 110;
pub const ROWS: u16 = 30;

const FONT_SIZE: f64 = 16.0;
/// DejaVu Sans Mono's advance (1233/2048 em), so runs of text land on the cell grid.
const CELL_W: f64 = FONT_SIZE * 1233.0 / 2048.0;
const CELL_H: f64 = 20.0;
const PAD: f64 = 24.0;
/// Pixels per SVG unit, for a sharp image on high-density screens.
const SCALE: f32 = 2.0;

const BACKGROUND: &str = "#1b1e26";
const FOREGROUND: &str = "#d4d8e0";

/// The badge view drawn into a buffer of `COLS`×`ROWS` cells.
fn buffer(b: &Badges) -> Result<Buffer> {
    let mut terminal = Terminal::new(TestBackend::new(COLS, ROWS)).context("off-screen terminal")?;
    terminal.draw(|f| crate::ui::draw_badges(f, f.area(), Some(b), CELL_H / CELL_W)).context("drawing the badge view")?;
    Ok(terminal.backend().buffer().clone())
}

/// A cell colour as the default dark terminal palette shows it; None for the terminal's own.
fn color(c: Color) -> Option<String> {
    let hex = match c {
        Color::Reset => return None,
        Color::Rgb(r, g, b) => return Some(format!("#{r:02x}{g:02x}{b:02x}")),
        Color::Black => "#3e4451",
        Color::Red => "#e06c75",
        Color::Green => "#98c379",
        Color::Yellow => "#e5c07b",
        Color::Blue => "#61afef",
        Color::Magenta => "#c678dd",
        Color::Cyan => "#56b6c2",
        Color::Gray => "#abb2bf",
        Color::DarkGray => "#5c6370",
        Color::LightRed => "#ef8891",
        Color::LightGreen => "#b5e09b",
        Color::LightYellow => "#f0d197",
        Color::LightBlue => "#8cc8ff",
        Color::LightMagenta => "#e09cf0",
        Color::LightCyan => "#7fd3dc",
        Color::White => "#ffffff",
        Color::Indexed(_) => FOREGROUND,
    };
    Some(hex.to_string())
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A cell's colours as drawn: (foreground, background), swapped when reversed.
fn colors(cell: &ratatui::buffer::Cell) -> (String, Option<String>) {
    let (fg, bg) = (color(cell.fg), color(cell.bg));
    if cell.modifier.contains(Modifier::REVERSED) {
        (bg.unwrap_or_else(|| BACKGROUND.to_string()), Some(fg.unwrap_or_else(|| FOREGROUND.to_string())))
    } else {
        (fg.unwrap_or_else(|| FOREGROUND.to_string()), bg)
    }
}

/// The text attributes a run shares: colour, bold, dim and italic.
fn text_style(cell: &ratatui::buffer::Cell) -> (String, bool, bool, bool) {
    let m = cell.modifier;
    (colors(cell).0, m.contains(Modifier::BOLD), m.contains(Modifier::DIM), m.contains(Modifier::ITALIC))
}

/// The dots of a braille cell, as (column 0-1, row 0-3).
fn braille_dots(c: char) -> Option<Vec<(u8, u8)>> {
    let bits = (c as u32).checked_sub(0x2800).filter(|b| *b <= 0xff)?;
    const DOTS: [(u32, (u8, u8)); 8] =
        [(0x01, (0, 0)), (0x02, (0, 1)), (0x04, (0, 2)), (0x40, (0, 3)), (0x08, (1, 0)), (0x10, (1, 1)), (0x20, (1, 2)), (0x80, (1, 3))];
    Some(DOTS.iter().filter(|(bit, _)| bits & bit != 0).map(|(_, at)| *at).collect())
}

/// The buffer as SVG: every cell's background, one text run per stretch of same-styled cells, and a
/// circle per braille dot.
fn svg(buf: &Buffer) -> String {
    let (w, h) = (buf.area.width, buf.area.height);
    let (width, height) = (PAD * 2.0 + CELL_W * w as f64, PAD * 2.0 + CELL_H * h as f64);
    let mut s = String::new();
    let _ = write!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\">\
         <rect width=\"100%\" height=\"100%\" rx=\"12\" fill=\"{BACKGROUND}\"/>"
    );
    for y in 0..h {
        let top = PAD + CELL_H * y as f64;
        for x in 0..w {
            if let Some(bg) = colors(&buf[(x, y)]).1 {
                let _ = write!(s, "<rect x=\"{:.2}\" y=\"{top:.2}\" width=\"{CELL_W:.2}\" height=\"{CELL_H}\" fill=\"{bg}\"/>", PAD + CELL_W * x as f64);
            }
        }
    }
    for y in 0..h {
        let top = PAD + CELL_H * y as f64;
        let baseline = top + CELL_H * 0.75;
        let mut x = 0;
        while x < w {
            let cell = &buf[(x, y)];
            let style = text_style(cell);
            let (fg, bold, dim, italic) = style.clone();
            let ch = cell.symbol().chars().next().unwrap_or(' ');
            if let Some(dots) = braille_dots(ch) {
                for (dx, dy) in dots {
                    let cx = PAD + CELL_W * (x as f64 + 0.27 + 0.46 * dx as f64);
                    let cy = top + CELL_H * (0.14 + 0.24 * dy as f64);
                    let _ = write!(s, "<circle cx=\"{cx:.2}\" cy=\"{cy:.2}\" r=\"{:.2}\" fill=\"{fg}\"/>", CELL_W * 0.15);
                }
                x += 1;
                continue;
            }
            // A run of cells with this cell's style.
            let start = x;
            let mut text = String::new();
            while x < w {
                let c = &buf[(x, y)];
                if text_style(c) != style || c.symbol().chars().next().and_then(braille_dots).is_some() {
                    break;
                }
                text.push_str(if c.symbol().is_empty() { " " } else { c.symbol() });
                x += 1;
            }
            if text.trim().is_empty() {
                continue;
            }
            let _ = write!(
                s,
                "<text x=\"{:.2}\" y=\"{baseline:.2}\" font-size=\"{FONT_SIZE}\" fill=\"{fg}\"{}{}{} xml:space=\"preserve\">{}</text>",
                PAD + CELL_W * start as f64,
                if bold { " font-weight=\"bold\"" } else { "" },
                if dim { " fill-opacity=\"0.6\"" } else { "" },
                if italic { " font-style=\"italic\"" } else { "" },
                escape(&text)
            );
        }
    }
    s.push_str("</svg>");
    s
}

fn options() -> usvg::Options<'static> {
    static FONTS: std::sync::OnceLock<std::sync::Arc<usvg::fontdb::Database>> = std::sync::OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_font_data(FONT_MONO.to_vec());
        db.load_font_data(FONT_MONO_BOLD.to_vec());
        std::sync::Arc::new(db)
    });
    usvg::Options { font_family: FAMILY.into(), fontdb: fonts.clone(), ..Default::default() }
}

/// The badge view as PNG bytes.
pub fn png(b: &Badges) -> Result<Vec<u8>> {
    let tree = usvg::Tree::from_str(&svg(&buffer(b)?), &options()).context("badge view svg")?;
    let size = tree.size();
    let (w, h) = ((size.width() * SCALE).ceil() as u32, (size.height() * SCALE).ceil() as u32);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).context("badge view pixmap")?;
    resvg::render(&tree, tiny_skia::Transform::from_scale(SCALE, SCALE), &mut pixmap.as_mut());
    pixmap.encode_png().context("encoding the badge view png")
}

/// Where the picture goes: beside the share card (Downloads, else `<data_dir>/cards/`). Same day, same file.
pub fn path(downloads: Option<PathBuf>, data_dir: &Path, date: &str) -> PathBuf {
    crate::share::card_path(downloads, data_dir, date).with_file_name(format!("marsad-badge-{date}.png"))
}

pub fn default_path() -> PathBuf {
    path(dirs::download_dir(), &crate::data_dir(), &crate::store::day_n_ago(0))
}

/// Writes the badge view to `path`, creating its folder.
pub fn write_at(b: &Badges, path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("couldn't create {}", dir.display()))?;
    }
    std::fs::write(path, png(b)?).with_context(|| format!("couldn't write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::badges::{badges, Stats, RANK_KEY};
    use std::collections::BTreeMap;

    fn sample() -> Badges {
        badges(&Stats(BTreeMap::from([(RANK_KEY, 10_200_000_000), ("streak", 5), ("skills", 53), ("subagents", 1344), ("busiest_day", 1_000_000_000)])))
    }

    #[test]
    fn braille_cells_map_to_their_dots() {
        assert_eq!(braille_dots('⠁'), Some(vec![(0, 0)]));
        assert_eq!(braille_dots('⣿').map(|d| d.len()), Some(8));
        assert_eq!(braille_dots('⢀'), Some(vec![(1, 3)]));
        assert_eq!(braille_dots('a'), None);
    }

    #[test]
    fn the_svg_has_the_rank_text_and_the_badge_dots() {
        let s = svg(&buffer(&sample()).unwrap());
        assert!(s.contains(">Practitioner</text>"), "rank in bold text");
        assert!(s.contains("font-weight=\"bold\""));
        assert!(s.matches("<circle").count() > 500, "the art is drawn as dots");
        assert!(usvg::Tree::from_str(&s, &options()).is_ok());
    }

    #[test]
    fn backgrounds_fill_every_cell_and_reversed_or_dim_text_stays_visible() {
        use ratatui::style::Style;
        let mut buf = Buffer::empty(ratatui::layout::Rect::new(0, 0, 6, 1));
        buf.set_string(0, 0, "ab", Style::new().bg(Color::Rgb(1, 2, 3)));
        buf.set_string(2, 0, "cd", Style::new().fg(Color::Black).reversed());
        buf.set_string(4, 0, "ef", Style::new().dim());
        let s = svg(&buf);
        assert_eq!(s.matches("fill=\"#010203\"").count(), 2, "both cells of a styled span: {s}");
        assert_eq!(s.matches("fill=\"#3e4451\"/>").count(), 2, "reversed: the black foreground becomes the background");
        assert!(s.contains(&format!("fill=\"{BACKGROUND}\" xml:space=\"preserve\">cd<")), "{s}");
        assert!(s.contains("fill-opacity=\"0.6\" xml:space=\"preserve\">ef<"), "{s}");
    }

    #[test]
    fn png_is_a_sharp_image_of_the_view() {
        let png = png(&sample()).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        assert_eq!(width, ((PAD * 2.0 + CELL_W * COLS as f64) * SCALE as f64).ceil() as u32);
    }

    #[test]
    fn the_file_goes_to_downloads_else_the_data_dir() {
        assert_eq!(path(Some("/d".into()), Path::new("/m"), "2026-10-27"), Path::new("/d/marsad-badge-2026-10-27.png"));
        assert_eq!(path(None, Path::new("/m"), "2026-10-27"), Path::new("/m/cards/marsad-badge-2026-10-27.png"));
    }
}
