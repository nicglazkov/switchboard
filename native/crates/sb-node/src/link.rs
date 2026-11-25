//! The link: one WebSocket to the hub's `/node`, kept up with backoff. The hub asks for RPCs (answered by running
//! `sb.py rpc`) and terminals (a PTY running sb.py's term mode, which execs `claude attach <bg>`, exactly what the
//! hub's ssh gets). Closing a terminal only detaches; the session keeps running.

use crate::{NodeConfig, Own, VERSION};
use futures_util::{SinkExt, StreamExt};
use sb_hub::link_proto::{self, CLOSE_REPLACED, CLOSE_UNPAIRED, FromNode, ToNode};
use sb_hub::pty::{self, Term};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};

const PING_EVERY: Duration = Duration::from_secs(20);
const RATE: u64 = 2 * 1024 * 1024;
/// Messages waiting to go out to the hub; past it, terminal output waits (the PTY pushes back on the process).
const LINK_BACKLOG: usize = 256;
/// How long one message to the hub may take to leave. A hub that stopped reading (its machine froze, the network
/// went away) fills the socket; past this the link is given up and made again.
const SEND_PATIENCE: Duration = if cfg!(test) { Duration::from_millis(500) } else { Duration::from_secs(20) };
/// A large message (a piece of a file) gets a twentieth of that more for each 32 KB (a second, as the app is
/// built): slow is not stuck.
fn send_patience(size: usize) -> Duration {
    SEND_PATIENCE + (SEND_PATIENCE / 20) * (size / (32 << 10)) as u32
}

/// Input waiting for one terminal's program before that terminal is ended. The hub cannot be told to slow down,
/// so past this the choice is ending the terminal or growing without limit.
const INPUT_WAITING: usize = 32 << 20;
/// The largest `rpc.result` sent: the hub drops a link that sends a message over 4 MiB, and with it every
/// terminal on this machine.
const RESULT_MAX: usize = (4 << 20) - 4096;
/// The most `sb.py rpc` may print before it is stopped. Its answer is one line that has to fit in `RESULT_MAX`;
/// twice that leaves room for whatever it printed before the answer.
const OUTPUT_MAX: usize = 8 << 20;
/// How much of what it wrote to stderr is kept, from the end: the last line is the error shown.
const ERRORS_KEPT: usize = 64 << 10;

pub(crate) fn log(msg: impl AsRef<str>) {
    eprintln!("switchboard-node: {}", msg.as_ref());
}

/// Why reconnecting at once cannot help.
enum Stop {
    /// The hub no longer knows this token (unpaired, or paired again). `linked` is false when the hub turned the
    /// token down before any link.
    Unpaired { why: String, linked: bool },
    /// Another node with this token took the link.
    Replaced,
}

/// How `run` ended.
#[derive(Debug, PartialEq, Eq)]
pub enum End {
    /// The process ends with this code: a signal, or (0) an update replaced this node and the new one has the link.
    Exit(i32),
    /// The hub no longer knows the token. `linked` is false when it was turned down without ever linking.
    Unpaired { why: String, linked: bool },
    /// Another node with the same token took the link.
    Replaced,
}

/// Held while this node has the link: a file next to node.json, locked. A node on this computer that was replaced
/// looks at it before it tries again, so two nodes here never take the link from each other in turns.
struct Holding {
    path: PathBuf,
    file: Option<std::fs::File>,
}

impl Holding {
    /// Takes the lock if it is free (asked again now and then: the node this one replaced lets go a moment later).
    fn take(&mut self) {
        if self.file.is_none()
            && let Ok(f) = lock_file(&self.path)
            && f.try_lock().is_ok()
        {
            self.file = Some(f);
        }
    }
}

fn lock_file(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(path)
}

/// True when another node on this computer has the link right now. False when nobody does, and when that cannot
/// be told (the file cannot be made, or this system has no file locks).
pub fn held_elsewhere(lock: &Path) -> bool {
    lock_file(lock).is_ok_and(|f| matches!(f.try_lock(), Err(std::fs::TryLockError::WouldBlock)))
}

