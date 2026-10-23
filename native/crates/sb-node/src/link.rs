//! The link: one WebSocket to the hub's `/node`, kept up with backoff. The hub asks for RPCs (answered by running
//! `sb.py rpc`) and terminals (a PTY running sb.py's term mode, which execs `claude attach <bg>`, exactly what the
//! hub's ssh gets). Closing a terminal only detaches; the session keeps running.

use crate::{NodeConfig, VERSION};
use futures_util::{SinkExt, StreamExt};
use sb_hub::link_proto::{self, CLOSE_REPLACED, CLOSE_UNPAIRED, FromNode, ToNode};
use sb_hub::pty::{self, Term};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{self, Message};

const PING_EVERY: Duration = Duration::from_secs(20);
const RATE: u64 = 2 * 1024 * 1024;
/// Messages waiting to go out to the hub; past it, terminal output waits (the PTY pushes back on the process).
const LINK_BACKLOG: usize = 256;

fn log(msg: impl AsRef<str>) {
    eprintln!("switchboard-node: {}", msg.as_ref());
}

enum Stop {
    /// The hub no longer knows this token (unpaired, or paired again): reconnecting cannot help.
    Unpaired(String),
    /// Another node with this token took the link.
    Replaced,
}

/// `run`'s answer when the hub no longer knows the token.
pub const UNPAIRED: i32 = 3;
/// ... and when another node with the same token took the link.
pub const REPLACED: i32 = 4;

/// Keep the link up until the token stops working or a signal arrives. Returns the exit code.
pub async fn run(cfg: NodeConfig) -> i32 {
    log(format!("{VERSION} linking {} to the hub at {}", cfg.machine, cfg.hub));
    let forever = async {
        let mut wait = 1u64;
        loop {
            let began = Instant::now();
            match link_once(&cfg).await {
                Ok(why) => log(format!("link closed: {why}")),
                Err(Ok(Stop::Unpaired(why))) => {
                    log(format!("{why} Pair it again: switchboard-node pair {} <code>", cfg.hub));
                    return UNPAIRED;
                }
                Err(Ok(Stop::Replaced)) => {
                    log("another switchboard-node for this machine took over the link; this one waits.");
                    return REPLACED;
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
            0
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

/// One link, from connect to close. Ok(why) means try again; Err(Ok) means stop; Err(Err) is a failed connect.
async fn link_once(cfg: &NodeConfig) -> Result<String, Result<Stop, String>> {
    let mut req = format!("ws://{}/node", cfg.hub).into_client_request().map_err(|e| Err(e.to_string()))?;
    let auth =
        format!("Bearer {}", cfg.token).parse().map_err(|_| Err("the saved token is not valid text".to_string()))?;
    req.headers_mut().insert("Authorization", auth);
    let ws = match tokio::time::timeout(Duration::from_secs(15), tokio_tungstenite::connect_async(req)).await {
        Err(_) => return Err(Err("timed out".into())),
        Ok(Err(tungstenite::Error::Http(r))) if r.status() == 401 => {
            return Err(Ok(Stop::Unpaired("The hub does not know this machine's token any more.".into())));
        }
        Ok(Err(tungstenite::Error::Http(r))) => return Err(Err(format!("the hub answered {}", r.status()))),
        Ok(Err(e)) => return Err(Err(e.to_string())),
        Ok(Ok((ws, _))) => ws,
    };
    log("linked");
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::channel::<Message>(LINK_BACKLOG); // bounded: output waits for the hub, never piles up
    let (done_tx, mut done_rx) = mpsc::unbounded_channel::<u32>();
    let label = crate::hostname_label();
    let _ = tx
        .send(Message::Text(
            link_proto::encode(&FromNode::Hello { version: VERSION.into(), os: crate::os_name().into(), label }).into(),
        ))
        .await;
    let writer = tokio::spawn(async move {
        while let Some(m) = rx.recv().await {
            if sink.send(m).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    let mut terms: HashMap<u32, Term> = HashMap::new();
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
                            tokio::spawn(async move {
                                let res = rpc(&cfg, &req, timeout).await;
                                let _ = tx.send(Message::Text(link_proto::encode(&FromNode::RpcResult { id, res }).into())).await;
                            });
                        }
                        Ok(ToNode::TermOpen { ch, bg, cols, rows }) => {
                            if let Some(t) = open_term(cfg, ch, &bg, cols, rows, tx.clone(), done_tx.clone()).await {
                                terms.insert(ch, t);
                            }
                        }
                        Ok(ToNode::TermResize { ch, cols, rows }) => {
                            if let Some(t) = terms.get(&ch) {
                                let (c, r) = sb_hub::proto::clamp_size(cols.into(), rows.into());
                                t.resize(c, r);
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
                        {
                            t.write(data.to_vec());
                        }
                    }
                    Some(Ok(Message::Close(f))) => {
                        let (code, why) = f.map(|f| (u16::from(f.code), f.reason.to_string())).unwrap_or((1005, String::new()));
                        if code == CLOSE_UNPAIRED {
                            break Err(Ok(Stop::Unpaired(if why.is_empty() { "The hub unlinked this machine.".into() } else { why })));
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
            _ = ping.tick() => {
                if heard.elapsed() > PING_EVERY * 3 {
                    break Ok("the hub went quiet".into());
                }
                let _ = tx.try_send(Message::Ping(Vec::new().into()));
            }
        }
    };
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
    let _ = tokio::time::timeout(Duration::from_secs(2), writer).await;
    end
}

/// `sb.py rpc` with the request on stdin; its JSON answer (the last line it prints), or why there is none.
async fn rpc(cfg: &NodeConfig, req: &Value, timeout: u64) -> Value {
    let mut cmd = tokio::process::Command::new(cfg.python());
    cmd.arg(cfg.sb()).arg("rpc").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    cmd.env_remove("SSH_ORIGINAL_COMMAND").current_dir(crate::home());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: never flash a console on the desktop
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return json!({"ok": false, "error": format!("could not run sb.py ({e})")}),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(req.to_string().as_bytes()).await;
    }
    let out = match tokio::time::timeout(Duration::from_secs(timeout.max(1)), child.wait_with_output()).await {
        Err(_) => return json!({"ok": false, "error": format!("sb.py did not answer in {timeout} s")}),
        Ok(Err(e)) => return json!({"ok": false, "error": format!("sb.py failed ({e})")}),
        Ok(Ok(o)) => o,
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    match stdout.lines().rev().find(|l| !l.trim().is_empty()).and_then(|l| serde_json::from_str::<Value>(l).ok()) {
        Some(v) => v,
        None => {
            let err = String::from_utf8_lossy(&out.stderr);
            let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
            let why = if last.is_empty() {
                format!("sb.py exited {} with no answer", out.status.code().unwrap_or(-1))
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

    #[tokio::test]
    async fn a_terminal_that_cannot_start_still_ends_on_a_busy_link() {
        let cfg = NodeConfig {
            hub: "192.0.2.10:8132".into(),
            machine: "lab".into(),
            token: "t".into(),
            sb: None,
            python: None,
        };
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
