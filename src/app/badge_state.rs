//! The dashboard's badge view data, its footer announcements and the share view's entry point.

use super::{App, View};
use crate::badges::Badges;
use crate::share_view::ShareView;
use std::time::{Duration, Instant};

/// The badge view's data, loaded after `B` and after each sync, never while drawing.
#[derive(Default)]
pub struct BadgeState {
    pub badges: Option<Badges>,
    /// The rank's badge art, kept for the tier it was built for.
    pub art: Option<(usize, crate::badge_art::Badge)>,
    pub error: Option<String>,
    /// Load before the next frame.
    pub(super) due: bool,
}

/// How long a drawn announcement stays in the footer.
const ANNOUNCE_FOR: Duration = Duration::from_secs(10);

/// One footer note for levels that rose: shown only on the list and badge views and never
/// over the share view, it outlives key presses, stays 10 s or until Esc, and is recorded as
/// announced once it has been drawn.
#[derive(Default)]
pub struct Announcement {
    pending: Option<(String, Badges)>,
    drawn_at: Option<Instant>,
}

impl Announcement {
    /// A note for levels that rose. The same note offered again (a sync before it was drawn) is kept as is.
    pub fn offer(&mut self, text: String, levels: Badges) {
        if self.pending.as_ref().is_some_and(|(t, _)| *t == text) {
            return;
        }
        self.pending = Some((text, levels));
        self.drawn_at = None;
    }

    /// The note, if this screen may show it.
    pub fn text(&self, view: View, sharing: bool) -> Option<&str> {
        let here = matches!(view, View::List | View::Badges) && !sharing;
        self.pending.as_ref().filter(|_| here).map(|(t, _)| t.as_str())
    }

    /// After a frame: the first time the note was on screen, returns the levels to record; once it
    /// has been up for `ANNOUNCE_FOR`, it goes.
    pub fn drawn(&mut self, now: Instant, view: View, sharing: bool) -> Option<Badges> {
        self.text(view, sharing)?;
        match self.drawn_at {
            None => {
                self.drawn_at = Some(now);
                self.pending.as_ref().map(|(_, b)| b.clone())
            }
            Some(at) => {
                if now.duration_since(at) >= ANNOUNCE_FOR {
                    self.pending = None;
                }
                None
            }
        }
    }

    /// Esc on a note that is up dismisses it; returns whether it did.
    pub fn dismiss(&mut self, view: View, sharing: bool) -> bool {
        let up = self.drawn_at.is_some() && self.text(view, sharing).is_some();
        if up {
            self.pending = None;
        }
        up
    }
}

impl App {
    /// Reads the shown badge values (never lower than the stored best) for the header's all-time
    /// figure, and rebuilds the art when the tier changed.
    pub(super) fn load_badges(&mut self) -> Option<Badges> {
        let total = self.counter.totals().1;
        let loaded = crate::store::open_readonly().and_then(|conn| crate::badges::shown(&conn, total)).map(|s| crate::badges::badges(&s));
        match loaded {
            Ok(b) => {
                let tier = b.rank.tier;
                if self.badges.art.as_ref().is_none_or(|(t, _)| *t != tier) {
                    self.badges.art = Some((tier, crate::badge_art::badge(tier)));
                }
                self.badges.badges = Some(b.clone());
                self.badges.error = None;
                Some(b)
            }
            Err(e) => {
                self.badges.error = Some(format!("{e:#}"));
                None
            }
        }
    }

    /// After a sync: fresh badge values, and a footer note for any level that rose.
    pub(super) fn after_sync(&mut self) {
        if let Some(b) = self.load_badges()
            && let Ok(Some(text)) = crate::badges::pending_announcement(&b)
        {
            self.announcement.offer(text, b);
        }
    }

    /// After a frame: records an announcement the first time it was drawn.
    pub(super) fn after_draw(&mut self) {
        if let Some(b) = self.announcement.drawn(Instant::now(), self.view, self.share.is_some())
            && let Err(e) = crate::badges::mark_announced(&b)
        {
            self.error = Some(format!("couldn't record the announcement: {e:#}"));
        }
    }

    /// Opens the share view: it syncs, freezes the stats and drafts the line.
    pub(super) fn start_share(&mut self) {
        self.share = Some(ShareView::new(crate::share_view::Deps::real(self.counter.totals().1)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn levels(rank: i64) -> Badges {
        let mut stats = crate::badges::Stats::default();
        stats.0.insert(crate::badges::RANK_KEY, rank);
        crate::badges::badges(&stats)
    }

    #[test]
    fn an_announcement_waits_for_the_share_view_outlives_keys_and_is_recorded_once_drawn() {
        let mut a = Announcement::default();
        let now = Instant::now();
        a.offer("rank up: Expert · new: gold streak, silver skills".into(), levels(300_000_000));
        // Offered while sharing: held back, and not recorded.
        assert_eq!(a.text(View::List, true), None);
        assert_eq!(a.drawn(now, View::List, true), None);
        assert_eq!(a.text(View::History, false), None, "only the list and badge views show it");
        // After the share closes it shows, and is recorded the first time it is drawn, once.
        assert_eq!(a.text(View::List, false), Some("rank up: Expert · new: gold streak, silver skills"));
        assert_eq!(a.drawn(now, View::List, false).map(|b| b.rank.tier), Some(4));
        assert_eq!(a.drawn(now + Duration::from_secs(1), View::Badges, false), None, "recorded only once");
        // The same note offered again by a later sync doesn't restart it.
        a.offer("rank up: Expert · new: gold streak, silver skills".into(), levels(300_000_000));
        assert_eq!(a.drawn(now + Duration::from_secs(2), View::List, false), None);
        assert!(a.text(View::List, false).is_some(), "still up after more frames and keys");
        a.drawn(now + ANNOUNCE_FOR, View::List, false);
        assert_eq!(a.text(View::List, false), None, "gone after 10 s");
    }
}
