//! A backend wrapper that keeps rows holding complex-script text (Bengali, emoji sequences) aligned.
//!
//! ratatui sizes a grapheme with `unicode-width`, which disagrees with terminals on clusters such as
//! Bengali vowel signs and conjuncts. The crossterm backend writes neighbouring cells without moving
//! the cursor, so one mismeasured cluster shifts the rest of its row, and since ratatui only redraws
//! what changed, the shifted glyphs stay on screen. This wrapper keeps its own copy of the screen and,
//! whenever a row that holds (or just held) such a cluster changes, rewrites that whole row with the
//! cursor placed explicitly after each cluster.

use ratatui::backend::{Backend, ClearType, WindowSize};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::{Position, Rect, Size};
use ratatui::text::Span;

pub struct Aligned<B> {
    inner: B,
    screen: Buffer,
}

impl<B: Backend> Aligned<B> {
    pub fn new(inner: B) -> Self {
        Self { inner, screen: Buffer::empty(Rect::ZERO) }
    }
}

/// A grapheme of several code points, whose drawn width the terminal may not agree on.
fn complex(cell: &Cell) -> bool {
    cell.symbol().chars().nth(1).is_some()
}

fn width(cell: &Cell) -> u16 {
    Span::raw(cell.symbol()).width().max(1) as u16
}

fn row_has_complex(screen: &Buffer, y: u16) -> bool {
    (0..screen.area.width).any(|x| complex(&screen[(x, y)]))
}

/// Draws `cells`, starting a fresh cursor position before and after every complex cluster, and
/// blanking the cells a cluster covers (up to the screen's right edge, `right`) first so a terminal
/// that draws it narrower leaves no remains.
fn draw_placed<B: Backend>(inner: &mut B, cells: Vec<(u16, u16, Cell)>, right: u16) -> Result<(), B::Error> {
    let mut run: Vec<(u16, u16, Cell)> = Vec::new();
    for (x, y, cell) in cells {
        if complex(&cell) {
            let mut blank = cell.clone();
            blank.set_symbol(" ");
            run.extend((x..(x + width(&cell)).min(right)).map(|bx| (bx, y, blank.clone())));
            inner.draw(run.iter().map(|(x, y, c)| (*x, *y, c)))?;
            run.clear();
            inner.draw(std::iter::once((x, y, &cell)))?;
        } else {
            run.push((x, y, cell));
        }
    }
    inner.draw(run.iter().map(|(x, y, c)| (*x, *y, c)))
}

impl<B: Backend> Backend for Aligned<B> {
    type Error = B::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let size = self.inner.size()?;
        let area = Rect::new(0, 0, size.width, size.height);
        if self.screen.area != area {
            self.screen = Buffer::empty(area);
        }
        let had: Vec<bool> = (0..area.height).map(|y| row_has_complex(&self.screen, y)).collect();
        let diff: Vec<(u16, u16, Cell)> = content.map(|(x, y, c)| (x, y, c.clone())).collect();
        for (x, y, cell) in &diff {
            if !area.contains(Position::new(*x, *y)) {
                continue;
            }
            self.screen[(*x, *y)] = cell.clone();
            // The cells a wide grapheme covers are blank in ratatui's buffer but never sent.
            for cx in (x + 1..x + width(cell)).filter(|cx| *cx < area.width) {
                self.screen[(cx, *y)].reset();
            }
        }
        let rewrite: Vec<bool> = (0..area.height)
            .map(|y| had[y as usize] || row_has_complex(&self.screen, y))
            .collect();

