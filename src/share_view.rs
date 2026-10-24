//! The dashboard's share view: a self-contained state machine for drafting, reviewing, editing and
//! confirming a LinkedIn post. The app hands it keys and pastes before its own keys, and runs the
//! two steps that need the terminal itself: the clipboard copy and the `$EDITOR` round trip.

use crate::clipboard::Copied;
use crate::share::{self, Confirmed, Desktop, Draft, Launch, Note, Snapshot, Step};
use ratatui::crossterm::event::KeyCode;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};

/// LinkedIn's limit for a post.
pub const MAX_POST: usize = 3000;

type Snapshotter = Arc<dyn Fn() -> anyhow::Result<Snapshot> + Send + Sync>;
type Drafter = Arc<dyn Fn(&Snapshot, &AtomicBool) -> Draft + Send + Sync>;
type Runner = Arc<dyn Fn(&Step) -> Result<(), String> + Send + Sync>;
type CardPath = Arc<dyn Fn(&str) -> PathBuf + Send + Sync>;

/// Everything the share view reaches outside itself; the tests swap each part.
#[derive(Clone)]
pub struct Deps {
    /// Syncs and freezes the stats, once per share.
    pub snapshot: Snapshotter,
    pub draft: Drafter,
    /// Runs one desktop tool (open, reveal).
    pub run: Runner,
    pub desktop: Desktop,
    /// Where the card goes, for the snapshot's day.
    pub card: CardPath,
}

impl Deps {
    /// The real thing. `rank_total` is the header's all-time figure, which may include live usage
    /// the index hasn't caught up with yet.
    pub fn real(rank_total: i64) -> Self {
        Deps {
            snapshot: Arc::new(move || {
                crate::store::open_and_sync()?;
                let conn = crate::store::open_readonly()?;
                let total = crate::store::lifetime(&conn)?.total.max(rank_total);
                share::snapshot(&conn, &crate::store::day_n_ago(0), total, dirs::home_dir(), share::git_user_name())
            }),
            draft: Arc::new(share::draft),
            run: Arc::new(share::launch),
            desktop: Desktop::current(),
            card: Arc::new(share::default_card_path),
        }
    }
}

/// Where the share stands.
#[derive(Debug, PartialEq)]
pub enum Stage {
    /// Waiting for the snapshot, then the line.
    Drafting,
    Preview,
    /// Editing the line inline: the text and the cursor, in chars. Esc keeps the old line.
    Editing { text: String, cursor: usize },
    /// The card is being written, then copied, opened and revealed.
    Saving,
    Done(Confirmed),
    /// The stats couldn't be gathered; nothing can be shared.
    Failed(String),
}

/// What the app must do after a key.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// The share is over; drop the view.
    Close,
    /// Suspend the terminal, edit this text in `$EDITOR`, then hand the result to `edited`.
    EditPost(String),
}

/// What the drafting worker reports.
enum Drafted {
    Snapshot(Result<Arc<Snapshot>, String>),
    Line(Draft),
}

/// The confirm worker's channels: it asks the main thread to copy, waits for the answer, then reports.
struct Saving {
    copy: Receiver<String>,
    copied: Sender<Copied>,
    done: Receiver<Result<Confirmed, String>>,
}

pub struct ShareView {
    deps: Deps,
    pub stage: Stage,
    pub snapshot: Option<Arc<Snapshot>>,
    pub line: String,
    /// Why the line is the template, when it is.
    pub note: Option<Note>,
    /// The whole post as edited with `E`; None builds it from the line.
    pub edited_post: Option<String>,
    /// A failure to show above the keys: a regenerate that fell back, a card that couldn't be written.
    pub status: Option<String>,
    pub scroll: u16,
    /// The furthest the preview can scroll, set while drawing.
    pub max_scroll: Cell<u16>,
    cancel: Arc<AtomicBool>,
    drafts: Option<Receiver<Drafted>>,
    saving: Option<Saving>,
    /// A drafted line is being replaced (`r`), so a failure keeps the line already shown.
    regenerating: bool,
}