/// Keep the link up until the token stops working or a signal arrives. `own` is this node's program file as it
/// was at the start, to tell an update's leftover process (Windows) from a second node. `lock` is the file held
/// while linked. `again` is a retry of a token the hub turned down or another node took: it says nothing unless
/// something changes.
pub async fn run(cfg: NodeConfig, own: Option<&Own>, lock: &Path, again: bool) -> End {
    if !again {
        log(format!("{VERSION} linking {} to the hub at {}", cfg.machine, cfg.hub));
    }
    let forever = async {
        let mut wait = 1u64;
        loop {
            let began = Instant::now();
            match link_once(&cfg, lock).await {
                Ok(why) => log(format!("link closed: {why}")),
                Err(Ok(Stop::Unpaired { why, linked })) => return End::Unpaired { why, linked },
                Err(Ok(Stop::Replaced)) => {
                    // On Windows an update cannot stop or replace the node that is running, only move its file
                    // aside; the new node then takes the link. That is this process's cue to end.
                    if cfg!(windows) && own.is_some_and(Own::superseded) {
                        log("an update replaced this switchboard-node and the new one has the link; exiting.");
                        return End::Exit(0);
                    }
                    return End::Replaced;
                }
                Err(Err(e)) => log(format!("cannot reach the hub at {} ({e}); trying again in {wait} s", cfg.hub)),
            }
            if began.elapsed() > Duration::from_secs(60) {
                wait = 1;
            }
            tokio::time::sleep(Duration::from_secs(wait)).await;
            wait = (wait * 2).min(30);
        }
    };
    tokio::select! {
        code = forever => code,
        _ = shutdown() => {
            log("stopping; open terminals detach");
            End::Exit(0)
        }
    }
}

