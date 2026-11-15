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
/// Output is sent at most this often while it keeps coming, each time with everything read since (up to about
/// `CHUNK`). A program that writes in hundreds of tiny pieces a second (ConPTY repainting on Windows) then costs a
/// handful of messages instead of one each; the first piece after a quiet moment still goes out at once.
const GATHER: Duration = Duration::from_millis(5);
/// How far into the output ConPTY's cursor request is looked for before giving up on it.
const ASK_WITHIN: usize = 4096;
/// Input is queued in pieces of at most this much.
pub const INPUT_PIECE: usize = 16 * 1024;
/// Input pieces waiting for the process to read them (4 MiB).
const INPUT_BACKLOG: usize = 256;
/// How long a write waits for the process to read before the terminal counts as stuck and is ended.
pub const INPUT_PATIENCE: Duration = Duration::from_secs(60);
/// What ConPTY writes first on Windows: a cursor position request (portable-pty opens it inheriting the cursor).
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
    // Only the start of the output needs an answer; the clone goes once it is given (or given up on), so dropping
    // the Term still closes the input.
    let mut answer = cfg!(windows).then(|| (in_tx.clone(), Asked::default()));
    // The reader hands what it reads to a second thread, which sends it on in fewer, larger messages (`GATHER`).
    let (read_tx, read_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(8);
    std::thread::Builder::new().name("pty-read".into()).spawn(move || {
        let burst = (rate / 4.0).max(CHUNK as f64);
        let (mut tokens, mut last) = (burst, Instant::now());
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = match reader.read(&mut buf) {
                Ok(0) | Err(_) => break, // EOF, or EIO once the process is gone
                Ok(n) => n,
            };
            let mut out = buf[..n].to_vec();
            if let Some((tx, asked)) = answer.as_mut() {
                let (rest, found) = asked.take(&out);
                out = rest;
                if found == Some(true) {
                    // ConPTY asks where the cursor is and holds the process until it hears back. Nobody may be
                    // watching yet, and a viewer's answer would be a second one, so answer here and pass nothing on.
                    let _ = tx.try_send(b"\x1b[1;1R".to_vec());
                }
                if found.is_some() {
                    answer = None;
                }
                if out.is_empty() {
                    continue;
                }
            }
            let now = Instant::now();
            tokens = (tokens + now.duration_since(last).as_secs_f64() * rate).min(burst) - out.len() as f64;
            last = now;
            if read_tx.send(out).is_err() {
                break; // nobody is listening any more
            }
            if tokens < 0.0 {
                std::thread::sleep(Duration::from_secs_f64(-tokens / rate));
            }
        }
        // the output ended while the start of a request was held back: those bytes were output after all
        if let Some((_, asked)) = answer
            && !asked.held.is_empty()
        {
            let _ = read_tx.send(asked.held);
        }
    })?;

    std::thread::Builder::new().name("pty-out".into()).spawn(move || {
        let mut sent = None::<Instant>;
        while let Ok(first) = read_rx.recv() {
            // right after a send, wait out the rest of its `GATHER`; after a quiet moment, send at once
            let until = sent.map_or_else(Instant::now, |t| t + GATHER);
            let out = gather(first, &read_rx, until, CHUNK);
            sent = Some(Instant::now());
            if out_tx.blocking_send(out).is_err() {
                break; // nobody is listening any more
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

/// `out` and everything else read before `until`, as one piece. It stops early at `max` bytes (the piece may run
/// over by less than one read) and when the output ends. Past `until` it still takes what is already waiting, but
/// waits for nothing.
fn gather(mut out: Vec<u8>, more: &std::sync::mpsc::Receiver<Vec<u8>>, until: Instant, max: usize) -> Vec<u8> {
    while out.len() < max {
        match more.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(b) => out.extend_from_slice(&b),
            Err(_) => break, // the time is up with nothing waiting, or the output ended
        }
    }
    out
}

/// Looks for ConPTY's cursor request at the start of the output. It is usually the first thing written, but may
/// come in a later read, or split across two.
#[derive(Default)]
struct Asked {
    /// The end of the last read, when it could be the start of the request.
    held: Vec<u8>,
    seen: usize,
}

impl Asked {
    /// The output to pass on from this read, and whether the search is over: Some(true) when the request was found
    /// (and taken out), Some(false) when it was not in the first `ASK_WITHIN` bytes.
    fn take(&mut self, data: &[u8]) -> (Vec<u8>, Option<bool>) {
        let mut all = std::mem::take(&mut self.held);
        all.extend_from_slice(data);
        if let Some(at) = all.windows(CONPTY_ASKS.len()).position(|w| w == CONPTY_ASKS) {
            all.drain(at..at + CONPTY_ASKS.len());
            return (all, Some(true));
        }
        self.seen += data.len();
        if self.seen >= ASK_WITHIN {
            return (all, Some(false));
        }
        // keep back an end that could be the request's first bytes, until the next read shows
        let keep = (1..CONPTY_ASKS.len()).rev().find(|&k| all.ends_with(&CONPTY_ASKS[..k])).unwrap_or(0);
        self.held = all.split_off(all.len() - keep);
        (all, None)
    }
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

    #[test]
    fn many_small_writes_go_out_as_a_few_pieces() {
        // what ConPTY does on Windows: hundreds of tiny writes a second
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(8);
        let writer = std::thread::spawn(move || {
            for i in 0..400u32 {
                tx.send(format!("\x1b[{};1Hx", i % 24 + 1).into_bytes()).unwrap();
                if i % 40 == 39 {
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        });
        let (mut pieces, mut bytes, mut sent) = (0, 0, None::<Instant>);
        while let Ok(first) = rx.recv() {
            let until = sent.map_or_else(Instant::now, |t| t + GATHER);
            let out = gather(first, &rx, until, CHUNK);
            sent = Some(Instant::now());
            pieces += 1;
            bytes += out.len();
        }
        writer.join().unwrap();
        let wrote: usize = (0..400u32).map(|i| format!("\x1b[{};1Hx", i % 24 + 1).len()).sum();
        assert_eq!(bytes, wrote, "nothing lost, nothing doubled");
        assert!(pieces <= 100, "{pieces} pieces for 400 writes");
    }

    #[test]
    fn gathering_keeps_the_order_and_stops_at_the_size_and_the_time() {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(8);
        for part in ["b", "c", "d"] {
            tx.send(part.as_bytes().to_vec()).unwrap();
        }
        // the time is already up: what is waiting is still taken, and nothing is waited for
        let began = Instant::now();
        assert_eq!(gather(b"a".to_vec(), &rx, Instant::now(), CHUNK), b"abcd");
        assert!(began.elapsed() < Duration::from_millis(100));
        // a full piece is sent without looking further
        tx.send(b"later".to_vec()).unwrap();
        assert_eq!(gather(vec![b'x'; 8], &rx, Instant::now() + Duration::from_secs(5), 8), vec![b'x'; 8]);
        assert_eq!(gather(b"".to_vec(), &rx, Instant::now(), 8), b"later");
        // a single keystroke's echo waits no longer than the window
        let began = Instant::now();
        assert_eq!(gather(b"k".to_vec(), &rx, Instant::now() + GATHER, CHUNK), b"k");
        assert!(began.elapsed() < Duration::from_millis(250), "{:?}", began.elapsed());
        // the end of the output ends the wait
        drop(tx);
        let began = Instant::now();
        assert_eq!(gather(b"z".to_vec(), &rx, Instant::now() + Duration::from_secs(5), CHUNK), b"z");
        assert!(began.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn the_cursor_request_is_found_in_a_later_read_or_split_in_two() {
        // in the first read, among other output
        let mut a = Asked::default();
        assert_eq!(a.take(b"ab\x1b[6ncd"), (b"abcd".to_vec(), Some(true)));
        // in the second read
        let mut a = Asked::default();
        assert_eq!(a.take(b"\x1b[?25l"), (b"\x1b[?25l".to_vec(), None));
        assert_eq!(a.take(b"\x1b[6n"), (vec![], Some(true)));
        // split two and two: the first half is held back, then both are taken out
        let mut a = Asked::default();
        assert_eq!(a.take(b"hi\x1b["), (b"hi".to_vec(), None));
        assert_eq!(a.take(b"6nthere"), (b"there".to_vec(), Some(true)));
        // a held end that turns out to be something else is passed on, in order
        let mut a = Asked::default();
        assert_eq!(a.take(b"x\x1b"), (b"x".to_vec(), None));
        assert_eq!(a.take(b"[2J"), (b"\x1b[2J".to_vec(), None));
        // never asked: the search stops, and nothing is kept back
        let mut a = Asked::default();
        assert_eq!(a.take(&vec![b'.'; ASK_WITHIN - 1]).1, None);
        let (out, found) = a.take(b"\x1b");
        assert_eq!((out, found), (b"\x1b".to_vec(), Some(false)));
        assert!(a.held.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_writing_in_tiny_pieces_reaches_the_viewer_whole_and_in_few_messages() {
        let script = "i=0; while [ $i -lt 300 ]; do printf 'line %03d;' $i; i=$((i+1)); done; printf END";
        let (_term, mut io) = run(&["/bin/sh".into(), "-c".into(), script.into()]);
        let (mut all, mut pieces) = (Vec::new(), 0);
        while let Some(b) = io.output.recv().await {
            all.extend_from_slice(&b);
            pieces += 1;
        }
        let text = String::from_utf8_lossy(&all);
        let want: String = (0..300).map(|i| format!("line {i:03};")).collect::<String>() + "END";
        assert_eq!(text, want);
        assert!(pieces < 150, "{pieces} messages for 300 writes");
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
