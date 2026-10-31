//! Optional desktop utilities, invoked without a shell or transcript-bearing arguments.
use super::Target;
use anyhow::{Context, Result, ensure};
use macafon_ipc::{DictationInsert, DictationOutcome, DictationPrefs};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub fn wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
}

fn available(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| {
            std::fs::metadata(dir.join(name))
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Injector {
    Wtype,
    Xdotool,
    Ydotool,
}
impl Injector {
    fn select(wayland: bool, has: impl Fn(&str) -> bool) -> Option<Self> {
        if wayland && has("wtype") {
            Some(Self::Wtype)
        } else if !wayland && has("xdotool") {
            Some(Self::Xdotool)
        } else if has("ydotool") {
            Some(Self::Ydotool)
        } else {
            None
        }
    }
    fn command(self, paste: bool) -> (&'static str, &'static [&'static str]) {
        match (self, paste) {
            (Self::Wtype, false) => ("wtype", &["-d", "0", "-"]),
            (Self::Wtype, true) => ("wtype", &["-M", "ctrl", "-P", "v", "-p", "v", "-m", "ctrl"]),
            (Self::Xdotool, false) => (
                "xdotool",
                &["type", "--clearmodifiers", "--delay", "0", "--file", "-"],
            ),
            (Self::Xdotool, true) => ("xdotool", &["key", "--clearmodifiers", "ctrl+v"]),
            (Self::Ydotool, false) => ("ydotool", &["type", "--key-delay", "0", "--file", "-"]),
            (Self::Ydotool, true) => ("ydotool", &["key", "29:1", "47:1", "47:0", "29:0"]),
        }
    }
    fn inject(self, text: &str, paste: bool) -> bool {
        let (program, args) = self.command(paste);
        run(program, args, (!paste).then_some(text.as_bytes()), false).is_ok()
    }
}

/// Clipboard owners fork into the background, so writes use /dev/null for output. Reads
/// and writes are bounded: a compositor refusing clipboard access must not hang the daemon.
fn run(program: &str, args: &[&str], input: Option<&[u8]>, capture: bool) -> Result<Vec<u8>> {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new(program)
        .args(args)
        .process_group(0)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::null())
        .spawn()
        .context("desktop tool unavailable")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    if let Some(mut stdout) = child.stdout.take() {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .by_ref()
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .and_then(|_| {
                    if bytes.len() > 8 * 1024 * 1024 {
                        Err(std::io::Error::other("clipboard too large"))
                    } else {
                        Ok(bytes)
                    }
                });
            let _ = sender.send(result);
        });
    }
    let writer = child.stdin.take().map(|mut stdin| {
        let bytes = input.unwrap_or_default().to_vec();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(stdin.write_all(&bytes));
        });
        receiver
    });
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "desktop tool failed");
            if let Some(writer) = writer {
                writer
                    .recv_timeout(Duration::from_millis(100))
                    .context("desktop tool input timed out")??;
            }
            return if capture {
                Ok(receiver
                    .recv_timeout(Duration::from_millis(100))
                    .context("desktop tool output timed out")??)
            } else {
                Ok(Vec::new())
            };
        }
        if started.elapsed() >= Duration::from_secs(2) {
            // SAFETY: the child has its own process group; kill only that utility and its helpers.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
            anyhow::bail!("desktop tool timed out");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn active_window() -> Option<usize> {
    if wayland() {
        return None;
    }
    if available("xprop")
        && let Ok(bytes) = run("xprop", &["-root", "_NET_ACTIVE_WINDOW"], None, true)
        && let Some(window) = parse_window(&String::from_utf8_lossy(&bytes))
    {
        return Some(window);
    }
    let bytes = run("xdotool", &["getactivewindow"], None, true).ok()?;
    String::from_utf8_lossy(&bytes)
        .trim()
        .parse()
        .ok()
        .filter(|id| *id != 0)
}
fn parse_window(value: &str) -> Option<usize> {
    let hex = value
        .split_whitespace()
        .find(|word| word.starts_with("0x"))?;
    usize::from_str_radix(hex.trim_end_matches(',').trim_start_matches("0x"), 16)
        .ok()
        .filter(|id| *id != 0)
}
impl Target {
    pub fn capture() -> Self {
        Self {
            window: active_window().unwrap_or(0),
            layout: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Clipboard {
    Wayland,
    Xclip,
    Xsel,
}
impl Clipboard {
    fn select(wayland: bool, has: impl Fn(&str) -> bool) -> Option<Self> {
        if wayland {
            has("wl-copy").then_some(Self::Wayland)
        } else if has("xclip") {
            Some(Self::Xclip)
        } else {
            has("xsel").then_some(Self::Xsel)
        }
    }
    fn read(self) -> Result<Vec<u8>> {
        let (program, args): (_, &[&str]) = match self {
            Self::Wayland => ("wl-paste", &["--no-newline", "--type", "text"]),
            Self::Xclip => (
                "xclip",
                &["-selection", "clipboard", "-out", "-target", "UTF8_STRING"],
            ),
            Self::Xsel => ("xsel", &["--clipboard", "--output"]),
        };
        run(program, args, None, true)
    }
    fn put(self, text: &[u8]) -> Result<()> {
        let (program, args): (_, &[&str]) = match self {
            Self::Wayland => ("wl-copy", &["--type", "text/plain;charset=utf-8"]),
            Self::Xclip => (
                "xclip",
                &[
                    "-selection",
                    "clipboard",
                    "-in",
                    "-target",
                    "UTF8_STRING",
                    "-silent",
                ],
            ),
            Self::Xsel => ("xsel", &["--clipboard", "--input"]),
        };
        run(program, args, Some(text), false).map(|_| ())
    }
}
fn clipboard() -> Result<Clipboard> {
    Clipboard::select(wayland(), available)
        .context("install wl-clipboard (Wayland) or xclip/xsel (X11) to copy dictation")
}
pub fn copy_text(text: &str) -> Result<()> {
    clipboard()?.put(text.as_bytes())
}

fn must_copy(
    wayland: bool,
    target: usize,
    current: Option<usize>,
    copy: bool,
    injector: Option<Injector>,
) -> bool {
    copy || injector.is_none() || (!wayland && (target == 0 || current != Some(target)))
}
fn should_restore(previous: Option<&[u8]>, inserted: &[u8], current: Option<&[u8]>) -> bool {
    previous.is_some() && current == Some(inserted)
}

pub fn insert(
    target: &Target,
    text: &str,
    prefs: &DictationPrefs,
    copy: bool,
    _game: bool,
) -> Result<DictationOutcome> {
    let text = format!(
        "{}{}",
        text.trim(),
        if prefs.trailing_space { " " } else { "" }
    );
    let wayland = wayland();
    let injector = Injector::select(wayland, available);
    if must_copy(wayland, target.window, active_window(), copy, injector) {
        copy_text(&text)?;
        return Ok(DictationOutcome::Copied);
    }
    let injector = injector.context("no insertion tool")?;
    if prefs.insert == DictationInsert::Type {
        // ydotool type is layout-dependent and cannot reliably type Unicode. Use it for
        // Ctrl+V instead; wtype and xdotool support Unicode via their virtual keyboards.
        if injector != Injector::Ydotool && injector.inject(&text, false) {
            return Ok(DictationOutcome::Inserted);
        }
        copy_text(&text)?;
        return Ok(DictationOutcome::Copied);
    }
    // Complete an older transaction before taking another snapshot (including identical text).
    super::finish_restores();
    let clipboard = clipboard()?;
    let previous = clipboard.read().ok();
    clipboard.put(text.as_bytes())?;
    // A failed snapshot must not turn into an empty restore or erase an unreadable image.
    if previous.is_none()
        || must_copy(
            wayland,
            target.window,
            active_window(),
            copy,
            Some(injector),
        )
        || !(injector.inject("", true)
            || (injector != Injector::Ydotool
                && available("ydotool")
                && Injector::Ydotool.inject("", true)))
    {
        return Ok(DictationOutcome::Copied);
    }
    super::track_restore(
        std::thread::Builder::new()
            .name("dictation-clipboard-restore".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                let current = clipboard.read().ok();
                if should_restore(previous.as_deref(), text.as_bytes(), current.as_deref())
                    && clipboard
                        .put(previous.as_deref().unwrap_or_default())
                        .is_err()
                {
                    tracing::warn!("Cannot restore dictation clipboard");
                }
            })?,
    );
    Ok(DictationOutcome::Inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dictation_tools_follow_the_session_and_keep_transcripts_off_arguments() {
        let has = |_: &str| true;
        assert_eq!(Injector::select(true, has), Some(Injector::Wtype));
        assert_eq!(Injector::select(false, has), Some(Injector::Xdotool));
        assert_eq!(Injector::select(true, |tool| tool == "xdotool"), None);
        assert_eq!(
            Injector::select(true, |tool| tool == "ydotool"),
            Some(Injector::Ydotool)
        );
        assert_eq!(Clipboard::select(true, has), Some(Clipboard::Wayland));
        assert_eq!(
            Clipboard::select(false, |tool| tool == "xsel"),
            Some(Clipboard::Xsel)
        );
        assert_eq!(
            Injector::Xdotool.command(false).1,
            ["type", "--clearmodifiers", "--delay", "0", "--file", "-"]
        );
        assert_eq!(
            Injector::Ydotool.command(true).1,
            ["key", "29:1", "47:1", "47:0", "29:0"]
        );
        assert_eq!(
            parse_window("_NET_ACTIVE_WINDOW(WINDOW): window id # 0x12ab"),
            Some(0x12ab)
        );
        assert_eq!(parse_window("_NET_ACTIVE_WINDOW: not found."), None);
        assert!(must_copy(
            false,
            12,
            Some(13),
            false,
            Some(Injector::Xdotool)
        ));
        assert!(must_copy(false, 0, None, false, Some(Injector::Xdotool)));
        assert!(!must_copy(true, 0, None, false, Some(Injector::Wtype)));
        assert!(must_copy(true, 0, None, false, None));
    }
    #[test]
    fn dictation_clipboard_restores_only_a_saved_snapshot_while_our_text_is_current() {
        assert!(should_restore(
            Some(b"before"),
            b"dictation",
            Some(b"dictation")
        ));
        assert!(should_restore(Some(b""), b"dictation", Some(b"dictation")));
        assert!(!should_restore(None, b"dictation", Some(b"dictation")));
        assert!(!should_restore(
            Some(b"before"),
            b"dictation",
            Some(b"new user copy")
        ));
        assert!(!should_restore(Some(b"before"), b"dictation", None));
    }
}