pub async fn shutdown() {
    #[cfg(unix)]
    {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// How long a request from the hub that is still running when the link ends is given to finish. Stopping sb.py
/// at once could catch it between two steps of one verb (a tool's folder taken away and its new one not yet in
/// place, a message taken to be sent with a relaunch that is then never queued); none of those takes this long.
/// One that is still running after it is stuck, or slow with nobody left to hear the answer, and is stopped.
const RPC_GRACE: Duration = Duration::from_secs(10);

/// One link, from connect to close. Ok(why) means try again; Err(Ok) means stop; Err(Err) is a failed connect.
async fn link_once(cfg: &NodeConfig, lock: &Path) -> Result<String, Result<Stop, String>> {
    link_for(cfg, lock, RPC_GRACE).await
}

async fn link_for(cfg: &NodeConfig, lock: &Path, grace: Duration) -> Result<String, Result<Stop, String>> {
    let mut req = format!("ws://{}/node", cfg.hub).into_client_request().map_err(|e| Err(e.to_string()))?;
    let auth =
        format!("Bearer {}", cfg.token).parse().map_err(|_| Err("the saved token is not valid text".to_string()))?;
    req.headers_mut().insert("Authorization", auth);
    let ws = match tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(req)).await {
        Err(_) => return Err(Err("timed out".into())),
        Ok(Err(tungstenite::Error::Http(r))) if r.status() == 401 => {
            let why = "The hub does not know this machine's token any more.".to_string();
            return Err(Ok(Stop::Unpaired { why, linked: false }));
        }
        Ok(Err(tungstenite::Error::Http(r))) => return Err(Err(format!("the hub answered {}", r.status()))),
        Ok(Err(e)) => return Err(Err(e.to_string())),
        Ok(Ok((ws, _))) => ws,
    };
    log("linked");
    let mut holding = Holding { path: lock.to_path_buf(), file: None };
    holding.take();
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::channel::<Message>(LINK_BACKLOG); // bounded: output waits for the hub, never piles up
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<u32>();
    let label = crate::hostname_label();
    let _ = tx
        .send(Message::Text(
            link_proto::encode(&FromNode::Hello { version: VERSION.into(), os: crate::os_name().into(), label }).into(),
        ))
        .await;
    let mut writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            let size = match &m {
                Message::Text(t) => t.len(),
                Message::Binary(b) => b.len(),
                _ => 0,
            };
            if !matches!(tokio::time::timeout(send_patience(size), sink.send(m)).await, Ok(Ok(()))) {
                break; // dropping `rx` here lets everything waiting to send (terminal output) go
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), sink.close()).await;
    });
    let mut writer_done = false;
    let mut terms: HashMap<u32, Open> = HashMap::new();
    // The requests that are running. None outlives the link by more than `grace` (see RPC_GRACE): its answer has
    // nowhere to go, and the hub has already told the server that the link dropped.
    let mut rpcs = tokio::task::JoinSet::new();
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut heard = Instant::now();
    let end = loop {
        tokio::select! {
            m = stream.next() => {
                heard = Instant::now();
                match m {
                    Some(Ok(Message::Text(t))) => match serde_json::from_str::<ToNode>(t.as_str()) {
                        Ok(ToNode::Rpc { id, req, timeout }) => {
                            let (tx, cfg) = (tx.clone(), cfg.clone());
                            rpcs.spawn(async move {
                                let res = rpc(&cfg, &req, timeout).await;
                                let _ = tx.send(Message::Text(result_message(id, res).into())).await;
                            });
                        }
                        Ok(ToNode::TermOpen { ch, bg, cols, rows }) => {
                            if let Some(t) = open_term(cfg, ch, &bg, cols, rows, tx.clone(), done_tx.clone()).await {
                                terms.insert(ch, Open::start(t, INPUT_WAITING));
                            }
                        }
                        Ok(ToNode::TermResize { ch, cols, rows }) => {
                            if let Some(t) = terms.get(&ch) {
                                let (c, r) = sb_hub::proto::clamp_size(cols.into(), rows.into());
                                t.term.resize(c, r);
                            }
                        }
                        Ok(ToNode::TermClose { ch }) => {
                            terms.remove(&ch); // hangs up: claude attach detaches
                        }
                        Err(_) => {} // a newer hub's message
                    },
                    Some(Ok(Message::Binary(b))) => {
                        if let Some((ch, data)) = link_proto::unframe(&b)
                            && let Some(t) = terms.get(&ch)
                            && !t.write(data)
                        {
                            log(format!("terminal {ch}: input fell far behind, closing it"));
                            terms.remove(&ch); // hangs up; its output ends and the hub hears `term.exit`
                        }
                    }
                    Some(Ok(Message::Close(f))) => {
                        let (code, why) = f.map(|f| (u16::from(f.code), f.reason.to_string())).unwrap_or((1005, String::new()));
                        if code == CLOSE_UNPAIRED {
                            let why = if why.is_empty() { "The hub unlinked this machine.".into() } else { why };
                            break Err(Ok(Stop::Unpaired { why, linked: true }));
                        }
                        if code == CLOSE_REPLACED {
                            break Err(Ok(Stop::Replaced));
                        }
                        break Ok(format!("the hub closed it ({code})"));
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => break Ok(e.to_string()),
                    None => break Ok("the connection ended".into()),
                }
            }
            Some(ch) = done_rx.recv() => {
                terms.remove(&ch);
            }
            Some(_) = rpcs.join_next() => {} // one finished: nothing is kept of it
            _ = &mut writer, if !writer_done => {
                writer_done = true;
                break Ok("the hub stopped taking messages".into());
            }
            _ = ping.tick() => {
                if heard.elapsed() > PING_EVERY * 3 {
                    break Ok("the hub went quiet".into());
                }
                let _ = tx.try_send(Message::Ping(Vec::new().into()));
                holding.take();
            }
        }
    };
    if !rpcs.is_empty() {
        // on a task of its own: the next link does not wait for them
        tokio::spawn(async move {
            let all = async { while rpcs.join_next().await.is_some() {} };
            if tokio::time::timeout(grace, all).await.is_err() {
                let running = rpcs.len();
                rpcs.shutdown().await; // (and its sb.py: kill_on_drop)
                let s = grace.as_secs();
                log(format!("{running} request(s) from the hub still running {s} s after the link ended: stopped"));
            }
        });
    }
    let open = terms.len();
    terms.clear();
    if open > 0 {
        log(format!("{open} terminal(s) detached"));
    }
    let _ = tx.try_send(Message::Close(Some(tungstenite::protocol::CloseFrame {
        code: CloseCode::Normal,
        reason: "".into(),
    })));
    drop(tx);
    if !writer_done && tokio::time::timeout(Duration::from_secs(2), &mut writer).await.is_err() {
        writer.abort(); // stuck sending to a hub that takes nothing: let the socket go with it
    }
    end
}

