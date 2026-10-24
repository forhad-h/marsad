//! The dashboard's share view: a state machine from drafting the line, through the preview and its
//! edits, to the confirm steps and a Done screen. The dashboard routes keys here before its own.

use crate::clipboard::Copied;
use crate::share::{self, Desktop, Draft, Launch, Note, Snapshot, Step};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::sync::Arc;

/// LinkedIn's limit on a post's length, in characters.
pub const MAX_POST: usize = 3000;

type DraftFn = dyn Fn(&Snapshot, &AtomicBool) -> Draft + Send + Sync;
type RunFn = dyn Fn(&Step) -> Result<(), String> + Send + Sync;

/// What the share view runs: the real thing in the dashboard, fakes in tests.
#[derive(Clone)]
pub struct Env {
    /// Syncs and freezes the stats.
    pub prepare: Arc<dyn Fn() -> Result<Snapshot, String> + Send + Sync>,
    pub draft: Arc<DraftFn>,
    pub card_path: Arc<dyn Fn(&Snapshot) -> PathBuf + Send + Sync>,
    /// Called on the main thread only: OSC 52 writes to the terminal and would interleave with drawing.
    pub copy: Arc<dyn Fn(&str) -> Copied + Send + Sync>,
    pub run: Arc<RunFn>,
    pub desktop: Desktop,
}

impl Env {
    /// The dashboard's: `rank_total` is the header's all-time figure.
    pub fn real(rank_total: i64) -> Self {
        Env {
            prepare: Arc::new(move || prepare(rank_total).map_err(|e| format!("{e:#}"))),
            draft: Arc::new(share::draft),
            card_path: Arc::new(|s| share::default_card_path(&s.date)),
            copy: Arc::new(crate::clipboard::copy),
            run: Arc::new(share::launch),
            desktop: Desktop::current(),
        }
    }
}

/// Syncs, then freezes the snapshot the post, the card and its file name share. git is asked for the
/// user's name meanwhile, since it may take up to two seconds.
fn prepare(rank_total: i64) -> anyhow::Result<Snapshot> {
    let name = std::thread::spawn(share::git_user_name);
    crate::store::open_and_sync()?;
    let conn = crate::store::open_readonly()?;
    let total = rank_total.max(crate::store::lifetime(&conn)?.total);
    share::snapshot(&conn, &crate::store::day_n_ago(0), total, dirs::home_dir(), name.join().ok().flatten())
}

/// What a key did to the view.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Stay,
    /// The share is over: cancelled, failed or done.
    Close,
    /// Suspend the dashboard and open this post text in the user's editor.
    EditPost(String),
}

/// One confirm step's result, for the Done screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    Done,
    Skipped,
    Failed,
}

/// What confirming did.
#[derive(Clone, Debug, PartialEq)]
pub struct Done {
    pub card: PathBuf,
    pub copied: Copied,
    pub url: String,
    pub opened: Launch,
    pub revealed: Launch,
    pub post: String,
}

impl Done {
    /// One row per step: its name, how it went and what to say about it.
    pub fn steps(&self) -> Vec<(&'static str, Mark, String)> {
        let launch = |l: &Launch, done: &str| match l {
            Launch::Spawned => (Mark::Done, done.to_string()),
            Launch::Skipped(why) => (Mark::Skipped, format!("skipped ({why})")),
            Launch::Failed(why) => (Mark::Failed, format!("failed: {why}")),
        };
        let (copied, copy_text) = match self.copied {
            Copied::Native(tool) => (Mark::Done, format!("the post is on your clipboard ({tool})")),
            Copied::Terminal => (Mark::Done, "sent to your terminal's clipboard, if it allows that".to_string()),
            Copied::Failed => (Mark::Failed, "couldn't reach a clipboard; copy the post below by hand".to_string()),
        };
        let (opened, open_text) = launch(&self.opened, "opened LinkedIn's post composer");
        let (revealed, reveal_text) = launch(&self.revealed, "showed the card in your file manager");
        vec![
            ("card", Mark::Done, self.card.display().to_string()),
            ("clipboard", copied, copy_text),
            ("browser", opened, open_text),
            ("reveal", revealed, reveal_text),
        ]
    }

