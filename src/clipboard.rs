//! Copies text to the clipboard: a system tool when marsad runs locally, else OSC 52 through
//! the terminal. OSC 52 has no acknowledgement, so only a system tool counts as copied.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How the text left marsad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Copied {
    /// This system tool took the text.
    Native(&'static str),
    /// OSC 52 went to the terminal, which may or may not honour it.
    Terminal,
    /// Not even OSC 52 could be written.
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Platform {
    Macos,
    Linux,
    Windows,
    Other,
}

/// The parts of the environment that decide how to copy.
#[derive(Clone, Copy, Debug)]
struct Env {
    ssh: bool,
    tmux: bool,
    platform: Platform,
    wsl: bool,
}

/// A system tool: program and arguments. The text goes in on stdin.
type Tool = (&'static str, &'static [&'static str]);

/// The longest a system tool may take before it counts as failed.
const TOOL_WAIT: Duration = Duration::from_secs(1);

/// Copies `text` per the real environment and platform.
pub fn copy(text: &str) -> Copied {
    let env = Env::current();
    run(text, &tools(&env), TOOL_WAIT, env.tmux, &mut std::io::stdout())
}

impl Env {
    fn current() -> Self {
        let set = |name| std::env::var_os(name).is_some();
        let platform = if cfg!(target_os = "macos") {
            Platform::Macos
        } else if cfg!(target_os = "linux") {
            Platform::Linux
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Other
        };
        // WSL runs a Linux kernel whose release names Microsoft.
        let wsl = platform == Platform::Linux
            && std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .is_ok_and(|release| release.to_lowercase().contains("microsoft"));
        Env { ssh: set("SSH_CONNECTION") || set("SSH_TTY"), tmux: set("TMUX"), platform, wsl }
    }
}

/// The system tools to try, in order. None over SSH: they would fill the remote machine's clipboard.
fn tools(env: &Env) -> Vec<Tool> {
    const CLIP: Tool = ("clip.exe", &[]);
    const LINUX: [Tool; 3] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    if env.ssh {
        return Vec::new();
    }
    match env.platform {
        Platform::Macos => vec![("pbcopy", &[])],
        Platform::Linux if env.wsl => [CLIP].into_iter().chain(LINUX).collect(),
        Platform::Linux => LINUX.to_vec(),
        Platform::Windows => vec![CLIP],
        Platform::Other => Vec::new(),
    }
}

/// Tries each tool in turn; when none takes the text, writes OSC 52 to `out`.
fn run(text: &str, tools: &[Tool], wait: Duration, tmux: bool, out: &mut impl Write) -> Copied {
    if let Some(&(name, _)) = tools.iter().find(|&&(name, args)| run_tool(name, args, text, wait)) {
        return Copied::Native(name);
    }
    match out.write_all(&osc52(text, tmux)).and_then(|_| out.flush()) {
        Ok(()) => Copied::Terminal,
        Err(_) => Copied::Failed,
    }
}

/// Feeds `text` to one tool and waits at most `wait` for it to exit successfully. A missing
/// program, a failed write, a non-zero exit or an overrun all count as failure.
fn run_tool(name: &str, args: &[&str], text: &str, wait: Duration) -> bool {
    let child = Command::new(name).args(args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else { return false };
    // Dropping stdin closes it, so the tool sees the end of the text.
    let written = child.stdin.take().is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
    let deadline = Instant::now() + wait;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return written && status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// Standard base64 with padding (RFC 4648).
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                s.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// The OSC 52 sequence that sets the clipboard, wrapped for tmux to pass it through.
fn osc52(text: &str, tmux: bool) -> Vec<u8> {
    let seq = format!("\x1b]52;c;{}\x07", base64(text.as_bytes())).into_bytes();
    if !tmux {
        return seq;
    }
    // tmux forwards a DCS passthrough to the outer terminal once every inner ESC is doubled.
    let mut wrapped = b"\x1bPtmux;".to_vec();
    for &b in &seq {
        if b == 0x1b {
            wrapped.push(0x1b);
        }
        wrapped.push(b);
    }
    wrapped.extend_from_slice(b"\x1b\\");
    wrapped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(platform: Platform) -> Env {
        Env { ssh: false, tmux: false, platform, wsl: false }
    }

    fn names(tools: &[Tool]) -> Vec<&'static str> {
        tools.iter().map(|t| t.0).collect()
    }

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xfb, 0xff]), "+/8=", "the last two letters of the alphabet");
    }

    #[test]
    fn osc52_sets_the_clipboard() {
        assert_eq!(osc52("foo", false), b"\x1b]52;c;Zm9v\x07");
    }

    #[test]
    fn osc52_passes_through_tmux() {
        assert_eq!(osc52("foo", true), b"\x1bPtmux;\x1b\x1b]52;c;Zm9v\x07\x1b\\");
    }

    #[test]
    fn ssh_skips_system_tools() {
        for platform in [Platform::Macos, Platform::Linux, Platform::Windows] {
            assert!(tools(&Env { ssh: true, ..env(platform) }).is_empty(), "{platform:?}");
        }
        assert!(tools(&Env { ssh: true, wsl: true, ..env(Platform::Linux) }).is_empty());
    }

    #[test]
    fn each_platform_has_its_tools() {
        assert_eq!(names(&tools(&env(Platform::Macos))), ["pbcopy"]);
        assert_eq!(names(&tools(&env(Platform::Linux))), ["wl-copy", "xclip", "xsel"]);
        assert_eq!(tools(&env(Platform::Linux))[1].1, ["-selection", "clipboard"]);
        assert_eq!(tools(&env(Platform::Linux))[2].1, ["--clipboard", "--input"]);
        assert_eq!(names(&tools(&env(Platform::Windows))), ["clip.exe"]);
        assert_eq!(names(&tools(&Env { wsl: true, ..env(Platform::Linux) }))[0], "clip.exe");
        assert!(tools(&env(Platform::Other)).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_tool_falls_through_to_the_next() {
        let mut out = Vec::new();
        let tools: [Tool; 3] = [("marsad-no-such-tool", &[]), ("false", &[]), ("cat", &[])];
        assert_eq!(run("foo", &tools, TOOL_WAIT, false, &mut out), Copied::Native("cat"));
        assert!(out.is_empty(), "no OSC 52 after a tool took the text");
    }

    #[cfg(unix)]
    #[test]
    fn osc52_when_every_tool_fails() {
        let mut out = Vec::new();
        let tools: [Tool; 2] = [("marsad-no-such-tool", &[]), ("false", &[])];
        assert_eq!(run("foo", &tools, TOOL_WAIT, false, &mut out), Copied::Terminal);
        assert_eq!(out, b"\x1b]52;c;Zm9v\x07");
        out.clear();
        assert_eq!(run("foo", &[], TOOL_WAIT, true, &mut out), Copied::Terminal);
        assert_eq!(out, osc52("foo", true));
    }

    #[cfg(unix)]
    #[test]
    fn a_hung_tool_is_killed() {
        let mut out = Vec::new();
        let start = Instant::now();
        let tools: [Tool; 1] = [("sleep", &["5"])];
        assert_eq!(run("foo", &tools, Duration::from_millis(100), false, &mut out), Copied::Terminal);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn an_unwritable_terminal_fails() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert_eq!(run("foo", &[], TOOL_WAIT, false, &mut Broken), Copied::Failed);
    }
}