/// A terminal on this machine with its input. Input is written by a task of its own, which waits while the program
/// is slow to read (a big paste), so neither the link nor the terminal is held up or ended by it. Dropping this
/// hangs up.
struct Open {
    term: Arc<Term>,
    input: mpsc::UnboundedSender<Vec<u8>>,
    waiting: Arc<AtomicUsize>,
    bound: usize,
}

impl Open {
    fn start(term: Term, bound: usize) -> Open {
        let term = Arc::new(term);
        let (input, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let waiting = Arc::new(AtomicUsize::new(0));
        let (t, w) = (term.clone(), waiting.clone());
        tokio::spawn(async move {
            while let Some(b) = rx.recv().await {
                let ok = t.write_all(&b).await;
                w.fetch_sub(b.len(), Ordering::Relaxed);
                if !ok {
                    break; // the program ended, or took nothing for a minute and was hung up
                }
            }
        });
        Open { term, input, waiting, bound }
    }

    /// Queues input for the program. False when more than the bound is already waiting: the caller ends the
    /// terminal.
    fn write(&self, data: &[u8]) -> bool {
        if self.waiting.load(Ordering::Relaxed) + data.len() > self.bound {
            return false;
        }
        self.waiting.fetch_add(data.len(), Ordering::Relaxed);
        if self.input.send(data.to_vec()).is_err() {
            self.waiting.fetch_sub(data.len(), Ordering::Relaxed); // the terminal has ended; nothing to do
        }
        true
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        self.term.hang_up(); // the input task may still hold the terminal for a moment
    }
}

/// The `rpc.result` message for the hub. An answer too large for the link becomes an error instead, so one
/// oversized answer fails one request rather than dropping the link.
fn result_message(id: u64, res: Value) -> String {
    let text = link_proto::encode(&FromNode::RpcResult { id, res });
    if text.len() <= RESULT_MAX {
        return text;
    }
    let mb = text.len() as f64 / (1 << 20) as f64;
    let res = json!({"ok": false, "error": format!("The answer was too large to send ({mb:.1} MB).")});
    link_proto::encode(&FromNode::RpcResult { id, res })
}

/// Everything `from` writes, up to `max` bytes. None when it wrote more: the caller stops the program.
async fn read_capped(from: Option<impl AsyncRead + Unpin>, max: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let Some(mut from) = from else { return Some(out) };
    let mut buf = vec![0u8; 64 << 10];
    loop {
        match from.read(&mut buf).await {
            Ok(0) | Err(_) => return Some(out),
            Ok(n) if out.len() + n > max => return None,
            Ok(n) => out.extend_from_slice(&buf[..n]),
        }
    }
}

/// The last `keep` bytes (or a little more) of everything `from` writes. It is read to its end either way, so a
/// program that writes a lot is never left waiting on a full pipe.
async fn read_tail(from: Option<impl AsyncRead + Unpin>, keep: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let Some(mut from) = from else { return out };
    let mut buf = vec![0u8; 16 << 10];
    loop {
        match from.read(&mut buf).await {
            Ok(0) | Err(_) => return out,
            Ok(n) => out.extend_from_slice(&buf[..n]),
        }
        if out.len() > keep * 2 {
            out.drain(..out.len() - keep);
        }
    }
}

/// `sb.py rpc` with the request on stdin; its JSON answer (the last line it prints), or why there is none.
async fn rpc(cfg: &NodeConfig, req: &Value, timeout: u64) -> Value {
    rpc_capped(cfg, req, timeout, OUTPUT_MAX).await
}

async fn rpc_capped(cfg: &NodeConfig, req: &Value, timeout: u64, output_max: usize) -> Value {
    let mut cmd = tokio::process::Command::new(cfg.python());
    cmd.arg(cfg.sb()).arg("rpc").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    cmd.env_remove("SSH_ORIGINAL_COMMAND").current_dir(crate::home());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: never flash a console on the desktop
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "error": format!("could not run sb.py ({e})")}),
    };
    // The request is written while the answer is read, and both under the one timeout: a request larger than the
    // pipe holds, sent to an sb.py that is stuck before it reads, would otherwise wait here for good.
    let stdin = child.stdin.take();
    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    let body = req.to_string().into_bytes();
    let work = async move {
        let send = async move {
            if let Some(mut stdin) = stdin {
                let _ = stdin.write_all(&body).await;
            } // dropped: sb.py sees the end of its input
            Ok::<(), ()>(())
        };
        // What it prints is read as it comes and counted: held until it exits, a helper that never stops printing
        // would grow this process until the timeout. Past the limit nothing more is waited for (try_join), and
        // the child is stopped.
        let printed = async { read_capped(stdout, output_max).await.ok_or(()) };
        let errors = async { Ok::<Vec<u8>, ()>(read_tail(stderr, ERRORS_KEPT).await) };
        let Ok(((), stdout, stderr)) = tokio::try_join!(send, printed, errors) else {
            let _ = child.kill().await;
            return Ok(None);
        };
        child.wait().await.map(|status| Some((status, stdout, stderr)))
    };
    // (on a timeout the child goes with `work`: kill_on_drop)
    let (status, stdout, stderr) = match tokio::time::timeout(Duration::from_secs(timeout.max(1)), work).await {
        Err(_) => return json!({"ok": false, "error": format!("sb.py did not answer in {timeout} s")}),
        Ok(Err(e)) => return json!({"ok": false, "error": format!("sb.py failed ({e})")}),
        Ok(Ok(None)) => {
            let mb = output_max as f64 / (1 << 20) as f64;
            return json!({"ok": false, "error": format!("sb.py printed more than {mb:.0} MB and was stopped.")});
        }
        Ok(Ok(Some(o))) => o,
    };
    let stdout = String::from_utf8_lossy(&stdout);
    match stdout.lines().rev().find(|l| !l.trim().is_empty()).and_then(|l| serde_json::from_str::<Value>(l).ok()) {
        Some(v) => v,
        None => {
            let err = String::from_utf8_lossy(&stderr);
            let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
            let why = if last.is_empty() {
                format!("sb.py exited {} with no answer", status.code().unwrap_or(-1))
            } else {
                last
            };
            json!({"ok": false, "error": why})
        }
    }
}

