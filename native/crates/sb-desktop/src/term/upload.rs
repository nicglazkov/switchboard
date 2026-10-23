//! Files go to the chat's machine (the web app's d37dcc3): the chat usually runs on another computer, so a dropped,
//! attached or pasted file is sent there first (POST /api/upload), with a chip that shows its progress or what went
//! wrong, and then its path there is pasted into the chat (a bracketed paste, as if typed).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use alacritty_terminal::term::TermMode;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, PathPromptOptions, SharedString,
    StatefulInteractiveElement, Styled, div, px, relative,
};
use sb_core::term::keys;

use super::{Phase, TermTab};
use crate::theme::Theme;
use crate::ui;

/// The server's cap (server.py UPLOAD_MAX), checked here first so a big file fails at once with a clear word.
pub const MAX: u64 = 20 * 1024 * 1024;

pub enum UpState {
    Sending,
    /// On the chat's machine at this path, and whether the path went into the chat (only when it was attached).
    Done(String, bool),
    Failed(String),
    /// Not a file: something about attaching itself (a title and a line under it), kept until it is closed.
    Note(String, String),
}

pub struct Upload {
    pub id: u64,
    pub name: String,
    pub total: u64,
    pub sent: Arc<AtomicU64>,
    pub state: UpState,
}

/// A path as a shell reads it: as is when it is plain, else in single quotes (double quotes on Windows).
fn quote(p: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "/._-+:@".contains(c) || (cfg!(windows) && c == '\\');
    if !p.is_empty() && p.chars().all(plain) {
        p.to_string()
    } else if cfg!(windows) {
        format!("\"{p}\"")
    } else {
        format!("'{}'", p.replace('\'', "'\\''"))
    }
}

fn mb(n: u64) -> String {
    if n >= 1024 * 1024 { format!("{:.1} MB", n as f64 / 1048576.) } else { format!("{} KB", (n / 1024).max(1)) }
}

/// A file to send, read whole, or why it can't be. Run off the UI thread: a file on a slow disk or a network share
/// can take a while.
fn read_file(p: &Path) -> Result<Vec<u8>, String> {
    if p.is_dir() {
        return Err("Folders can't be attached. Pick the files inside it.".into());
    }
    match std::fs::metadata(p) {
        Ok(m) if m.len() > MAX => Err(format!("That file is {}. Files up to 20 MB can be attached.", mb(m.len()))),
        Ok(_) => std::fs::read(p).map_err(|e| format!("Couldn't read it: {e}")),
        Err(e) => Err(format!("Couldn't read it: {e}")),
    }
}

/// The line under a sent file's name.
fn done_line(path: &str, pasted: bool) -> String {
    if pasted {
        format!("Pasted its path: {path}")
    } else {
        format!("Not pasted, the terminal wasn't attached. Its path: {path}")
    }
}

