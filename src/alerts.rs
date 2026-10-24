//! Sounds and desktop notifications when a session starts waiting on the user or finishes
//! its work. Played by the dashboard only, in child processes it never waits on.

use crate::model::{IdleCause, Session, Status};
use std::collections::HashMap;
use std::process::{Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sound {
    /// A session asks a question or wants approval.
    Attention,
    /// A session finished its turn.
    Done,
}

/// What changed since the previous observation.
#[derive(Debug, Default, PartialEq)]
pub struct Alerts {
    /// (title, body) of a notification for each session that started waiting on the user.
    pub attention: Vec<(String, String)>,
    pub done: bool,
}

impl Alerts {
    /// One sound per kind, however many sessions changed, plus a notification per waiting session.
    pub fn fire(&self) {
        if !self.attention.is_empty() {
            play(Sound::Attention);
        }
        for (title, body) in &self.attention {
            notify(title, body);
        }
        if self.done {
            play(Sound::Done);
        }
    }
}

/// Compares each observation with the previous one; the first only records the starting state.
#[derive(Default)]
pub struct Detector {
    last: Option<HashMap<String, (Status, f64, IdleCause)>>,
}

impl Detector {
    pub fn observe<'a>(&mut self, sessions: impl IntoIterator<Item = &'a Session>) -> Alerts {
        let mut alerts = Alerts::default();
        let mut seen = HashMap::new();
        for s in sessions {
            let now = (s.status, s.status_since, s.idle_cause);
            // Status, the time it was entered and why it went idle: unchanged means nothing new
            // happened. An idle session's cause alone changes when a late Stop finishes it.
            if let Some(last) = &self.last
                && let prev = last.get(&s.id)
                && prev != Some(&now)
            {
                if s.status.needs_user() {
                    alerts.attention.push(notification(s));
                }
                if prev.is_some() && s.status == Status::Idle && s.idle_cause == IdleCause::Finished {
                    alerts.done = true;
                }
            }
            seen.insert(s.id.clone(), now);
        }
        self.last = Some(seen);
        alerts
    }
}

fn notification(s: &Session) -> (String, String) {
    let body = match (s.status, s.detail.is_empty()) {
        (Status::Approve, true) => "Waiting for your approval".to_string(),
        (Status::Approve, false) => format!("Approve: {}", s.detail),
        (_, true) => "Has a question for you".to_string(),
        (_, false) => s.detail.clone(),
    };
    (format!("marsad · {}", s.project()), body)
}

/// Starts `cmd` detached from the terminal and reaps it in the background.
fn spawn(cmd: &str, args: &[&str]) -> bool {
    let child = Command::new(cmd).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    match child {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
            true
        }
        Err(_) => false,
    }
}

/// The platform's own sound, or the terminal bell when there is none.
fn play(sound: Sound) {
    if !play_system(sound) {
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = out.write_all(b"\x07").and_then(|_| out.flush());
    }
}

#[cfg(target_os = "macos")]
fn play_system(sound: Sound) -> bool {
    let file = match sound {
        Sound::Attention => "/System/Library/Sounds/Ping.aiff",
        Sound::Done => "/System/Library/Sounds/Glass.aiff",
    };
    std::path::Path::new(file).exists() && spawn("afplay", &[file])
}

#[cfg(target_os = "linux")]
fn play_system(sound: Sound) -> bool {
    let file = match sound {
        Sound::Attention => "/usr/share/sounds/freedesktop/stereo/message-new-instant.oga",
        Sound::Done => "/usr/share/sounds/freedesktop/stereo/complete.oga",
    };
    std::path::Path::new(file).exists() && (spawn("paplay", &[file]) || spawn("canberra-gtk-play", &["-f", file]))
}

#[cfg(windows)]
fn play_system(sound: Sound) -> bool {
    let name = match sound {
        Sound::Attention => "Exclamation",
        Sound::Done => "Asterisk",
    };
    let script = format!("[System.Media.SystemSounds]::{name}.Play(); Start-Sleep -Milliseconds 800");
    spawn("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script])
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
fn play_system(_: Sound) -> bool {
    false
}

/// A desktop notification. Text goes in as arguments, never through a shell.
fn notify(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    spawn("osascript", &[
        "-e", "on run argv", "-e", "display notification (item 2 of argv) with title (item 1 of argv)", "-e", "end run",
        title, body,
    ]);
    #[cfg(target_os = "linux")]
    spawn("notify-send", &[title, body]);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = (title, body);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, status: Status, since: f64) -> Session {
        let mut s = Session::new(id, 0.0);
        s.cwd = format!("/p/{id}");
        s.set_status(status, since);
        s
    }

    #[test]
    fn existing_states_are_not_announced() {
        let mut d = Detector::default();
        let waiting = [session("a", Status::Approve, 1.0), session("b", Status::Question, 1.0), session("c", Status::Idle, 1.0)];
        assert_eq!(d.observe(&waiting), Alerts::default());
        assert_eq!(d.observe(&waiting), Alerts::default(), "nothing changed");
    }

    #[test]
    fn entering_a_waiting_state_is_announced_once() {
        let mut d = Detector::default();
        let mut a = session("a", Status::Active, 1.0);
        a.detail = "Bash: rm -rf target".into();
        let mut b = session("b", Status::Active, 1.0);
        d.observe([&a, &b]);
        a.set_status(Status::Approve, 2.0);
        b.set_status(Status::Question, 2.0);
        b.detail = "Which database?".into();
        let alerts = d.observe([&a, &b]);
        assert_eq!(alerts.attention, [
            ("marsad · a".to_string(), "Approve: Bash: rm -rf target".to_string()),
            ("marsad · b".to_string(), "Which database?".to_string()),
        ]);
        assert!(!alerts.done);
        assert!(d.observe([&a, &b]).attention.is_empty(), "announced once");
        // Back to work and waiting again between two observations still counts.
        a.set_status(Status::Active, 3.0);
        a.set_status(Status::Approve, 3.5);
        assert_eq!(d.observe([&a, &b]).attention.len(), 1);
    }

    #[test]
    fn only_finished_turns_sound_done() {
        let mut d = Detector::default();
        let mut s = session("a", Status::Active, 1.0);
        d.observe([&s]);
        s.set_idle(IdleCause::Interrupted, 2.0);
        assert!(!d.observe([&s]).done, "an interrupt is not a finish");
        s.set_status(Status::Active, 3.0);
        d.observe([&s]);
        s.set_idle(IdleCause::Finished, 4.0);
        assert!(d.observe([&s]).done);
        // A Stop that finishes a session already idled as interrupted still sounds done.
        s.set_status(Status::Active, 4.5);
        d.observe([&s]);
        s.set_idle(IdleCause::Interrupted, 5.0);
        d.observe([&s]);
        s.set_idle(IdleCause::Finished, 6.0);
        assert!(d.observe([&s]).done, "the cause changed even though the status did not");
        // A session first seen after startup, already idle, is not news.
        let fresh = session("b", Status::Idle, 5.0);
        assert!(!d.observe([&s, &fresh]).done);
    }
}