impl Drop for ShareView {
    fn drop(&mut self) {
        // Whatever closes the view, a running `claude` is killed.
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl ShareView {
    /// Opens the view and starts gathering the stats and drafting the line.
    pub fn new(deps: Deps) -> Self {
        let mut v = ShareView {
            deps,
            stage: Stage::Drafting,
            snapshot: None,
            line: String::new(),
            note: None,
            edited_post: None,
            status: None,
            scroll: 0,
            max_scroll: Cell::new(0),
            cancel: Arc::new(AtomicBool::new(false)),
            drafts: None,
            saving: None,
            regenerating: false,
        };
        v.start_draft();
        v
    }

    /// Drafts a line on a worker thread, first taking the snapshot if there is none yet.
    fn start_draft(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (deps, cancel, known) = (self.deps.clone(), self.cancel.clone(), self.snapshot.clone());
        std::thread::spawn(move || {
            let snapshot = match known {
                Some(s) => s,
                None => match (deps.snapshot)() {
                    Ok(s) => {
                        let s = Arc::new(s);
                        let _ = tx.send(Drafted::Snapshot(Ok(s.clone())));
                        s
                    }
                    Err(e) => {
                        let _ = tx.send(Drafted::Snapshot(Err(format!("{e:#}"))));
                        return;
                    }
                },
            };
            let _ = tx.send(Drafted::Line((deps.draft)(&snapshot, &cancel)));
        });
        self.drafts = Some(rx);
        self.stage = Stage::Drafting;
    }

    /// The post as it will be shared.
    pub fn post(&self) -> String {
        match (&self.edited_post, &self.snapshot) {
            (Some(text), _) => text.clone(),
            (None, Some(s)) => share::post(s, &self.line),
            (None, None) => String::new(),
        }
    }

    /// Collects what the workers finished. Returns text the main thread must put on the clipboard,
    /// then hand the result to `copied`.
    pub fn tick(&mut self) -> Option<String> {
        while let Some(rx) = &self.drafts {
            match rx.try_recv() {
                Ok(Drafted::Snapshot(Ok(s))) => self.snapshot = Some(s),
                Ok(Drafted::Snapshot(Err(e))) => {
                    self.drafts = None;
                    self.stage = Stage::Failed(format!("Couldn't gather today's stats: {e}"));
                }
                Ok(Drafted::Line(d)) => {
                    self.drafts = None;
                    self.drafted(d);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.drafts = None;
                    match &self.snapshot {
                        Some(s) => {
                            let d = Draft::template(s, Note::Failed("the drafting thread stopped".into()));
                            self.drafted(d);
                        }
                        None => self.stage = Stage::Failed("Couldn't gather today's stats: the worker stopped".into()),
                    }
                }
            }
        }
        let saving = self.saving.as_ref()?;
        if let Ok(text) = saving.copy.try_recv() {
            return Some(text);
        }
        match saving.done.try_recv() {
            Ok(Ok(done)) => {
                self.saving = None;
                self.stage = Stage::Done(done);
            }
            Ok(Err(e)) => {
                self.saving = None;
                self.stage = Stage::Preview;
                self.status = Some(format!("Couldn't save the card: {e}. Nothing was copied or opened."));
            }
            Err(TryRecvError::Disconnected) => {
                self.saving = None;
                self.stage = Stage::Preview;
                self.status = Some("Saving stopped unexpectedly; nothing was opened.".into());
            }
            Err(TryRecvError::Empty) => {}
        }
        None
    }

    fn drafted(&mut self, d: Draft) {
        if self.regenerating && d.note.is_some() {
            // Keep the line already shown; say why it didn't change.
            let why = d.note.as_ref().map(Note::message).unwrap_or_default();
            self.status = Some(format!("Couldn't regenerate, kept the line. {why}"));
        } else {
            self.line = d.line;
            self.note = d.note;
            self.edited_post = None;
        }
        self.regenerating = false;
        self.stage = Stage::Preview;
    }

    /// The clipboard result for the confirm worker.
    pub fn copied(&mut self, c: Copied) {
        if let Some(saving) = &self.saving {
            let _ = saving.copied.send(c);
        }
    }

    /// The post back from `$EDITOR`, or why it couldn't be edited.
    pub fn edited(&mut self, result: anyhow::Result<String>) {
        match result {
            Ok(text) => {
                let text = text.trim_end().to_string();
                if text.trim().is_empty() {
                    self.status = Some("The edited post was empty; kept the previous one.".into());
                } else if text != self.post() {
                    self.edited_post = Some(text);
                    self.status = None;
                }
            }
            Err(e) => self.status = Some(format!("Couldn't open the editor: {e:#}")),
        }
    }

    pub fn on_key(&mut self, code: KeyCode) -> Action {
        match &mut self.stage {
            Stage::Drafting => {
                if matches!(code, KeyCode::Esc | KeyCode::Char('q')) {
                    return Action::Close;
                }
            }
            Stage::Preview => match code {
                KeyCode::Esc | KeyCode::Char('q') => return Action::Close,
                KeyCode::Enter => self.confirm(),
                KeyCode::Char('e') => {
                    let cursor = self.line.chars().count();
                    self.stage = Stage::Editing { text: self.line.clone(), cursor };
                }
                KeyCode::Char('E') => return Action::EditPost(self.post()),
                KeyCode::Char('r') => {
                    self.status = None;
                    self.regenerating = self.note.is_none();
                    self.start_draft();
                }
                KeyCode::Down | KeyCode::Char('j') => self.scroll = (self.scroll + 1).min(self.max_scroll.get()),
                KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
                _ => {}
            },
            Stage::Editing { text, cursor } => match code {
                KeyCode::Enter => {
                    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if line != self.line {
                        // A new line rebuilds the post, so a whole-post edit is dropped.
                        if self.edited_post.take().is_some() {
                            self.status = Some("The post was rebuilt from the new line.".into());
                        }
                        self.line = line;
                    }
                    self.stage = Stage::Preview;
                }
                KeyCode::Esc => self.stage = Stage::Preview,
                KeyCode::Char(c) => insert(text, cursor, &c.to_string()),
                KeyCode::Backspace if *cursor > 0 => {
                    *cursor -= 1;
                    remove(text, *cursor);
                }
                KeyCode::Delete if *cursor < text.chars().count() => remove(text, *cursor),
                KeyCode::Left => *cursor = cursor.saturating_sub(1),
                KeyCode::Right => *cursor = (*cursor + 1).min(text.chars().count()),
                KeyCode::Home => *cursor = 0,
                KeyCode::End => *cursor = text.chars().count(),
                _ => {}
            },
            Stage::Saving => {}
            Stage::Done(_) | Stage::Failed(_) => return Action::Close,
        }
        Action::None
    }

    /// A bracketed paste: into the line being edited, as one line.
    pub fn on_paste(&mut self, pasted: &str) {
        if let Stage::Editing { text, cursor } = &mut self.stage {
            let flat: String = pasted.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
            insert(text, cursor, &flat);
        }
    }

    /// Runs the confirm steps on a worker; the clipboard copy comes back to `tick`.
    fn confirm(&mut self) {
        let Some(snapshot) = self.snapshot.clone() else { return };
        let text = self.post();
        let card = (self.deps.card)(&snapshot.date);
        let (copy_tx, copy) = mpsc::channel();
        let (copied, copied_rx) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let (desktop, run) = (self.deps.desktop, self.deps.run.clone());
        std::thread::spawn(move || {
            let copy = |t: &str| {
                let _ = copy_tx.send(t.to_string());
                copied_rx.recv().unwrap_or(Copied::Failed)
            };
            let res = share::confirm(&snapshot, &text, &card, &desktop, copy, &*run).map_err(|e| format!("{e:#}"));
            let _ = done_tx.send(res);
        });
        self.saving = Some(Saving { copy, copied, done });
        self.status = None;
        self.stage = Stage::Saving;
    }
}

fn byte_at(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i)
}

fn insert(text: &mut String, cursor: &mut usize, s: &str) {
    text.insert_str(byte_at(text, *cursor), s);
    *cursor += s.chars().count();
}

fn remove(text: &mut String, at: usize) {
    let i = byte_at(text, at);
    if i < text.len() {
        text.remove(i);
    }
}

/// How one confirm step went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Done,
    Skipped,
    Failed,
}