/// A terminal: sb.py's term mode in a PTY (it execs `claude attach <bg>`), its output framed onto the link, and
/// `term.exit` once it ends. A failure to start is shown in the terminal itself, then it exits.
async fn open_term(
    cfg: &NodeConfig,
    ch: u32,
    bg: &str,
    cols: u16,
    rows: u16,
    tx: mpsc::Sender<Message>,
    done: mpsc::UnboundedSender<u32>,
) -> Option<Term> {
    // Sent on its own task, waiting for room: on a busy link a dropped `term.exit` would leave the hub's channel
    // open with nothing behind it.
    let say_and_exit = |msg: String, code: i64| {
        let tx = tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(Message::Binary(link_proto::frame(ch, msg.as_bytes()).into())).await;
            let _ =
                tx.send(Message::Text(link_proto::encode(&FromNode::TermExit { ch, code: Some(code) }).into())).await;
        });
    };
    if !sb_hub::ticket::valid_bg(bg) {
        say_and_exit("Switchboard: that is not a background session id.\r\n".into(), 2);
        return None;
    }
    let argv = vec![cfg.python(), cfg.sb(), "rpc".into()];
    let env = vec![
        ("SSH_ORIGINAL_COMMAND".to_string(), format!("term {bg}")),
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
    ];
    let home = crate::home();
    let (c, r) = sb_hub::proto::clamp_size(cols.into(), rows.into());
    let spawned = tokio::task::spawn_blocking(move || {
        pty::spawn(pty::Spawn {
            argv: &argv,
            env: &env,
            env_remove: &[],
            cwd: Some(&home),
            cols: c,
            rows: r,
            rate: RATE,
        })
    })
    .await;
    let (term, mut io) = match spawned {
        Ok(Ok(x)) => x,
        Ok(Err(e)) => {
            log(format!("terminal {ch}: could not start sb.py: {e}"));
            say_and_exit(format!("Switchboard: could not start Claude Code here ({e}).\r\n"), 127);
            return None;
        }
        Err(e) => {
            say_and_exit(format!("Switchboard: could not start Claude Code here ({e}).\r\n"), 127);
            return None;
        }
    };
    log(format!("terminal {ch} open: attach {bg} {c}x{r}"));
    tokio::spawn(async move {
        while let Some(b) = io.output.recv().await {
            if tx.send(Message::Binary(link_proto::frame(ch, &b).into())).await.is_err() {
                break;
            }
        }
        let code =
            tokio::time::timeout(Duration::from_secs(3), &mut io.exited).await.ok().and_then(|r| r.ok()).flatten();
        let _ = tx.send(Message::Text(link_proto::encode(&FromNode::TermExit { ch, code }).into())).await;
        let _ = done.send(ch);
        log(format!("terminal {ch} closed ({code:?})"));
    });
    Some(term)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node whose "sb.py" is the shell script at `sb` (run with /bin/sh in place of Python).
    fn lab_cfg(hub: &str, sb: Option<&std::path::Path>) -> NodeConfig {
        NodeConfig {
            hub: hub.into(),
            machine: "lab".into(),
            token: "t".into(),
            sb: sb.map(|p| p.display().to_string()),
            python: sb.map(|_| "/bin/sh".to_string()),
        }
    }

    #[cfg(unix)]
    fn script(tag: &str, text: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("sbnode-link-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("sb.sh");
        std::fs::write(&p, text).unwrap();
        p
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_rpc_whose_request_is_never_read_still_times_out() {
        // sb.py stuck before it reads its input, and a request larger than a pipe holds
        let sb = script("stuck", "sleep 30\n");
        let cfg = lab_cfg("192.0.2.10:8132", Some(&sb));
        let req = json!({"verb": "x", "pad": "p".repeat(2 << 20)});
        let began = Instant::now();
        let res = tokio::time::timeout(Duration::from_secs(20), rpc(&cfg, &req, 1)).await.expect("rpc never came back");
        assert_eq!(res, json!({"ok": false, "error": "sb.py did not answer in 1 s"}));
        assert!(began.elapsed() < Duration::from_secs(10));
        let _ = std::fs::remove_dir_all(sb.parent().unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_rpc_with_a_large_request_and_answer_goes_through() {
        // the answer is written before the request is read to its end: both pipes must move at once
        let sb = script(
            "echo",
            "head -c 300000 /dev/zero | tr '\\0' 'a'; echo; cat >/dev/null; echo '{\"ok\": true, \"n\": 7}'\n",
        );
        let cfg = lab_cfg("192.0.2.10:8132", Some(&sb));
        let req = json!({"verb": "x", "pad": "p".repeat(2 << 20)});
        let res =
            tokio::time::timeout(Duration::from_secs(30), rpc(&cfg, &req, 20)).await.expect("rpc never came back");
        assert_eq!(res, json!({"ok": true, "n": 7}));
        let _ = std::fs::remove_dir_all(sb.parent().unwrap());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_rpc_that_never_stops_printing_is_stopped_at_the_limit() {
        // a helper gone wrong: it prints for good, and would be held in memory until the timeout
        let sb = script("flood-out", "cat >/dev/null; yes 0123456789abcdef; echo '{\"ok\": true}'\n");
        let cfg = lab_cfg("192.0.2.10:8132", Some(&sb));
        let began = Instant::now();
        let res = tokio::time::timeout(Duration::from_secs(20), rpc_capped(&cfg, &json!({}), 15, 1 << 20))
            .await
            .expect("rpc never came back");
        assert_eq!(res, json!({"ok": false, "error": "sb.py printed more than 1 MB and was stopped."}));
        assert!(began.elapsed() < Duration::from_secs(10), "it waited for the timeout: {:?}", began.elapsed());
        // under the limit nothing changes, and a helper that fills stderr is not left waiting on it
        let sb2 = script(
            "noisy",
            "cat >/dev/null; head -c 400000 /dev/zero | tr '\\0' 'e' >&2; echo >&2; echo last line >&2; exit 3\n",
        );
        let cfg = lab_cfg("192.0.2.10:8132", Some(&sb2));
        let res = tokio::time::timeout(Duration::from_secs(20), rpc_capped(&cfg, &json!({}), 15, 1 << 20))
            .await
            .expect("rpc never came back");
        assert_eq!(res, json!({"ok": false, "error": "last line"}));
        let _ = std::fs::remove_dir_all(sb.parent().unwrap());
        let _ = std::fs::remove_dir_all(sb2.parent().unwrap());
    }

    #[tokio::test]
    async fn reading_stops_at_the_limit_and_the_tail_keeps_the_end() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        assert_eq!(read_capped(Some(&data[..]), 200_000).await.as_deref(), Some(&data[..]));
        assert_eq!(read_capped(Some(&data[..]), 199_999).await, None);
        assert_eq!(read_capped(None::<&[u8]>, 10).await, Some(vec![]));
        let tail = read_tail(Some(&data[..]), 1000).await;
        assert!(tail.len() >= 1000 && tail.len() <= 2000 + (16 << 10), "{}", tail.len());
        assert!(data.ends_with(&tail));
        assert_eq!(read_tail(Some(&data[..10]), 1000).await, &data[..10]);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_still_running_when_the_link_ends_is_stopped() {
        // A hub that asks for something slow and then goes away.
        let hub = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = hub.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (s, _) = hub.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
            let m = ToNode::Rpc { id: 1, req: json!({}), timeout: 30 };
            ws.send(Message::Text(link_proto::encode(&m).into())).await.unwrap();
            tokio::time::sleep(Duration::from_millis(700)).await;
            drop(ws);
        });
        // it says when it started, and would say when it finished
        let sb = script("cancel", "cd \"$(dirname \"$0\")\"; cat >/dev/null; touch began; sleep 3; touch finished\n");
        let dir = sb.parent().unwrap().to_path_buf();
        let cfg = lab_cfg(&addr, Some(&sb));
        let lock = dir.join("node.lock");
        let grace = Duration::from_millis(500);
        let end =
            tokio::time::timeout(Duration::from_secs(10), link_for(&cfg, &lock, grace)).await.expect("never ended");
        assert!(end.is_ok(), "the link just dropped");
        assert!(dir.join("began").exists(), "the request was running");
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(!dir.join("finished").exists(), "it ran on after the link was gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_nearly_done_when_the_link_ends_is_let_finish() {
        // The hub restarts while sb.py is between the two steps of one verb: it is not stopped there.
        let hub = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = hub.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (s, _) = hub.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
            let m = ToNode::Rpc { id: 1, req: json!({}), timeout: 30 };
            ws.send(Message::Text(link_proto::encode(&m).into())).await.unwrap();
            tokio::time::sleep(Duration::from_millis(700)).await;
            drop(ws);
        });
        let sb = script("grace", "cd \"$(dirname \"$0\")\"; cat >/dev/null; touch began; sleep 2; touch finished\n");
        let dir = sb.parent().unwrap().to_path_buf();
        let cfg = lab_cfg(&addr, Some(&sb));
        let lock = dir.join("node.lock");
        let began = Instant::now();
        let end = tokio::time::timeout(Duration::from_secs(10), link_for(&cfg, &lock, RPC_GRACE)).await;
        assert!(end.expect("never ended").is_ok(), "the link just dropped");
        assert!(began.elapsed() < Duration::from_millis(1900), "the next link does not wait for it");
        assert!(dir.join("began").exists() && !dir.join("finished").exists(), "the request was running");
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(dir.join("finished").exists(), "it was stopped half way");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_answer_too_large_for_the_link_becomes_an_error() {
        let small = result_message(3, json!({"ok": true, "data": "x".repeat(1000)}));
        assert_eq!(
            serde_json::from_str::<FromNode>(&small).unwrap(),
            FromNode::RpcResult { id: 3, res: json!({"ok": true, "data": "x".repeat(1000)}) }
        );
        let big = result_message(4, json!({"ok": true, "data": "x".repeat(5 << 20)}));
        assert!(big.len() < 1000, "{}", big.len());
        let FromNode::RpcResult { id, res } = serde_json::from_str::<FromNode>(&big).unwrap() else { panic!() };
        assert_eq!((id, res["ok"].clone()), (4, json!(false)));
        assert_eq!(res["error"], "The answer was too large to send (5.0 MB).");
        // right at the hub's limit still fits under it
        let edge = result_message(5, json!("x".repeat(RESULT_MAX - 64)));
        assert!(edge.len() <= RESULT_MAX && edge.len() > RESULT_MAX - 64, "{}", edge.len());
    }

    #[cfg(unix)]
    fn stalled() -> (Term, pty::TermIo) {
        // a program that reads nothing: after a few lines the PTY takes no more input
        let argv = ["/bin/sh".to_string(), "-c".to_string(), "sleep 30".to_string()];
        pty::spawn(pty::Spawn { argv: &argv, env: &[], env_remove: &[], cwd: None, cols: 80, rows: 24, rate: RATE })
            .unwrap()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_big_paste_into_a_slow_program_does_not_end_the_terminal() {
        let (term, mut io) = stalled();
        let open = Open::start(term, INPUT_WAITING);
        let mut line = vec![b'p'; 1023];
        line.push(b'\n');
        let piece: Vec<u8> = line.iter().copied().cycle().take(pty::INPUT_PIECE).collect();
        // 300 pieces is more than the PTY's own backlog (256), where the terminal used to be hung up
        for i in 0..300 {
            assert!(open.write(&piece), "piece {i} was refused");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let mut ended = false;
        loop {
            match io.output.try_recv() {
                Ok(_) => {}
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    ended = true;
                    break;
                }
            }
        }
        assert!(!ended, "the terminal was hung up");
        drop(open);
        let gone = async { while io.output.recv().await.is_some() {} };
        assert!(tokio::time::timeout(Duration::from_secs(10), gone).await.is_ok(), "dropping it hangs up");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn input_far_past_what_the_program_takes_is_refused() {
        let (term, _io) = stalled();
        let open = Open::start(term, 64 * 1024);
        let mut line = vec![b'p'; 1023];
        line.push(b'\n');
        let piece: Vec<u8> = line.iter().copied().cycle().take(pty::INPUT_PIECE).collect();
        let mut took = 0;
        for _ in 0..600 {
            if !open.write(&piece) {
                break;
            }
            took += 1;
            tokio::task::yield_now().await;
        }
        assert!(took < 600, "nothing bounds what waits for a program that reads nothing");
        assert!(open.waiting.load(Ordering::Relaxed) <= 64 * 1024);
    }

    #[test]
    fn a_large_message_is_given_longer_to_leave_in_step_with_the_patience() {
        assert_eq!(send_patience(0), SEND_PATIENCE);
        assert_eq!(send_patience(31 << 10), SEND_PATIENCE);
        assert_eq!(send_patience(64 << 10), SEND_PATIENCE + SEND_PATIENCE / 10);
        // 3 MB, as the stuck-hub test below sends: still given up on, not waited out
        assert!(send_patience(3_000_000) < SEND_PATIENCE * 6);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_hub_that_stops_reading_ends_the_link_instead_of_holding_it() {
        // A hub that links, asks for a dozen large answers, and then reads nothing more (its machine froze).
        let hub = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = hub.local_addr().unwrap().to_string();
        tokio::spawn(async move {
            let (s, _) = hub.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
            for id in 0..16u64 {
                let m = ToNode::Rpc { id, req: json!({}), timeout: 20 };
                ws.send(Message::Text(link_proto::encode(&m).into())).await.unwrap();
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
            drop(ws);
        });
        let sb = script(
            "flood",
            "cat >/dev/null; printf '{\"ok\": true, \"pad\": \"'; head -c 3000000 /dev/zero | tr '\\0' 'a'; echo '\"}'\n",
        );
        let cfg = lab_cfg(&addr, Some(&sb));
        let began = Instant::now();
        let lock = sb.with_file_name("node.lock");
        let end =
            tokio::time::timeout(Duration::from_secs(30), link_once(&cfg, &lock)).await.expect("the link never ended");
        assert_eq!(end.ok(), Some("the hub stopped taking messages".to_string()));
        assert!(began.elapsed() < Duration::from_secs(25), "{:?}", began.elapsed());
        let _ = std::fs::remove_dir_all(sb.parent().unwrap());
    }

    #[tokio::test]
    async fn a_terminal_that_cannot_start_still_ends_on_a_busy_link() {
        let cfg = lab_cfg("192.0.2.10:8132", None);
        let (tx, mut rx) = mpsc::channel(1);
        tx.try_send(Message::Ping(Vec::new().into())).unwrap(); // the link is full
        let (done, _done_rx) = mpsc::unbounded_channel();
        assert!(open_term(&cfg, 7, "not-a-bg", 80, 24, tx, done).await.is_none());
        let mut exit = None;
        while let Ok(Some(m)) = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
            if let Message::Text(t) = m {
                exit = serde_json::from_str::<FromNode>(t.as_str()).ok();
            }
        }
        assert_eq!(exit, Some(FromNode::TermExit { ch: 7, code: Some(2) }), "the hub is told it ended");
    }
}