        let mut cells = Vec::with_capacity(diff.len());
        let mut done = vec![false; area.height as usize];
        for (x, y, cell) in diff {
            if !rewrite.get(y as usize).copied().unwrap_or(false) {
                cells.push((x, y, cell));
            } else if !std::mem::replace(&mut done[y as usize], true) {
                let mut x = 0;
                while x < area.width {
                    let cell = self.screen[(x, y)].clone();
                    let w = width(&cell);
                    cells.push((x, y, cell));
                    x += w;
                }
            }
        }
        draw_placed(&mut self.inner, cells, area.width)
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        self.inner.get_cursor_position()
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.screen.reset();
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> Result<(), Self::Error> {
        if clear_type == ClearType::All {
            self.screen.reset();
        }
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> Result<Size, Self::Error> {
        self.inner.size()
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;

    /// Records where each draw call starts, since that is where the real backend moves the cursor.
    #[derive(Default)]
    struct Log {
        size: (u16, u16),
        calls: Vec<Vec<(u16, u16, String)>>,
    }

    impl Backend for Log {
        type Error = std::io::Error;
        fn draw<'a, I: Iterator<Item = (u16, u16, &'a Cell)>>(&mut self, content: I) -> std::io::Result<()> {
            let call: Vec<_> = content.map(|(x, y, c)| (x, y, c.symbol().to_string())).collect();
            if !call.is_empty() {
                self.calls.push(call);
            }
            Ok(())
        }
        fn hide_cursor(&mut self) -> std::io::Result<()> { Ok(()) }
        fn show_cursor(&mut self) -> std::io::Result<()> { Ok(()) }
        fn get_cursor_position(&mut self) -> std::io::Result<Position> { Ok(Position::ORIGIN) }
        fn set_cursor_position<P: Into<Position>>(&mut self, _: P) -> std::io::Result<()> { Ok(()) }
        fn clear(&mut self) -> std::io::Result<()> { Ok(()) }
        fn clear_region(&mut self, _: ClearType) -> std::io::Result<()> { Ok(()) }
        fn size(&self) -> std::io::Result<Size> { Ok(Size::new(self.size.0, self.size.1)) }
        fn window_size(&mut self) -> std::io::Result<WindowSize> { unimplemented!() }
        fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
    }

    fn frame(width: u16, lines: &[&str]) -> Buffer {
        let mut b = Buffer::empty(Rect::new(0, 0, width, lines.len() as u16));
        for (y, l) in lines.iter().enumerate() {
            b.set_string(0, y as u16, l, ratatui::style::Style::new());
        }
        b
    }

    fn draw(a: &mut Aligned<Log>, prev: &Buffer, next: &Buffer) {
        a.inner.calls.clear();
        a.draw(prev.diff(next).into_iter()).unwrap();
    }

    #[test]
    fn plain_rows_pass_through_as_one_run() {
        let mut a = Aligned::new(Log { size: (6, 1), ..Default::default() });
        draw(&mut a, &frame(6, &[""]), &frame(6, &["abc"]));
        assert_eq!(a.inner.calls.len(), 1);
    }

    #[test]
    fn the_cell_after_a_cluster_starts_a_new_cursor_position() {
        let mut a = Aligned::new(Log { size: (8, 1), ..Default::default() });
        draw(&mut a, &frame(8, &[""]), &frame(8, &["লাx"]));
        let starts: Vec<_> = a.inner.calls.iter().map(|c| (c[0].0, c[0].2.clone())).collect();
        assert!(starts.contains(&(0, "লা".to_string())), "{starts:?}");
        assert!(starts.contains(&(1, "x".to_string())), "{starts:?}");
    }

    #[test]
    fn a_row_that_held_a_cluster_is_rewritten_whole_when_it_changes() {
        let mut a = Aligned::new(Log { size: (6, 1), ..Default::default() });
        let first = frame(6, &["লাabcd"]);
        draw(&mut a, &frame(6, &[""]), &first);
        // Only the last cell changes, but the row's earlier cells may sit shifted on screen.
        draw(&mut a, &first, &frame(6, &["xyabce"]));
        let sent: String = a.inner.calls.iter().flatten().map(|(_, _, s)| s.as_str()).collect();
        assert_eq!(sent, "xyabce");
    }

    #[test]
    fn rows_without_clusters_are_left_alone() {
        let mut a = Aligned::new(Log { size: (4, 2), ..Default::default() });
        let first = frame(4, &["লা", "ab"]);
        draw(&mut a, &frame(4, &["", ""]), &first);
        draw(&mut a, &first, &frame(4, &["লা", "ac"]));
        assert_eq!(a.inner.calls, vec![vec![(1, 1, "c".to_string())]]);
    }

    #[test]
    fn blanks_stop_at_the_right_edge() {
        let mut a = Aligned::new(Log { size: (2, 1), ..Default::default() });
        let mut wide = Cell::default();
        wide.set_symbol("স্ট্রিং");
        a.draw(std::iter::once((1, 0, &wide))).unwrap();
        assert!(a.inner.calls.iter().flatten().all(|(x, _, _)| *x < 2), "{:?}", a.inner.calls);
    }

    #[test]
    fn works_with_ratatui_terminal() {
        let mut t = ratatui::Terminal::new(Aligned::new(TestBackend::new(10, 1))).unwrap();
        t.draw(|f| f.render_widget("ক্ষমা ok", f.area())).unwrap();
    }
}