    /// The link to open by hand, when the browser didn't open it.
    pub fn manual_url(&self) -> Option<&str> {
        (self.opened != Launch::Spawned).then_some(self.url.as_str())
    }
}

enum Phase {
    /// Syncing, freezing the snapshot and drafting the line.
    Drafting(Receiver<Result<(Snapshot, Draft), String>>),
    Preview,
    /// Editing the line in place; the line itself changes only on Enter.
    Editing { text: Vec<char>, cursor: usize },
    Regenerating(Receiver<Draft>),
    Saving(Receiver<Result<(), String>>),
    Opening { rx: Receiver<(String, Launch, Launch)>, copied: Copied },
    Done(Done),
    /// The sync or the snapshot failed, so there is nothing to share.
    Failed(String),
}

/// What the dashboard draws.
pub enum Screen<'a> {
    Busy(&'static str),
    Preview { post: String, editing: Option<(&'a [char], usize)>, note: Option<String>, warning: Option<&'a str> },
    Done(&'a Done),
    Failed(&'a str),
}

pub struct ShareView {
    env: Env,
    phase: Phase,
    snapshot: Option<Snapshot>,
    line: String,
    note: Option<Note>,
    /// The whole post after an `E` edit; editing or regenerating the line replaces it.
    custom: Option<String>,
    /// A failure to show above the keys: a regenerate that didn't work, a card that couldn't be written.
    warning: Option<String>,
    /// Set when the share is cancelled or closed, which kills a running `claude`.
    cancel: Arc<AtomicBool>,
    /// The badges of a snapshot just frozen after the share's own sync, until the dashboard takes them.
    prepared: Option<crate::badges::Badges>,
    pub scroll: u16,
    /// The furthest the preview can scroll, set while drawing.
    pub max_scroll: Cell<u16>,
}

impl Drop for ShareView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl ShareView {
    /// Starts drafting on a worker thread.
    pub fn start(env: Env) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (prepare, draft, flag) = (env.prepare.clone(), env.draft.clone(), cancel.clone());
        std::thread::spawn(move || {
            let res = prepare().map(|s| {
                let d = draft(&s, &flag);
                (s, d)
            });
            let _ = tx.send(res);
        });
        ShareView {
            env,
            phase: Phase::Drafting(rx),
            snapshot: None,
            line: String::new(),
            note: None,
            custom: None,
            warning: None,
            cancel,
            prepared: None,
            scroll: 0,
            max_scroll: Cell::new(0),
        }
    }

    /// The post as it would be shared now.
    pub fn post(&self) -> String {
        match (&self.custom, &self.snapshot) {
            (Some(text), _) => text.clone(),
            (None, Some(s)) => share::post(s, &self.line),
            (None, None) => String::new(),
        }
    }

    pub fn screen(&self) -> Screen<'_> {
        match &self.phase {
            Phase::Drafting(_) => Screen::Busy("Drafting your line with Claude Code"),
            Phase::Regenerating(_) => Screen::Busy("Drafting a new line with Claude Code"),
            Phase::Saving(_) => Screen::Busy("Saving the card"),
            Phase::Opening { .. } => Screen::Busy("Opening LinkedIn"),
            Phase::Done(done) => Screen::Done(done),
            Phase::Failed(why) => Screen::Failed(why),
            Phase::Preview | Phase::Editing { .. } => Screen::Preview {
                post: self.post(),
                editing: match &self.phase {
                    Phase::Editing { text, cursor } => Some((text.as_slice(), *cursor)),
                    _ => None,
                },
                note: self.note.as_ref().map(Note::message),
                warning: self.warning.as_deref(),
            },
        }
    }

    /// The keys the footer offers now.
    pub fn keys(&self) -> Vec<(&'static str, &'static str)> {
        match &self.phase {
            Phase::Preview => vec![("enter", "confirm"), ("e", "edit line"), ("E", "edit post"), ("r", "regenerate"), ("↑↓", "scroll"), ("q/esc", "cancel")],
            Phase::Editing { .. } => vec![("enter", "keep"), ("esc", "discard")],
            Phase::Drafting(_) => vec![("q/esc", "cancel")],
            Phase::Regenerating(_) => vec![("esc", "keep the current line"), ("q", "cancel")],
            Phase::Saving(_) | Phase::Opening { .. } => vec![],
            Phase::Done(_) | Phase::Failed(_) => vec![("enter", "close")],
        }
    }

    /// Collects finished work; call every frame on the main thread.
    pub fn tick(&mut self) {
        match &self.phase {
            Phase::Drafting(rx) => match rx.try_recv() {
                Ok(Ok((s, d))) => {
                    self.prepared = Some(s.badges.clone());
                    self.snapshot = Some(s);
                    (self.line, self.note) = (d.line, d.note);
                    self.phase = Phase::Preview;
                }
                Ok(Err(why)) => self.phase = Phase::Failed(why),
                Err(TryRecvError::Disconnected) => self.phase = Phase::Failed("drafting stopped unexpectedly".into()),
                Err(TryRecvError::Empty) => {}
            },
            Phase::Regenerating(rx) => match rx.try_recv() {
                Ok(d) => {
                    self.regenerated(d);
                    self.phase = Phase::Preview;
                }
                Err(TryRecvError::Disconnected) => {
                    self.warning = Some("Couldn't regenerate: drafting stopped unexpectedly.".into());
                    self.phase = Phase::Preview;
                }
                Err(TryRecvError::Empty) => {}
            },
            Phase::Saving(rx) => match rx.try_recv() {
                Ok(Ok(())) => {
                    let post = self.post();
                    let copied = (self.env.copy)(&post);
                    self.phase = Phase::Opening { rx: self.open(post), copied };
                }
                Ok(Err(why)) => {
                    self.warning = Some(format!("Couldn't save the card, so nothing was copied or opened: {why}"));
                    self.phase = Phase::Preview;
                }
                Err(TryRecvError::Disconnected) => {
                    self.warning = Some("Couldn't save the card: the save stopped unexpectedly.".into());
                    self.phase = Phase::Preview;
                }
                Err(TryRecvError::Empty) => {}
            },
            Phase::Opening { rx, copied } => match rx.try_recv() {
                Ok((url, opened, revealed)) => {
                    let card = self.card_path();
                    self.phase = Phase::Done(Done { card, copied: *copied, url, opened, revealed, post: self.post() });
                }
                Err(TryRecvError::Disconnected) => {
                    let (card, copied, post) = (self.card_path(), *copied, self.post());
                    let failed = || Launch::Failed("stopped unexpectedly".into());
                    self.phase = Phase::Done(Done { card, copied, url: share::linkedin_url(&post), opened: failed(), revealed: failed(), post });
                }
                Err(TryRecvError::Empty) => {}
            },
            Phase::Preview | Phase::Editing { .. } | Phase::Done(_) | Phase::Failed(_) => {}
        }
    }

    /// Takes a regenerated line, unless it fell back to the template while the current line is a real draft.
    fn regenerated(&mut self, d: Draft) {
        match (&d.note, &self.note) {
            (Some(note), None) => self.warning = Some(format!("Couldn't regenerate, so the current line stays. {}", note.message())),
            _ => {
                (self.line, self.note, self.custom, self.warning) = (d.line, d.note, None, None);
                self.scroll = 0;
            }
        }
    }

    fn card_path(&self) -> PathBuf {
        self.snapshot.as_ref().map(|s| (self.env.card_path)(s)).unwrap_or_default()
    }

    fn confirm(&mut self) {
        let Some(s) = self.snapshot.clone() else { return };
        let path = self.card_path();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(share::save_card(&s, &path).map_err(|e| format!("{e:#}")));
        });
        self.warning = None;
        self.phase = Phase::Saving(rx);
    }

    fn open(&self, post: String) -> Receiver<(String, Launch, Launch)> {
        let (tx, rx) = channel();
        let (run, desktop, card) = (self.env.run.clone(), self.env.desktop, self.card_path());
        std::thread::spawn(move || {
            let (url, opened) = share::open_linkedin(&desktop, &post, &*run);
            let revealed = share::reveal(&desktop, &card, &*run);
            let _ = tx.send((url, opened, revealed));
        });
        rx
    }

    fn regenerate(&mut self) {
        let Some(s) = self.snapshot.clone() else { return };
        // A fresh flag, so stopping one regeneration doesn't poison the next.
        self.cancel.store(true, Ordering::SeqCst);
        self.cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (draft, flag) = (self.env.draft.clone(), self.cancel.clone());
        std::thread::spawn(move || {
            let _ = tx.send(draft(&s, &flag));
        });
        self.warning = None;
        self.phase = Phase::Regenerating(rx);
    }

    /// The badges the share's sync produced, once: the dashboard announces them as after its own sync.
    pub fn take_prepared(&mut self) -> Option<crate::badges::Badges> {
        self.prepared.take()
    }

    /// Takes the post back from the user's editor.
    pub fn edited_post(&mut self, edited: Result<String, String>) {
        match edited {
            Ok(text) if text.trim().is_empty() => self.warning = Some("The edited post was empty, so it was not used.".into()),
            Ok(text) => {
                // The user's own post: no longer the template, so a failed regenerate keeps it.
                (self.custom, self.note, self.warning) = (Some(text.trim_end().to_string()), None, None);
            }
            Err(why) => self.warning = Some(format!("Couldn't edit the post: {why}")),
        }
    }

    /// Inserts pasted text while editing the line; a line has no line breaks, so they become spaces.
    pub fn on_paste(&mut self, pasted: &str) {
        if let Phase::Editing { text, cursor } = &mut self.phase {
            let flat = pasted.replace("\r\n", " ").replace(['\r', '\n'], " ");
            for c in flat.chars() {
                text.insert(*cursor, c);
                *cursor += 1;
            }
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) -> Outcome {
        let code = key.code;
        match &mut self.phase {
            Phase::Drafting(_) => {
                if matches!(code, KeyCode::Char('q') | KeyCode::Esc) {
                    return Outcome::Close;
                }
            }
            Phase::Regenerating(_) => match code {
                KeyCode::Esc => {
                    self.cancel.store(true, Ordering::SeqCst);
                    self.phase = Phase::Preview;
                }
                KeyCode::Char('q') => return Outcome::Close,
                _ => {}
            },
            Phase::Preview => match code {
                KeyCode::Char('q') | KeyCode::Esc => return Outcome::Close,
                KeyCode::Enter => self.confirm(),
                KeyCode::Char('e') => {
                    let text: Vec<char> = self.line.chars().collect();
                    self.phase = Phase::Editing { cursor: text.len(), text };
                }
                KeyCode::Char('E') => return Outcome::EditPost(self.post()),
                KeyCode::Char('r') => self.regenerate(),
                KeyCode::Down | KeyCode::Char('j') => self.scroll_by(1),
                KeyCode::Up | KeyCode::Char('k') => self.scroll_by(-1),
                KeyCode::PageDown | KeyCode::Char(' ') => self.scroll_by(10),
                KeyCode::PageUp => self.scroll_by(-10),
                _ => {}
            },
            Phase::Editing { text, cursor } => match code {
                KeyCode::Enter => {
                    let line: String = text.iter().collect::<String>().trim().to_string();
                    if !line.is_empty() {
                        // The user's own line: no longer the template, so a failed regenerate keeps it.
                        (self.line, self.custom, self.note) = (line, None, None);
                    }
                    self.phase = Phase::Preview;
                }
                KeyCode::Esc => self.phase = Phase::Preview,
                KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                    text.insert(*cursor, c);
                    *cursor += 1;
                }
                KeyCode::Backspace if *cursor > 0 => {
                    *cursor -= 1;
                    text.remove(*cursor);
                }
                KeyCode::Delete if *cursor < text.len() => {
                    text.remove(*cursor);
                }
                KeyCode::Left => *cursor = cursor.saturating_sub(1),
                KeyCode::Right => *cursor = (*cursor + 1).min(text.len()),
                KeyCode::Home => *cursor = 0,
                KeyCode::End => *cursor = text.len(),
                _ => {}
            },
            Phase::Saving(_) | Phase::Opening { .. } => {}
            Phase::Done(_) | Phase::Failed(_) => {
                if matches!(code, KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q')) {
                    return Outcome::Close;
                }
            }
        }
        Outcome::Stay
    }

    fn scroll_by(&mut self, delta: i32) {
        self.scroll = (self.scroll as i32 + delta).clamp(0, self.max_scroll.get() as i32) as u16;
    }
}