/// The Done screen: one line per confirm step.
pub fn done_lines(c: &Confirmed) -> Vec<(Mark, String)> {
    let mut out = vec![(Mark::Done, format!("Card saved to {}", c.card.display()))];
    out.push(match c.copied {
        Copied::Native(tool) => (Mark::Done, format!("Post copied to the clipboard ({tool})")),
        Copied::Terminal => (Mark::Done, "Post sent to the terminal's clipboard, if it allows that".into()),
        Copied::Failed => (Mark::Failed, "Couldn't reach a clipboard: copy the post below by hand".into()),
    });
    out.push(match &c.opened {
        Launch::Spawned => (Mark::Done, "Opened LinkedIn; if the post box is empty, paste the post".into()),
        Launch::Skipped(why) => (Mark::Skipped, format!("Browser not opened ({why}); open {}", c.url)),
        Launch::Failed(why) => (Mark::Failed, format!("Couldn't open the browser ({why}); open {}", c.url)),
    });
    let folder = c.card.parent().unwrap_or(Path::new(".")).display();
    out.push(match &c.revealed {
        Launch::Spawned => (Mark::Done, "Showed the card in the file manager: drag it into the post".into()),
        Launch::Skipped(why) => (Mark::Skipped, format!("File manager not opened ({why}); the card is in {folder}")),
        Launch::Failed(why) => (Mark::Failed, format!("Couldn't show the card ({why}); it is in {folder}")),
    });
    out
}

