//! A local terminal: a shell on this computer in a PTY of its own, spoken to through the same channels as the hub
//! (`Conn`), so the tab draws it with everything a chat's terminal has. Nothing goes through the server, and it is
//! never on the board.
//!
//! Three threads: one reads the output, one takes the tab's input and resizes, one waits for the shell to end. A
//! close from the tab (or the tab going away) kills the shell.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use sb_core::term::shells::{self, Profile};
use sb_core::term::ws::{Closed, Conn, FromHub, ToHub};

/// What the tab keeps to ask whether a program other than the shell is running.
pub struct Pty {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    pid: Option<u32>,
}

impl Pty {
    /// The program a close would cut off, or None while the shell itself is in front.
    pub fn busy(&self) -> Option<String> {
        #[cfg(unix)]
        {
            let lead = self.master.lock().ok()?.process_group_leader()?;
            if lead <= 0 || Some(lead as u32) == self.pid {
                return None;
            }
            Some(name_of(lead as u32).unwrap_or_else(|| "a program".into()))
        }
        #[cfg(not(unix))]
        {
            // ConPTY does not say which program is in front, so Windows never asks.
            let _ = (&self.master, self.pid);
            None
        }
    }
}

#[cfg(target_os = "linux")]
fn name_of(pid: u32) -> Option<String> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(all(unix, not(target_os = "linux")))]
fn name_of(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps").args(["-o", "comm=", "-p", &pid.to_string()]).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let base = s.rsplit('/').next().unwrap_or(&s).trim_start_matches('-').to_string();
    Some(base).filter(|s| !s.is_empty())
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows: rows.max(1), cols: cols.max(1), pixel_width: 0, pixel_height: 0 }
}

/// The home folder, where every local terminal starts.
fn home() -> Option<String> {
    let k = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var(k).ok().filter(|h| !h.is_empty() && std::path::Path::new(h).is_dir())
}

/// Starts the profile's shell at this size. The events come as the hub's would: "open" at once, the output, then
/// the exit code and a normal close.
pub fn spawn(p: &Profile, cols: u16, rows: u16) -> std::io::Result<(Conn, Pty)> {
    let pair = native_pty_system().openpty(size(cols, rows)).map_err(io_err)?;
    let mut cmd = CommandBuilder::new(&p.program);
    cmd.args(&p.args);
    let lang = std::env::var("LANG").ok();
    for (k, v) in shells::env(lang.as_deref()) {
        cmd.env(k, v);
    }
    // Left over when the app itself was started from a terminal; they would describe that one, not this.
    for k in ["TERM_PROGRAM_VERSION", "TERM_SESSION_ID", "ITERM_SESSION_ID", "WT_SESSION", "WT_PROFILE_ID"] {
        cmd.env_remove(k);
    }
    if let Some(h) = home() {
        cmd.cwd(h);
    }
    let mut child = pair.slave.spawn_command(cmd).map_err(io_err)?;
    drop(pair.slave); // the shell holds the only slave side, so its exit ends our reads
    let pid = child.process_id();
    let mut reader = pair.master.try_clone_reader().map_err(io_err)?;
    let mut writer = pair.master.take_writer().map_err(io_err)?;
    // None once the shell has been waited for: its PID may belong to another process by then.
    let killer: Arc<Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>> =
        Arc::new(Mutex::new(Some(child.clone_killer())));
    let master = Arc::new(Mutex::new(pair.master));

    let (out_tx, out_rx) = smol::channel::bounded::<FromHub>(256);
    let (in_tx, in_rx) = smol::channel::bounded::<ToHub>(256);
    let _ = out_tx.try_send(FromHub::Status("open".into()));

    // Output. It ends at EOF (or EIO once the shell and everything it started are gone).
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let tx = out_tx.clone();
    std::thread::Builder::new().name("local-read".into()).spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            if tx.send_blocking(FromHub::Data(buf[..n].to_vec())).is_err() {
                break; // the tab is gone
            }
        }
        let _ = done_tx.send(());
    })?;

    // Input and size. A close, or the tab dropping its end, kills the shell.
    let m = master.clone();
    let k = killer.clone();
    std::thread::Builder::new().name("local-write".into()).spawn(move || {
        while let Ok(msg) = in_rx.recv_blocking() {
            match msg {
                ToHub::Input(b) => {
                    if writer.write_all(&b).and_then(|_| writer.flush()).is_err() {
                        break;
                    }
                }
                ToHub::Resize { cols, rows } => {
                    if let Ok(m) = m.lock() {
                        let _ = m.resize(size(cols, rows));
                    }
                }
                ToHub::Focus | ToHub::Redraw => {}
                ToHub::Close => break,
            }
        }
        if let Ok(mut k) = k.lock()
            && let Some(k) = k.as_mut()
        {
            let _ = k.kill();
        }
    })?;

    // The end: the code once the shell exits, after the last output (a program it left behind holding the
    // terminal gets a moment, not forever).
    std::thread::Builder::new().name("local-wait".into()).spawn(move || {
        let st = child.wait().ok();
        killer.lock().unwrap_or_else(|e| e.into_inner()).take(); // a close after this would signal a stranger
        let _ = done_rx.recv_timeout(Duration::from_millis(800));
        let code = st.as_ref().map(|s| s.exit_code() as i64);
        let note = st.as_ref().and_then(|s| s.signal().map(|sig| format!("The shell was stopped ({sig}).")));
        let _ = out_tx.send_blocking(FromHub::Exit { code, note });
        let _ = out_tx.send_blocking(FromHub::Closed(Closed::Normal));
    })?;

    Ok((Conn { tx: in_tx, rx: out_rx }, Pty { master, pid }))
}

/// portable-pty returns anyhow errors; this keeps anyhow out of our own dependency list.
fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}
