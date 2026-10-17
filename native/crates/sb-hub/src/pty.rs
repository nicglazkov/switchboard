//! One terminal: a process (ssh to the machine) in a PTY, with its output read on a thread and paced to a byte
//! rate, its input written on another thread, and its exit code reported once.

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

pub struct Term {
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    input: mpsc::Sender<Vec<u8>>,
}

pub struct TermIo {
    /// Output chunks; closed once the process has exited and everything it wrote is read.
    pub output: mpsc::Receiver<Vec<u8>>,
    /// The exit code (None if it was killed by a signal or could not be read).
    pub exited: oneshot::Receiver<Option<i64>>,
}

pub struct Spawn<'a> {
    pub argv: &'a [String],
    pub env: &'a [(String, String)],
    pub env_remove: &'a [String],
    pub cwd: Option<&'a std::path::Path>,
    pub cols: u16,
    pub rows: u16,
    /// Output bytes per second (the spec's 2 MB/s); reading pauses past it, so the PTY pushes back on the process.
    pub rate: u64,
}

const CHUNK: usize = 32 * 1024;
/// Input is queued in pieces of at most this much.
pub const INPUT_PIECE: usize = 16 * 1024;
/// Input pieces waiting for the process to read them (4 MiB).
const INPUT_BACKLOG: usize = 256;
/// How long a write waits for the process to read before the terminal counts as stuck and is ended.
pub const INPUT_PATIENCE: Duration = Duration::from_secs(60);

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows, cols, pixel_width: 0, pixel_height: 0 }
}

pub fn spawn(s: Spawn) -> std::io::Result<(Term, TermIo)> {
    let pair = native_pty_system().openpty(size(s.cols, s.rows)).map_err(io_err)?;
    let mut cmd = CommandBuilder::new(&s.argv[0]);
    cmd.args(&s.argv[1..]);
    for k in s.env_remove {
        cmd.env_remove(k);
    }
    for (k, v) in s.env {
        cmd.env(k, v);
    }
    if let Some(d) = s.cwd {
        cmd.cwd(d);
    }
    let mut child = pair.slave.spawn_command(cmd).map_err(io_err)?;
    drop(pair.slave); // the child holds the only slave side, so its exit ends our reads
    let mut reader = pair.master.try_clone_reader().map_err(io_err)?;
    let mut writer = pair.master.take_writer().map_err(io_err)?;
    let killer = child.clone_killer();

    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(64);
    let rate = s.rate.max(1) as f64;
    std::thread::Builder::new().name("pty-read".into()).spawn(move || {
        let burst = (rate / 4.0).max(CHUNK as f64);
        let (mut tokens, mut last) = (burst, Instant::now());
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break, // EOF, or EIO once the process is gone
                Ok(n) => n,
            };
            let now = Instant::now();
            tokens = (tokens + now.duration_since(last).as_secs_f64() * rate).min(burst) - n as f64;
            last = now;
            if out_tx.blocking_send(buf[..n].to_vec()).is_err() {
                break; // nobody is listening any more
            }
            if tokens < 0.0 {
                std::thread::sleep(Duration::from_secs_f64(-tokens / rate));
            }
        }
    })?;

    let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(INPUT_BACKLOG);
    std::thread::Builder::new().name("pty-write".into()).spawn(move || {
        while let Some(b) = in_rx.blocking_recv() {
            if writer.write_all(&b).and_then(|_| writer.flush()).is_err() {
                break;
            }
        }
    })?;

    let (exit_tx, exit_rx) = oneshot::channel();
    std::thread::Builder::new().name("pty-wait".into()).spawn(move || {
        let code = child.wait().ok().map(|st| st.exit_code() as i64);
        let _ = exit_tx.send(code);
    })?;

    Ok((
        Term { master: Mutex::new(pair.master), killer: Mutex::new(killer), input: in_tx },
        TermIo { output: out_rx, exited: exit_rx },
    ))
}

impl Term {
    /// Writes without waiting (switchboard-node's PTYs): input past the backlog ends the terminal rather than grow
    /// without limit.
    pub fn write(&self, bytes: Vec<u8>) {
        for piece in bytes.chunks(INPUT_PIECE) {
            if let Err(mpsc::error::TrySendError::Full(_)) = self.input.try_send(piece.to_vec()) {
                self.hang_up(); // typed faster than ssh reads, for a long time: end it rather than grow without limit
                return;
            }
        }
    }

    /// Writes, waiting while the process is slow to read (a big paste), so the backlog stays bounded without
    /// ending the terminal. False if it ended, or took nothing for `INPUT_PATIENCE` (then it is hung up).
    pub async fn write_all(&self, bytes: &[u8]) -> bool {
        for piece in bytes.chunks(INPUT_PIECE) {
            match tokio::time::timeout(INPUT_PATIENCE, self.input.send(piece.to_vec())).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return false,
                Err(_) => {
                    self.hang_up();
                    return false;
                }
            }
        }
        true
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        if let Ok(m) = self.master.lock() {
            let _ = m.resize(size(cols, rows));
        }
    }

    /// Hang up: ssh gets SIGHUP and closes, and on the machine `claude attach` detaches. The session keeps running.
    pub fn hang_up(&self) {
        if let Ok(mut k) = self.killer.lock() {
            let _ = k.kill();
        }
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        self.hang_up();
    }
}

/// portable-pty returns anyhow errors; this keeps anyhow out of our own dependency list.
fn io_err(e: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(e.to_string())
}