/// The editor for `E`: `$VISUAL`, then `$EDITOR`, then `notepad` or `vi`.
pub fn editor() -> String {
    ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.trim().is_empty())
        .unwrap_or_else(|| if cfg!(windows) { "notepad".into() } else { "vi".into() })
}

/// Edits `text` in `editor` (which may carry arguments, as in `code -w`) through a temp file.
pub fn edit_with(editor: &str, text: &str) -> anyhow::Result<String> {
    use anyhow::Context;
    let mut parts = editor.split_whitespace();
    let program = parts.next().context("no editor set")?;
    let path = std::env::temp_dir().join(format!("marsad-post-{}.txt", std::process::id()));
    std::fs::write(&path, text).context("couldn't write the post to a temp file")?;
    let status = std::process::Command::new(program).args(parts).arg(&path).status();
    let result = match status {
        Ok(s) if s.success() => std::fs::read_to_string(&path).context("couldn't read the edited post"),
        Ok(s) => Err(anyhow::anyhow!("{program} exited with {s}")),
        Err(e) => Err(anyhow::anyhow!("couldn't run {program}: {e}")),
    };
    let _ = std::fs::remove_file(&path);
    result
}

/// An Expert's stats for the tests.
#[cfg(test)]
fn test_snapshot() -> Snapshot {
    let mut stats = crate::badges::Stats::default();
    stats.0.insert(crate::badges::RANK_KEY, 386_200_000);
    Snapshot {
        date: "2026-10-24".into(),
        today: 12_000_000,
        week: 198_000_000,
        month: 340_000_000,
        daily_average: 11_300_000,
        all_time: 386_200_000,
        badges: crate::badges::badges(&stats),
        name: Some("Ada".into()),
        work: share::Work::default(),
        home: None,
    }
}