impl TermTab {
    /// Files dropped on the terminal or picked with Attach.
    pub fn send_files(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.local.is_some() {
            // A local shell has the files already: their paths are typed, quoted for the shell, as a drop does in
            // any terminal.
            let text = paths.iter().map(|p| quote(&p.display().to_string())).collect::<Vec<_>>().join(" ");
            if !text.is_empty() && self.phase == Phase::Open {
                let bracketed = self.term.mode().contains(TermMode::BRACKETED_PASTE);
                self.typed(keys::paste(&format!("{text} "), bracketed), cx);
            }
            return;
        }
        // Read one at a time in the background, so the files still go in the order they were dropped.
        cx.spawn(async move |this, cx| {
            for p in paths {
                let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "file".into());
                let read = cx.background_executor().spawn(async move { read_file(&p) }).await;
                let sent = this.update(cx, |t, cx| match read {
                    Ok(bytes) => t.send_bytes(name, bytes, cx),
                    Err(why) => t.up_fail(name, why, cx),
                });
                if sent.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn up_fail(&mut self, name: String, why: String, cx: &mut Context<Self>) {
        self.up_seq += 1;
        self.uploads.push(Upload {
            id: self.up_seq,
            name,
            total: 0,
            sent: Arc::new(AtomicU64::new(0)),
            state: UpState::Failed(why),
        });
        cx.notify();
    }

    /// Sends one file to the chat's machine, then pastes its path there into the chat.
    pub fn send_bytes(&mut self, name: String, bytes: Vec<u8>, cx: &mut Context<Self>) {
        if bytes.is_empty() {
            return self.up_fail(name, "That file is empty.".into(), cx);
        }
        self.up_seq += 1;
        let id = self.up_seq;
        let sent = Arc::new(AtomicU64::new(0));
        self.uploads.push(Upload {
            id,
            name: name.clone(),
            total: bytes.len() as u64,
            sent: sent.clone(),
            state: UpState::Sending,
        });
        let core = self.app.read(cx).core.clone();
        let machine = self.machine.clone();
        // Redraw while it goes, so the bar moves.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(150)).await;
                let going = this.update(cx, |t, cx| {
                    cx.notify();
                    t.uploads.iter().any(|u| u.id == id && matches!(u.state, UpState::Sending))
                });
                if !going.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let s2 = sent.clone();
            let res = cx.background_executor().spawn(async move { core.upload(&machine, &name, &bytes, &s2) }).await;
            let _ = this.update(cx, |t, cx| {
                let paste = match &res {
                    Ok(u) => Some(u.path.clone()),
                    Err(_) => None,
                };
                let pasted = paste.is_some() && t.phase == Phase::Open;
                if let Some(u) = t.uploads.iter_mut().find(|u| u.id == id) {
                    u.state = match res {
                        Ok(r) => UpState::Done(r.path, pasted),
                        Err(e) => UpState::Failed(e.to_string()),
                    };
                }
                if let Some(path) = paste.filter(|_| pasted) {
                    let bracketed = t.term.mode().contains(TermMode::BRACKETED_PASTE);
                    let shown = if path.contains(' ') { format!("\"{path}\" ") } else { format!("{path} ") };
                    t.typed(keys::paste(&shown, bracketed), cx);
                }
                // A pasted file's chip leaves after a few seconds; one not pasted keeps its path in view, and a
                // failed one stays, until it is closed.
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_secs(5)).await;
                    let _ = this.update(cx, |t, cx| {
                        t.uploads.retain(|u| !(u.id == id && matches!(u.state, UpState::Done(_, true))));
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// "Reply: <suggested reply>" from the details sheet: once the tab is attached, the reply goes in the same way a
    /// sent file's path does (a bracketed paste, as if typed), then Enter. A tab that was still connecting waits for
    /// Claude Code to draw its prompt first. Gives up after a minute. `done` always hears whether it went in, also
    /// when the tab closed meanwhile.
    pub fn reply_when_open(
        &mut self,
        text: String,
        done: impl FnOnce(bool, &mut gpui::App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let was_live = self.live();
        cx.spawn(async move |this, cx| {
            let went_in = async {
                if !was_live {
                    let mut live = false;
                    for _ in 0..120 {
                        match this.update(cx, |t, _| t.live()) {
                            Ok(true) => {
                                live = true;
                                break;
                            }
                            Ok(false) => cx.background_executor().timer(Duration::from_millis(500)).await,
                            Err(_) => return false,
                        }
                    }
                    if !live {
                        return false;
                    }
                    cx.background_executor().timer(Duration::from_millis(1200)).await;
                }
                let ok = this.update(cx, |t, cx| {
                    if t.phase != Phase::Open {
                        return false;
                    }
                    let bracketed = t.term.mode().contains(TermMode::BRACKETED_PASTE);
                    t.typed(keys::paste(&text, bracketed), cx);
                    true
                });
                if !matches!(ok, Ok(true)) {
                    return false;
                }
                cx.background_executor().timer(Duration::from_millis(160)).await;
                this.update(cx, |t, cx| t.typed(b"\r".to_vec(), cx)).is_ok()
            }
            .await;
            cx.update(|cx| done(went_in, cx));
        })
        .detach();
    }

    /// Attach: the system file picker, then the same path as a drop.
    pub fn attach(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn(async move |this, cx| match rx.await {
            Ok(Ok(Some(paths))) => {
                let _ = this.update(cx, |t, cx| t.send_files(paths, cx));
            }
            // Linux asks the desktop's file chooser portal, and a bare window manager may have none: say so, and
            // what still works, instead of doing nothing.
            Ok(Err(e)) if cfg!(not(any(target_os = "macos", target_os = "windows"))) => {
                log::info!("file picker: {e:#}");
                let _ = this.update(cx, |t, cx| {
                    t.up_seq += 1;
                    let note = UpState::Note("No file picker here".into(), "This desktop has no file chooser (xdg-desktop-portal). Drag the files onto the terminal instead.".into());
                    t.uploads.push(Upload { id: t.up_seq, name: String::new(), total: 0, sent: Arc::new(AtomicU64::new(0)), state: note });
                    cx.notify();
                });
            }
            _ => {}
        })
        .detach();
    }

    /// The chips over the bottom of the terminal: each file on its way, sent, or what went wrong.
    pub fn uploads_el(&self, t: &Theme, cx: &Context<Self>) -> Option<AnyElement> {
        if self.uploads.is_empty() {
            return None;
        }
        let label = if self.label.is_empty() { self.machine.clone() } else { self.label.clone() };
        let mut col = div().absolute().left(px(14.)).bottom(px(12.)).flex().flex_col().gap(px(6.)).max_w(px(460.));
        for u in &self.uploads {
            let id = u.id;
            let (title, sub, tone, frac) = match &u.state {
                UpState::Sending => {
                    let f = if u.total > 0 {
                        (u.sent.load(Ordering::Relaxed) as f32 / u.total as f32).clamp(0.02, 0.98)
                    } else {
                        0.5
                    };
                    (
                        format!("Sending {} to {label}", u.name),
                        format!("{} of {}", mb(u.sent.load(Ordering::Relaxed).min(u.total)), mb(u.total)),
                        t.run,
                        Some(f),
                    )
                }
                UpState::Done(p, pasted) => (format!("{} is on {label}", u.name), done_line(p, *pasted), t.warm, None),
                UpState::Failed(e) => (format!("{} wasn't sent", u.name), e.clone(), t.bad, None),
                UpState::Note(title, sub) => (title.clone(), sub.clone(), t.bad, None),
            };
            col = col.child(
                div()
                    .id(SharedString::from(format!("up-{id}")))
                    .relative()
                    .overflow_hidden()
                    .pl(px(12.))
                    .pr(px(6.))
                    .py(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(if matches!(u.state, UpState::Failed(_) | UpState::Note(..)) {
                        t.bad.alpha(0.5)
                    } else {
                        t.line2
                    })
                    .bg(t.panel2)
                    .shadow(t.drop())
                    .child(ui::icon(
                        match u.state {
                            UpState::Failed(_) | UpState::Note(..) => "warn",
                            UpState::Done(..) => "check",
                            UpState::Sending => "upload",
                        },
                        15.,
                        tone,
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                ui::wrap(title).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink),
                            )
                            .child(ui::wrap(sub).text_size(px(12.)).line_height(relative(1.35)).text_color(t.dim)),
                    )
                    .child(ui::icon_btn(t, SharedString::from(format!("up-x-{id}")), "x", 24.).on_click(cx.listener(
                        move |this, _: &gpui::ClickEvent, _, cx| {
                            this.uploads.retain(|u| u.id != id);
                            cx.notify();
                        },
                    )))
                    .children(frac.map(|f| div().absolute().left_0().bottom_0().h(px(2.)).w(relative(f)).bg(t.run))),
            );
        }
        Some(col.into_any_element())
    }

    /// While files are dragged over the terminal: where they will go.
    pub fn drop_hint(&self, t: &Theme) -> AnyElement {
        let label = if self.label.is_empty() { self.machine.clone() } else { self.label.clone() };
        let (head, sub) = if self.local.is_some() {
            ("Drop to type its path".to_string(), "The path goes in at the cursor.")
        } else {
            (format!("Drop to send to {label}"), "Its path there is pasted into the chat. Files up to 20 MB.")
        };
        div()
            .absolute()
            .inset(px(10.))
            .rounded(px(ui::R_SHEET))
            .border_2()
            .border_dashed()
            .border_color(t.accent)
            .bg(t.accent.alpha(0.08))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .invisible()
            .group_drag_over::<gpui::ExternalPaths>("term-drop", |s| s.visible())
            .child(ui::icon("upload", 24., t.accent))
            .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(head))
            .child(div().text_size(px(12.5)).text_color(t.dim).child(sub))
            .into_any_element()
    }
}

#[cfg(test)]
mod quote_tests {
    use super::{MAX, done_line, quote, read_file};

    #[test]
    fn plain_paths_stay_and_others_are_quoted() {
        assert_eq!(quote("/tmp/a-b_c.txt"), "/tmp/a-b_c.txt");
        if cfg!(windows) {
            assert_eq!(quote("C:\\My Files\\a.txt"), "\"C:\\My Files\\a.txt\"");
        } else {
            assert_eq!(quote("/tmp/my file.txt"), "'/tmp/my file.txt'");
            assert_eq!(quote("/tmp/it's"), "'/tmp/it'\\''s'");
        }
    }

    #[test]
    fn a_path_not_pasted_says_so() {
        assert_eq!(done_line("/tmp/a.png", true), "Pasted its path: /tmp/a.png");
        assert_eq!(done_line("/tmp/a.png", false), "Not pasted, the terminal wasn't attached. Its path: /tmp/a.png");
    }

    #[test]
    fn reading_a_file_to_send() {
        let dir = std::env::temp_dir().join(format!("sb-upload-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let small = dir.join("a.txt");
        std::fs::write(&small, b"hello").unwrap();
        assert_eq!(read_file(&small).unwrap(), b"hello");
        let big = dir.join("big.bin");
        std::fs::File::create(&big).unwrap().set_len(MAX + 1).unwrap();
        assert_eq!(read_file(&big).unwrap_err(), "That file is 20.0 MB. Files up to 20 MB can be attached.");
        assert!(read_file(&dir).unwrap_err().starts_with("Folders can't be attached"));
        assert!(read_file(&dir.join("missing")).unwrap_err().starts_with("Couldn't read it"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