/// Opens `text` in `$VISUAL`, `$EDITOR`, or `vi` (`notepad` on Windows), and returns what was saved.
pub fn edit_in_editor(text: &str) -> Result<String, String> {
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|e| !e.trim().is_empty())
        .unwrap_or_else(|| if cfg!(windows) { "notepad".into() } else { "vi".into() });
    let mut parts = editor.split_whitespace();
    let program = parts.next().ok_or("no editor is set")?;
    // A fresh file that must not exist yet, so a planted file or symlink in the shared temp dir is never written through.
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let path = std::env::temp_dir().join(format!("marsad-post-{}-{nanos:x}.txt", std::process::id()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
        .map_err(|e| format!("couldn't write {}: {e}", path.display()))?;
    let status = std::process::Command::new(program).args(parts).arg(&path).status();
    let result = match status {
        Ok(s) if s.success() => std::fs::read_to_string(&path).map_err(|e| format!("couldn't read {}: {e}", path.display())),
        Ok(s) => Err(format!("{program} exited with {s}")),
        Err(e) => Err(format!("couldn't run {program}: {e}")),
    };
    let _ = std::fs::remove_file(&path);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("marsad-share-view-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn snapshot() -> Snapshot {
        let stats = crate::badges::Stats([("rank", 300_000_000)].into_iter().collect());
        Snapshot {
            date: "2026-10-24".into(),
            today: 12_000_000,
            week: 80_000_000,
            month: 200_000_000,
            daily_average: 6_600_000,
            all_time: 300_000_000,
            badges: crate::badges::badges(&stats),
            name: Some("Test User".into()),
            work: share::Work::default(),
            home: None,
        }
    }

    /// Records every copy and launch; the card goes to `dir`; drafting answers `line` (after `wait`).
    struct Fake {
        copies: Arc<Mutex<Vec<String>>>,
        runs: Arc<Mutex<Vec<Step>>>,
    }

    fn env(dir: &Path, line: &'static str, wait: Duration, copied: Copied) -> (Env, Fake) {
        let (copies, runs) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
        let (c, r, dir) = (copies.clone(), runs.clone(), dir.to_path_buf());
        let env = Env {
            prepare: Arc::new(|| Ok(snapshot())),
            draft: Arc::new(move |_, cancel| {
                let until = Instant::now() + wait;
                while Instant::now() < until && !cancel.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Draft { line: line.into(), note: None }
            }),
            card_path: Arc::new(move |s| dir.join(format!("marsad-{}.png", s.date))),
            copy: Arc::new(move |text| {
                c.lock().unwrap().push(text.to_string());
                copied
            }),
            run: Arc::new(move |step| {
                r.lock().unwrap().push(step.clone());
                Ok(())
            }),
            desktop: Desktop::for_tests(),
        };
        (env, Fake { copies, runs })
    }

    /// Ticks until the view leaves its busy phase.
    fn settle(v: &mut ShareView) {
        let until = Instant::now() + Duration::from_secs(30);
        while matches!(v.screen(), Screen::Busy(_)) && Instant::now() < until {
            v.tick();
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn preview(dir: &Path) -> (ShareView, Fake) {
        let (env, fake) = env(dir, "I shipped a parser with two agents.", Duration::ZERO, Copied::Native("pbcopy"));
        let mut v = ShareView::start(env);
        settle(&mut v);
        assert!(matches!(v.screen(), Screen::Preview { .. }));
        (v, fake)
    }

    fn files(dir: &Path) -> usize {
        std::fs::read_dir(dir).unwrap().count()
    }

    #[test]
    fn cancelling_while_drafting_or_in_the_preview_writes_and_opens_nothing() {
        // Covers AE2.
        let dir = scratch("cancel");
        let (env, fake) = env(&dir, "never shown", Duration::from_secs(20), Copied::Native("pbcopy"));
        let mut v = ShareView::start(env);
        assert!(matches!(v.screen(), Screen::Busy(_)));
        assert_eq!(v.on_key(key(KeyCode::Esc)), Outcome::Close);
        drop(v);

        let (mut v, preview_fake) = preview(&dir);
        assert_eq!(v.on_key(key(KeyCode::Char('q'))), Outcome::Close, "q cancels the share, not marsad");
        drop(v);
        assert_eq!(files(&dir), 0);
        for f in [fake, preview_fake] {
            assert!(f.copies.lock().unwrap().is_empty() && f.runs.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn closing_while_drafting_kills_the_draft() {
        let dir = scratch("kill");
        let flag = Arc::new(Mutex::new(None::<bool>));
        let seen = flag.clone();
        let (mut env, _) = env(&dir, "", Duration::ZERO, Copied::Failed);
        env.draft = Arc::new(move |_, cancel| {
            let until = Instant::now() + Duration::from_secs(10);
            while Instant::now() < until && !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            *seen.lock().unwrap() = Some(cancel.load(Ordering::SeqCst));
            Draft { line: String::new(), note: None }
        });
        let v = ShareView::start(env);
        std::thread::sleep(Duration::from_millis(50));
        drop(v);
        let until = Instant::now() + Duration::from_secs(5);
        while flag.lock().unwrap().is_none() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*flag.lock().unwrap(), Some(true), "the draft saw the cancel");
    }

    #[test]
    fn editing_the_line_takes_every_letter_and_esc_restores_it() {
        let dir = scratch("edit");
        let (mut v, _) = preview(&dir);
        let before = v.post();
        v.on_key(key(KeyCode::Char('e')));
        for c in " q".chars() {
            assert_eq!(v.on_key(key(KeyCode::Char(c))), Outcome::Stay, "typing {c:?} while editing never closes");
        }
        assert!(matches!(v.screen(), Screen::Preview { editing: Some((t, _)), .. } if t.ends_with(&[' ', 'q'])));
        assert_eq!(v.on_key(key(KeyCode::Esc)), Outcome::Stay, "esc leaves the edit, not the share");
        assert_eq!(v.post(), before, "esc restores the previous line");

        v.on_key(key(KeyCode::Char('e')));
        v.on_key(key(KeyCode::Home));
        v.on_key(key(KeyCode::Char('>')));
        v.on_key(key(KeyCode::Enter));
        assert!(v.post().starts_with(">I shipped"), "{}", v.post());
    }

    #[test]
    fn a_pasted_newline_becomes_a_space_in_the_line() {
        let dir = scratch("paste");
        let (mut v, _) = preview(&dir);
        v.on_paste("ignored outside editing\n");
        v.on_key(key(KeyCode::Char('e')));
        v.on_key(key(KeyCode::End));
        v.on_paste(" And\r\nmore\nlines.");
        v.on_key(key(KeyCode::Enter));
        let post = v.post();
        let first = post.lines().next().unwrap();
        assert_eq!(first, "I shipped a parser with two agents. And more lines.");
        assert!(!post.contains("ignored"));
    }

    #[test]
    fn confirm_saves_copies_on_the_main_thread_opens_and_reveals() {
        let dir = scratch("confirm");
        let (mut v, fake) = preview(&dir);
        v.on_key(key(KeyCode::Enter));
        settle(&mut v);
        let Screen::Done(done) = v.screen() else { panic!("not done") };
        assert!(done.card.exists() && done.card.starts_with(&dir));
        assert_eq!(fake.copies.lock().unwrap().as_slice(), std::slice::from_ref(&done.post));
        let runs = fake.runs.lock().unwrap();
        assert!(runs[0].args.iter().any(|a| a.contains("linkedin.com")), "opened LinkedIn first: {runs:?}");
        assert!(runs.len() >= 2, "then revealed the card");
        assert_eq!(done.steps().iter().map(|s| s.1).collect::<Vec<_>>(), [Mark::Done; 4]);
        assert_eq!(done.manual_url(), None);
        drop(runs);
        assert_eq!(v.on_key(key(KeyCode::Enter)), Outcome::Close);
    }

    #[test]
    fn a_card_that_cant_be_written_keeps_the_preview_and_copies_nothing() {
        let dir = scratch("unwritable");
        let blocker = dir.join("not-a-folder");
        std::fs::write(&blocker, "").unwrap();
        let (mut env, fake) = env(&dir, "line", Duration::ZERO, Copied::Native("pbcopy"));
        env.card_path = Arc::new(move |_| blocker.join("card.png"));
        let mut v = ShareView::start(env);
        settle(&mut v);
        v.on_key(key(KeyCode::Enter));
        settle(&mut v);
        let Screen::Preview { warning: Some(w), .. } = v.screen() else { panic!("left the preview") };
        assert!(w.contains("Couldn't save the card"), "{w}");
        assert!(fake.copies.lock().unwrap().is_empty() && fake.runs.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failed_clipboard_is_reported_with_the_card_path() {
        let dir = scratch("noclip");
        let (env, _) = env(&dir, "line", Duration::ZERO, Copied::Failed);
        let mut v = ShareView::start(env);
        settle(&mut v);
        v.on_key(key(KeyCode::Enter));
        settle(&mut v);
        let Screen::Done(done) = v.screen() else { panic!("not done") };
        let steps = done.steps();
        assert_eq!(steps[0].0, "card");
        assert!(steps[0].2.contains("marsad-2026-10-24.png"));
        assert_eq!((steps[1].0, steps[1].1), ("clipboard", Mark::Failed));
        assert!(steps[1].2.contains("by hand"));
    }

    #[test]
    fn a_regenerate_that_falls_back_keeps_the_drafted_line() {
        let dir = scratch("regen");
        let (mut v, _) = preview(&dir);
        v.env.draft = Arc::new(|s, _| Draft::template(s, Note::Timeout));
        v.on_key(key(KeyCode::Char('r')));
        settle(&mut v);
        let Screen::Preview { post, warning: Some(w), .. } = v.screen() else { panic!("no warning") };
        assert!(post.starts_with("I shipped"), "{post}");
        assert!(w.contains("Couldn't regenerate"));
    }

    #[test]
    fn an_edited_post_replaces_the_text_until_the_line_changes() {
        let dir = scratch("post");
        let (mut v, _) = preview(&dir);
        assert_eq!(v.on_key(key(KeyCode::Char('E'))), Outcome::EditPost(v.post()));
        v.edited_post(Ok("My own words.\n".into()));
        assert_eq!(v.post(), "My own words.");
        v.edited_post(Ok("  \n".into()));
        assert_eq!(v.post(), "My own words.", "an empty edit is not used");
        v.on_key(key(KeyCode::Char('e')));
        v.on_key(key(KeyCode::Enter));
        assert!(v.post().starts_with("I shipped"), "editing the line rebuilds the post");
    }

    #[test]
    fn a_failed_regenerate_keeps_a_hand_edited_template_line_and_post() {
        let dir = scratch("regen-edited");
        let (mut v, _) = preview(&dir);
        v.env.draft = Arc::new(|s, _| Draft::template(s, Note::NotInstalled));
        // The line on screen is the template, then the user writes their own.
        (v.line, v.note) = ("Template.".into(), Some(Note::NotInstalled));
        v.on_key(key(KeyCode::Char('e')));
        v.on_paste(" Mine.");
        v.on_key(key(KeyCode::Enter));
        v.on_key(key(KeyCode::Char('r')));
        settle(&mut v);
        assert!(v.post().starts_with("Template. Mine."), "{}", v.post());
        assert!(matches!(v.screen(), Screen::Preview { warning: Some(_), .. }));
        v.edited_post(Ok("My own words.".into()));
        v.on_key(key(KeyCode::Char('r')));
        settle(&mut v);
        assert_eq!(v.post(), "My own words.", "the edited post survives too");
    }
}