/// Deps that touch nothing real: an instant snapshot and line, the card at `card`, and desktop tools that fail.
#[cfg(test)]
pub(crate) fn fake_deps(card: PathBuf) -> Deps {
    Deps {
        snapshot: Arc::new(|| Ok(test_snapshot())),
        draft: Arc::new(|_, _| Draft { line: "Drafted.".into(), note: None }),
        run: Arc::new(|_| Err("not in tests".into())),
        desktop: Desktop::from_env(|_| false),
        card: Arc::new(move |_| card.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    fn snapshot() -> Snapshot {
        test_snapshot()
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marsad-share-view-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Fakes for every outside part: the line drafts at once (or waits for the cancel), the card
    /// goes to `card`, and every desktop tool is recorded instead of run.
    struct Fakes {
        deps: Deps,
        ran: Arc<Mutex<Vec<String>>>,
        drafts: Arc<Mutex<usize>>,
    }

    fn fakes(card: PathBuf, slow: bool) -> Fakes {
        let ran = Arc::new(Mutex::new(Vec::new()));
        let drafts = Arc::new(Mutex::new(0));
        let (r, d) = (ran.clone(), drafts.clone());
        let deps = Deps {
            snapshot: Arc::new(|| Ok(snapshot())),
            draft: Arc::new(move |s, cancel| {
                *d.lock().unwrap() += 1;
                if slow {
                    while !cancel.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    return Draft::template(s, Note::Cancelled);
                }
                Draft { line: format!("Drafted line {}.", d.lock().unwrap()), note: None }
            }),
            run: Arc::new(move |step| {
                r.lock().unwrap().push(step.program.to_string());
                Ok(())
            }),
            desktop: Desktop::from_env(|_| false),
            card: Arc::new(move |_| card.clone()),
        };
        Fakes { deps, ran, drafts }
    }

    /// Ticks until the view leaves `stage` (answering copy requests with `copy`), or panics after 10 s.
    fn settle(v: &mut ShareView, copy: Copied) {
        let start = Instant::now();
        loop {
            if let Some(_text) = v.tick() {
                v.copied(copy);
            }
            if !matches!(v.stage, Stage::Drafting | Stage::Saving) {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "stuck in {:?}", v.stage);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn preview(f: &Fakes) -> ShareView {
        let mut v = ShareView::new(f.deps.clone());
        settle(&mut v, Copied::Failed);
        assert_eq!(v.stage, Stage::Preview);
        v
    }

    #[test]
    fn drafting_ends_in_a_preview_of_the_line_and_the_stats() {
        let dir = scratch("preview");
        let f = fakes(dir.join("c.png"), false);
        let v = preview(&f);
        assert_eq!(v.line, "Drafted line 1.");
        assert!(v.post().starts_with("Drafted line 1.\n\nToday 12M tokens"), "{}", v.post());
        assert!(v.post().contains("Rank: Expert"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn covers_ae2_cancelling_writes_nothing_and_opens_nothing() {
        let dir = scratch("cancel");
        let card = dir.join("Downloads").join("marsad.png");
        // From Drafting: the running draft is told to stop.
        let f = fakes(card.clone(), true);
        let mut v = ShareView::new(f.deps.clone());
        while *f.drafts.lock().unwrap() == 0 {
            v.tick();
            std::thread::sleep(Duration::from_millis(5));
        }
        let cancel = v.cancel.clone();
        assert_eq!(v.on_key(KeyCode::Esc), Action::Close);
        drop(v);
        assert!(cancel.load(Ordering::Relaxed), "the child is killed");
        // From Preview.
        let f2 = fakes(card.clone(), false);
        let mut v = preview(&f2);
        assert_eq!(v.on_key(KeyCode::Char('q')), Action::Close);
        drop(v);
        std::thread::sleep(Duration::from_millis(50));
        assert!(!card.exists() && !dir.join("Downloads").exists(), "no file is written");
        assert!(f.ran.lock().unwrap().is_empty() && f2.ran.lock().unwrap().is_empty(), "nothing is opened");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn typing_q_while_editing_inserts_it() {
        let dir = scratch("edit-q");
        let f = fakes(dir.join("c.png"), false);
        let mut v = preview(&f);
        v.on_key(KeyCode::Char('e'));
        for code in [KeyCode::Home, KeyCode::Char('q'), KeyCode::Char(' '), KeyCode::Char('Q')] {
            assert_eq!(v.on_key(code), Action::None);
        }
        assert_eq!(v.stage, Stage::Editing { text: "q QDrafted line 1.".into(), cursor: 3 });
        v.on_key(KeyCode::Enter);
        assert_eq!((v.line.as_str(), &v.stage), ("q QDrafted line 1.", &Stage::Preview));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_pasted_newline_becomes_a_space_in_one_line() {
        let dir = scratch("paste");
        let f = fakes(dir.join("c.png"), false);
        let mut v = preview(&f);
        v.on_paste("ignored outside the editor\n");
        v.on_key(KeyCode::Char('e'));
        v.on_key(KeyCode::End);
        v.on_paste(" Then\nmore\r\nlines");
        v.on_key(KeyCode::Enter);
        assert_eq!(v.line, "Drafted line 1. Then more lines");
        assert_eq!(v.post().lines().next(), Some("Drafted line 1. Then more lines"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn esc_while_editing_restores_the_line_and_stays_in_the_preview() {
        let dir = scratch("edit-esc");
        let f = fakes(dir.join("c.png"), false);
        let mut v = preview(&f);
        v.on_key(KeyCode::Char('e'));
        v.on_key(KeyCode::Backspace);
        v.on_key(KeyCode::Char('!'));
        assert_eq!(v.on_key(KeyCode::Esc), Action::None);
        assert_eq!((v.line.as_str(), &v.stage), ("Drafted line 1.", &Stage::Preview));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn regenerate_drafts_again_and_a_failure_keeps_the_line() {
        let dir = scratch("regen");
        let f = fakes(dir.join("c.png"), false);
        let mut v = preview(&f);
        v.on_key(KeyCode::Char('r'));
        settle(&mut v, Copied::Failed);
        assert_eq!(v.line, "Drafted line 2.");
        // A regenerate that falls back to the template keeps the drafted line.
        v.deps.draft = Arc::new(|s, _| Draft::template(s, Note::Timeout));
        v.on_key(KeyCode::Char('r'));
        settle(&mut v, Copied::Failed);
        assert_eq!(v.line, "Drafted line 2.");
        assert!(v.status.as_deref().unwrap().contains("kept the line"), "{:?}", v.status);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_whole_post_can_be_edited_and_a_new_line_rebuilds_it() {
        let dir = scratch("edit-post");
        let f = fakes(dir.join("c.png"), false);
        let mut v = preview(&f);
        let Action::EditPost(text) = v.on_key(KeyCode::Char('E')) else { panic!("E opens the editor") };
        v.edited(Ok(format!("{text}\n#AI\n\n")));
        assert!(v.post().ends_with("Rank: Expert\n#AI"), "{}", v.post());
        v.edited(Ok("   \n".into()));
        assert!(v.post().ends_with("#AI"), "an empty edit is ignored");
        v.on_key(KeyCode::Char('e'));
        v.on_key(KeyCode::Char('!'));
        v.on_key(KeyCode::Enter);
        assert!(v.edited_post.is_none() && v.post().starts_with("Drafted line 1.!"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn confirm_saves_then_copies_on_the_main_thread_then_opens() {
        let dir = scratch("confirm");
        let card = dir.join("Downloads").join("marsad-2026-10-24.png");
        let mut f = fakes(card.clone(), false);
        f.deps.desktop = Desktop::from_env(|name| name == "DISPLAY");
        let mut v = preview(&f);
        let post = v.post();
        assert_eq!(v.on_key(KeyCode::Enter), Action::None);
        assert_eq!(v.stage, Stage::Saving);
        // The worker waits for the main thread's copy.
        let start = Instant::now();
        let asked = loop {
            if let Some(text) = v.tick() {
                break text;
            }
            assert!(start.elapsed() < Duration::from_secs(30), "no copy request");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(asked, post);
        assert!(card.exists(), "the card is written before the copy");
        assert!(f.ran.lock().unwrap().is_empty(), "nothing opens before the copy");
        v.copied(Copied::Native("pbcopy"));
        settle(&mut v, Copied::Failed);
        let Stage::Done(done) = &v.stage else { panic!("{:?}", v.stage) };
        assert_eq!(done.copied, Copied::Native("pbcopy"));
        assert_eq!(f.ran.lock().unwrap().len(), 2, "browser, then file manager");
        assert_eq!(v.on_key(KeyCode::Char('x')), Action::Close, "any key closes the Done screen");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_failed_card_write_keeps_the_preview_and_copies_and_opens_nothing() {
        let dir = scratch("unwritable");
        std::fs::write(dir.join("file"), "").unwrap();
        let f = fakes(dir.join("file").join("marsad.png"), false);
        let mut v = preview(&f);
        v.on_key(KeyCode::Enter);
        let start = Instant::now();
        while v.stage == Stage::Saving {
            assert!(v.tick().is_none(), "nothing is copied");
            assert!(start.elapsed() < Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(v.stage, Stage::Preview);
        assert!(v.status.as_deref().unwrap().starts_with("Couldn't save the card"), "{:?}", v.status);
        assert!(f.ran.lock().unwrap().is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_failed_snapshot_says_why_and_closes_on_any_key() {
        let dir = scratch("snapshot");
        let mut f = fakes(dir.join("c.png"), false);
        f.deps.snapshot = Arc::new(|| Err(anyhow::anyhow!("database is locked")));
        let mut v = ShareView::new(f.deps.clone());
        settle(&mut v, Copied::Failed);
        assert_eq!(v.stage, Stage::Failed("Couldn't gather today's stats: database is locked".into()));
        assert_eq!(v.on_key(KeyCode::Enter), Action::Close);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_done_screen_says_a_clipboard_failure_and_still_lists_the_card() {
        let done = Confirmed {
            card: PathBuf::from("/d/marsad-2026-10-24.png"),
            copied: Copied::Failed,
            url: "https://www.linkedin.com/feed/".into(),
            opened: Launch::Skipped("over SSH"),
            revealed: Launch::Failed("not installed".into()),
        };
        let lines = done_lines(&done);
        assert_eq!(lines[0], (Mark::Done, "Card saved to /d/marsad-2026-10-24.png".into()));
        assert_eq!(lines[1].0, Mark::Failed);
        assert!(lines[1].1.contains("Couldn't reach a clipboard"));
        assert_eq!(lines[2], (Mark::Skipped, "Browser not opened (over SSH); open https://www.linkedin.com/feed/".into()));
        assert_eq!(lines[3], (Mark::Failed, "Couldn't show the card (not installed); it is in /d".into()));
    }

    #[cfg(unix)]
    #[test]
    fn the_editor_round_trip_returns_the_edited_text() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("editor");
        let script = dir.join("ed");
        std::fs::write(&script, "#!/bin/sh\nprintf ' and more' >> \"$2\"\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let got = edit_with(&format!("{} --wait", script.display()), "my post").unwrap();
        assert_eq!(got, "my post and more");
        assert!(edit_with("marsad-no-such-editor", "x").is_err());
        assert!(edit_with("false", "x").is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
