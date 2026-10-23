//! One terminal: a process (ssh to the machine) in a PTY, with its output read on a thread and paced to a byte
//! rate, its input written on another thread, and its exit code reported once.

use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};

pub struct Term {
    /// On Windows, None once the process has exited: ConPTY ends the output only when it is closed.
    master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    /// None once the process has been waited for: its PID may belong to another process by then.
    killer: Arc<Mutex<Option<Box<dyn ChildKiller + Send + Sync>>>>,
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
/// The first thing ConPTY writes on Windows: a cursor position request (portable-pty opens it inheriting the cursor).
const CONPTY_ASKS: &[u8] = b"\x1b[6n";

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
    let killer = Arc::new(Mutex::new(Some(child.clone_killer())));
    let master = Arc::new(Mutex::new(Some(pair.master)));

    let (in_tx, mut in_rx) = mpsc::channel::<Vec<u8>>(INPUT_BACKLOG);
    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(64);
    let rate = s.rate.max(1) as f64;
    let answer = in_tx.clone();
    std::thread::Builder::new().name("pty-read".into()).spawn(move || {
        let burst = (rate / 4.0).max(CHUNK as f64);
        let (mut tokens, mut last) = (burst, Instant::now());
        let mut buf = vec![0u8; CHUNK];
        let mut first = cfg!(windows);
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break, // EOF, or EIO once the process is gone
                Ok(n) => n,
            };
            let mut out = &buf[..n];
            if std::mem::take(&mut first) && out.starts_with(CONPTY_ASKS) {
                // ConPTY asks where the cursor is and holds the process until it hears back. Nobody may be
                // watching yet, and a viewer's answer would be a second one, so answer here and pass nothing on.
                let _ = answer.try_send(b"\x1b[1;1R".to_vec());
                out = &out[CONPTY_ASKS.len()..];
                if out.is_empty() {
                    continue;
                }
            }
            let now = Instant::now();
            tokens = (tokens + now.duration_since(last).as_secs_f64() * rate).min(burst) - out.len() as f64;
            last = now;
            if out_tx.blocking_send(out.to_vec()).is_err() {
                break; // nobody is listening any more
            }
            if tokens < 0.0 {
                std::thread::sleep(Duration::from_secs_f64(-tokens / rate));
            }
        }
    })?;

    std::thread::Builder::new().name("pty-write".into()).spawn(move || {
        while let Some(b) = in_rx.blocking_recv() {
            if writer.write_all(&b).and_then(|_| writer.flush()).is_err() {
                break;
            }
        }
    })?;

    let (exit_tx, exit_rx) = oneshot::channel();
    let (reaped, console) = (killer.clone(), master.clone());
    std::thread::Builder::new().name("pty-wait".into()).spawn(move || {
        let code = child.wait().ok().map(|st| st.exit_code() as i64);
        reaped.lock().unwrap_or_else(|e| e.into_inner()).take(); // a hang up after this would signal a stranger
        if cfg!(windows) {
            // ConPTY keeps the output open after the process ends; closing it lets the reader drain it and stop
            let m = console.lock().unwrap_or_else(|e| e.into_inner()).take();
            drop(m);
        }
        let _ = exit_tx.send(code);
    })?;

    Ok((Term { master, killer, input: in_tx }, TermIo { output: out_rx, exited: exit_rx }))
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
        if let Ok(m) = self.master.lock()
            && let Some(m) = m.as_ref()
        {
            let _ = m.resize(size(cols, rows));
        }
    }

    /// Hang up: ssh gets SIGHUP and closes, and on the machine `claude attach` detaches. The session keeps running.
    /// Nothing once it has exited.
    pub fn hang_up(&self) {
        if let Ok(mut k) = self.killer.lock()
            && let Some(k) = k.as_mut()
        {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn exit_with(code: u8) -> Vec<String> {
        let (shell, flag) = if cfg!(windows) { ("cmd.exe", "/c") } else { ("/bin/sh", "-c") };
        vec![shell.into(), flag.into(), format!("exit {code}")]
    }

    fn run(argv: &[String]) -> (Term, TermIo) {
        spawn(Spawn { argv, env: &[], env_remove: &[], cwd: None, cols: 80, rows: 24, rate: 1 << 20 }).unwrap()
    }

    #[tokio::test]
    async fn an_exited_process_is_never_signalled() {
        let (term, mut io) = run(&exit_with(3));
        while io.output.recv().await.is_some() {}
        assert_eq!(io.exited.await.unwrap(), Some(3));
        assert!(term.killer.lock().unwrap().is_none(), "its PID may be someone else's now");
        drop(term); // hangs up nothing
    }

    #[tokio::test]
    async fn output_ends_when_the_process_exits() {
        // the Term stays alive, so only the exit can end the output (on Windows, ConPTY needed closing for that)
        let (_term, mut io) = run(&exit_with(2));
        let drained = async { while io.output.recv().await.is_some() {} };
        assert!(tokio::time::timeout(Duration::from_secs(10), drained).await.is_ok(), "the output never ended");
        assert_eq!(io.exited.await.unwrap(), Some(2));
    }
}
