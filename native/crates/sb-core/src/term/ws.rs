//! One open terminal: a WebSocket to the hub on its own thread, so typing and output never wait on the UI or on
//! a refresh. The UI sends `ToHub` and awaits `FromHub` on async channels (any executor can await them).
//!
//! Protocol (spec v2 section 1, and the 2026-10-14 terminal pipeline): client -> hub binary = input bytes, text =
//! {"type":"resize"} (the size this viewer would like), {"type":"focus"} (this viewer sets the size now),
//! {"type":"redraw"} (send the screen again) and {"type":"ping"}; hub -> client binary = output, text = status, size
//! (the terminal's size: resize the grid here, in order with the output), owner, snapshot_begin and snapshot_end
//! (the screen as bytes for a fresh grid), exit, error, pong. Close 1000 after exit, 4401 bad or used ticket, 4403
//! bad origin, 4429 at the terminal limit.
//!
//! Output reaches the UI through a bounded channel: when the UI falls behind, this thread stops reading the socket,
//! and the hub, seeing this viewer fall behind, sends it the screen again once it catches up instead of every byte.

use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

use async_tungstenite::tungstenite::client::IntoClientRequest;
use async_tungstenite::tungstenite::http::HeaderValue;
use async_tungstenite::tungstenite::protocol::CloseFrame;
use async_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use async_tungstenite::tungstenite::{Error as WsError, Message};
use futures_util::StreamExt;
use serde::Deserialize;
use smol::channel::{Receiver, Sender, bounded, unbounded};
use smol::{Async, Timer};

/// How often the client pings, and how long silence may last before the connection counts as lost.
pub const PING_EVERY: Duration = Duration::from_secs(20);
pub const SILENCE: Duration = Duration::from_secs(50);
const DIAL: Duration = Duration::from_secs(8);
/// Events waiting for the UI before this thread stops reading the socket.
pub const UI_BACKLOG: usize = 256;
/// Input goes out in frames of at most this much (the hub takes frames up to 1 MiB).
pub const INPUT_FRAME: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToHub {
    Input(Vec<u8>),
    /// The size this viewer would like. The grid here changes only when the hub's `Size` comes back.
    Resize {
        cols: u16,
        rows: u16,
    },
    /// This viewer has focus: the terminal takes its size.
    Focus,
    /// Send the screen again (a snapshot).
    Redraw,
    /// Detach: close the socket. The session keeps running on its machine.
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FromHub {
    /// The WebSocket is up; the hub's own status follows.
    Connected,
    /// The hub's status: "connecting" (ssh is starting) or "open".
    Status(String),
    Data(Vec<u8>),
    /// The terminal's size from here on: output after this is drawn at it.
    Size {
        cols: u16,
        rows: u16,
    },
    /// Whether this viewer sets the size (it had focus or typed last). Only the one that does answers the program's
    /// questions (cursor position, colors), so it never gets two answers.
    Owner(bool),
    /// The screen follows as output, for a fresh grid, until `SnapshotEnd`.
    SnapshotBegin,
    SnapshotEnd,
    /// `claude attach` ended (the user detached, or the session stopped).
    Exit {
        code: Option<i64>,
        note: Option<String>,
    },
    Error(String),
    /// Always last, exactly once.
    Closed(Closed),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closed {
    /// 1000: a normal close, after exit.
    Normal,
    /// The client closed it (Close).
    ByClient,
    /// 4401: the ticket was bad, used or expired.
    Ticket,
    /// 4403: the hub refused this origin.
    Origin,
    /// 4429: the hub is at its terminal limit.
    Limit,
    /// The connection dropped or went silent.
    Lost(String),
    /// It never connected.
    Failed(String),
}

impl Closed {
    pub fn from_code(code: u16, reason: &str) -> Closed {
        match code {
            1000 => Closed::Normal,
            4401 => Closed::Ticket,
            4403 => Closed::Origin,
            4429 => Closed::Limit,
            _ if reason.is_empty() => Closed::Lost(format!("The connection closed ({code}).")),
            _ => Closed::Lost(reason.to_string()),
        }
    }

    /// A sentence for the Detached card.
    pub fn message(&self) -> String {
        match self {
            Closed::Normal | Closed::ByClient => "Detached. The session keeps running.".into(),
            Closed::Ticket => "The terminal ticket expired or was already used.".into(),
            Closed::Origin => "The terminal hub refused this app.".into(),
            Closed::Limit => "Too many terminals are open. Close one and try again.".into(),
            Closed::Lost(m) | Closed::Failed(m) => m.clone(),
        }
    }
}

/// The two ends the UI holds. Dropping `tx` closes the connection.
pub struct Conn {
    pub tx: Sender<ToHub>,
    pub rx: Receiver<FromHub>,
}

#[derive(Deserialize)]
struct Control {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    code: Option<i64>,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    cols: Option<u16>,
    #[serde(default)]
    rows: Option<u16>,
    #[serde(default)]
    owner: Option<bool>,
}

/// A hub text frame as an event (None for pong and anything unknown).
pub fn parse_control(text: &str) -> Option<FromHub> {
    let c: Control = serde_json::from_str(text).ok()?;
    match c.kind.as_str() {
        "status" => Some(FromHub::Status(c.state.unwrap_or_default())),
        "exit" => Some(FromHub::Exit { code: c.code, note: c.note }),
        "error" => Some(FromHub::Error(c.error.unwrap_or_else(|| "The terminal hub reported an error.".into()))),
        "size" => match (c.cols, c.rows) {
            (Some(cols), Some(rows)) if cols > 0 && rows > 0 => Some(FromHub::Size { cols, rows }),
            _ => None,
        },
        "owner" => Some(FromHub::Owner(c.owner.unwrap_or(false))),
        "snapshot_begin" => Some(FromHub::SnapshotBegin),
        "snapshot_end" => Some(FromHub::SnapshotEnd),
        _ => None,
    }
}

pub fn resize_frame(cols: u16, rows: u16) -> String {
    format!(r#"{{"type":"resize","cols":{cols},"rows":{rows}}}"#)
}

pub const PING_FRAME: &str = r#"{"type":"ping"}"#;
pub const FOCUS_FRAME: &str = r#"{"type":"focus"}"#;
pub const REDRAW_FRAME: &str = r#"{"type":"redraw"}"#;

/// "ws://host:port/path" -> (host, port). Only plain ws: the hub is on the LAN or Tailscale.
pub fn host_port(url: &str) -> Option<(String, u16)> {
    let rest = url.strip_prefix("ws://")?;
    let hostport = rest.split(['/', '?']).next()?;
    if let Some(v6) = hostport.strip_prefix('[') {
        let (h, tail) = v6.split_once(']')?;
        let port = tail.strip_prefix(':').map(|p| p.parse().ok()).unwrap_or(Some(80))?;
        return Some((h.to_string(), port));
    }
    match hostport.rsplit_once(':') {
        Some((h, p)) => Some((h.to_string(), p.parse().ok()?)),
        None => Some((hostport.to_string(), 80)),
    }
}

/// Opens the terminal at `url` (a ticket URL with cols and rows) on a new thread.
pub fn connect(url: String, user_agent: String) -> Conn {
    let (to_tx, to_rx) = unbounded::<ToHub>();
    let (from_tx, from_rx) = bounded::<FromHub>(UI_BACKLOG);
    let spawned = thread::Builder::new().name("sb-term-ws".into()).spawn({
        let from_tx = from_tx.clone();
        move || {
            let mut rec = super::rec::Recorder::from_env("term");
            if let Some(r) = rec.as_mut() {
                // the size the hub was asked for, not the ticket
                let q = url.split_once('?').map(|(_, q)| q).unwrap_or("");
                let keep: Vec<&str> = q.split('&').filter(|p| !p.starts_with("t=")).collect();
                r.frame(super::rec::Kind::Note, format!("connect {}", keep.join("&")).as_bytes());
            }
            let end = smol::block_on(run(&url, &user_agent, to_rx, &from_tx, &mut rec));
            if let Some(r) = rec.as_mut() {
                r.frame(super::rec::Kind::Note, format!("closed {end:?}").as_bytes());
            }
            // always last: wait for room rather than lose it
            let _ = smol::block_on(from_tx.send(FromHub::Closed(end)));
        }
    });
    if let Err(e) = spawned {
        let _ = from_tx.try_send(FromHub::Closed(Closed::Failed(format!("Could not start the terminal: {e}"))));
    }
    Conn { tx: to_tx, rx: from_rx }
}

enum Ev {
    Out(Option<ToHub>),
    In(Option<Result<Message, WsError>>),
    Tick,
}

async fn dial(host: &str, port: u16) -> Result<Async<TcpStream>, String> {
    let unreachable = || format!("Could not reach the terminal hub at {host}:{port}.");
    let addrs: Vec<std::net::SocketAddr> = smol::net::resolve((host, port)).await.map_err(|_| unreachable())?;
    let addr = *addrs.first().ok_or_else(unreachable)?;
    let connect = async { Async::<TcpStream>::connect(addr).await.map_err(|_| unreachable()) };
    let timeout = async {
        Timer::after(DIAL).await;
        Err(unreachable())
    };
    let stream = smol::future::or(connect, timeout).await?;
    let _ = stream.get_ref().set_nodelay(true);
    Ok(stream)
}

async fn run(
    url: &str,
    user_agent: &str,
    to_rx: Receiver<ToHub>,
    from_tx: &Sender<FromHub>,
    rec: &mut Option<super::rec::Recorder>,
) -> Closed {
    use super::rec::Kind;
    let mut note = |k: Kind, b: &[u8]| {
        if let Some(r) = rec.as_mut() {
            r.frame(k, b);
        }
    };
    let Some((host, port)) = host_port(url) else {
        return Closed::Failed("The terminal address is not one this app can open.".into());
    };
    let stream = match dial(&host, port).await {
        Ok(s) => s,
        Err(m) => return Closed::Failed(m),
    };
    let mut req = match url.into_client_request() {
        Ok(r) => r,
        Err(_) => return Closed::Failed("The terminal address is not valid.".into()),
    };
    if let Ok(v) = HeaderValue::from_str(user_agent) {
        req.headers_mut().insert("User-Agent", v);
    }
    let ws = match async_tungstenite::client_async(req, stream).await {
        Ok((ws, _)) => ws,
        Err(WsError::Http(r)) => {
            let status = r.status().as_u16();
            return match status {
                401 => Closed::Ticket,
                403 => Closed::Origin,
                429 => Closed::Limit,
                _ => Closed::Failed(format!("The terminal hub said no ({status}).")),
            };
        }
        Err(_) => return Closed::Failed(format!("The terminal hub at {host}:{port} did not answer.")),
    };
    let _ = from_tx.send(FromHub::Connected).await;
    let (mut sink, mut stream) = ws.split();
    let mut last_in = Instant::now();
    let mut next_ping = Instant::now() + PING_EVERY;
    loop {
        let ev = smol::future::or(
            async { Ev::Out(to_rx.recv().await.ok()) },
            smol::future::or(async { Ev::In(stream.next().await) }, async {
                Timer::at(next_ping).await;
                Ev::Tick
            }),
        )
        .await;
        match ev {
            Ev::Out(None) | Ev::Out(Some(ToHub::Close)) => {
                let frame = CloseFrame { code: CloseCode::Normal, reason: "detach".into() };
                let close = async {
                    let _ = sink.close(Some(frame)).await;
                };
                let wait = async {
                    Timer::after(Duration::from_secs(1)).await;
                };
                smol::future::or(close, wait).await;
                return Closed::ByClient;
            }
            Ev::Out(Some(ToHub::Input(bytes))) => {
                // a big paste goes in pieces; the hub takes each as fast as the machine reads it
                for piece in bytes.chunks(INPUT_FRAME) {
                    note(Kind::Input, piece);
                    if sink.send(Message::binary(piece.to_vec())).await.is_err() {
                        return Closed::Lost("The connection to the terminal hub dropped.".into());
                    }
                }
            }
            Ev::Out(Some(control @ (ToHub::Resize { .. } | ToHub::Focus | ToHub::Redraw))) => {
                let text = match control {
                    ToHub::Resize { cols, rows } => resize_frame(cols, rows),
                    ToHub::Focus => FOCUS_FRAME.to_string(),
                    _ => REDRAW_FRAME.to_string(),
                };
                note(Kind::TextOut, text.as_bytes());
                if sink.send(Message::text(text)).await.is_err() {
                    return Closed::Lost("The connection to the terminal hub dropped.".into());
                }
            }
            Ev::Tick => {
                if last_in.elapsed() > SILENCE {
                    return Closed::Lost("The terminal hub stopped answering.".into());
                }
                next_ping = Instant::now() + PING_EVERY;
                if sink.send(Message::text(PING_FRAME)).await.is_err() {
                    return Closed::Lost("The connection to the terminal hub dropped.".into());
                }
            }
            Ev::In(None) => return Closed::Lost("The connection to the terminal hub dropped.".into()),
            Ev::In(Some(Err(_))) => return Closed::Lost("The connection to the terminal hub dropped.".into()),
            Ev::In(Some(Ok(msg))) => {
                last_in = Instant::now();
                match msg {
                    Message::Binary(b) => {
                        note(Kind::Out, &b);
                        let _ = from_tx.send(FromHub::Data(b.to_vec())).await;
                    }
                    Message::Text(t) => {
                        note(Kind::TextIn, t.as_bytes());
                        if let Some(ev) = parse_control(t.as_str()) {
                            let _ = from_tx.send(ev).await;
                        }
                    }
                    Message::Close(frame) => {
                        return match frame {
                            Some(f) => Closed::from_code(u16::from(f.code), f.reason.as_str()),
                            None => Closed::Normal,
                        };
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn controls() {
        assert_eq!(parse_control(r#"{"type":"status","state":"open"}"#), Some(FromHub::Status("open".into())));
        assert_eq!(
            parse_control(r#"{"type":"exit","code":0,"note":"Detached. The session keeps running."}"#),
            Some(FromHub::Exit { code: Some(0), note: Some("Detached. The session keeps running.".into()) })
        );
        assert_eq!(
            parse_control(r#"{"type":"error","error":"ssh failed"}"#),
            Some(FromHub::Error("ssh failed".into()))
        );
        assert_eq!(parse_control(r#"{"type":"pong"}"#), None);
        assert_eq!(parse_control("nope"), None);
        assert_eq!(resize_frame(120, 36), r#"{"type":"resize","cols":120,"rows":36}"#);
        assert_eq!(
            parse_control(r#"{"type":"size","cols":100,"rows":30}"#),
            Some(FromHub::Size { cols: 100, rows: 30 })
        );
        assert_eq!(parse_control(r#"{"type":"size","cols":0,"rows":30}"#), None);
        assert_eq!(parse_control(r#"{"type":"size"}"#), None);
        assert_eq!(parse_control(r#"{"type":"owner","owner":true}"#), Some(FromHub::Owner(true)));
        assert_eq!(parse_control(r#"{"type":"snapshot_begin"}"#), Some(FromHub::SnapshotBegin));
        assert_eq!(parse_control(r#"{"type":"snapshot_end"}"#), Some(FromHub::SnapshotEnd));
        assert_eq!(Closed::from_code(4429, ""), Closed::Limit);
        assert_eq!(Closed::from_code(4401, "bad ticket"), Closed::Ticket);
        assert_eq!(Closed::from_code(1011, ""), Closed::Lost("The connection closed (1011).".into()));
    }

    #[test]
    fn addresses() {
        assert_eq!(host_port("ws://192.0.2.10:8132/term?t=x"), Some(("192.0.2.10".into(), 8132)));
        assert_eq!(host_port("ws://hub/term"), Some(("hub".into(), 80)));
        assert_eq!(host_port("ws://[fd7a::1]:8132/term"), Some(("fd7a::1".into(), 8132)));
        assert_eq!(host_port("wss://x:1/"), None);
    }

    /// A hub in a thread: sends status, echoes input as output, answers a resize with its size, then exits.
    fn fake_hub(close_code: Option<u16>) -> (String, thread::JoinHandle<Vec<String>>) {
        use async_tungstenite::tungstenite::accept;
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let h = thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut ws = accept(s).unwrap();
            let mut seen = Vec::new();
            if let Some(code) = close_code {
                let _ = ws.close(Some(CloseFrame { code: CloseCode::from(code), reason: "".into() }));
                let _ = ws.flush();
                while ws.read().is_ok() {}
                return seen;
            }
            ws.send(Message::text(r#"{"type":"status","state":"connecting"}"#)).unwrap();
            ws.send(Message::text(r#"{"type":"status","state":"open"}"#)).unwrap();
            loop {
                match ws.read() {
                    Ok(Message::Binary(b)) => {
                        let s = String::from_utf8_lossy(&b).to_string();
                        seen.push(format!("in:{s}"));
                        ws.send(Message::binary(format!("echo:{s}").into_bytes())).unwrap();
                        if s == "q" {
                            ws.send(Message::text(
                                r#"{"type":"exit","code":0,"note":"Detached. The session keeps running."}"#,
                            ))
                            .unwrap();
                            ws.close(Some(CloseFrame { code: CloseCode::Normal, reason: "".into() })).unwrap();
                        }
                    }
                    Ok(Message::Text(t)) => seen.push(format!("text:{t}")),
                    Ok(Message::Close(_)) => {
                        let _ = ws.flush();
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            seen
        });
        (format!("ws://127.0.0.1:{port}/term?t=abc&cols=80&rows=24"), h)
    }

    fn collect(rx: &Receiver<FromHub>) -> Vec<FromHub> {
        let mut out = Vec::new();
        loop {
            match smol::block_on(async {
                smol::future::or(async { rx.recv().await.ok() }, async {
                    Timer::after(Duration::from_secs(5)).await;
                    None
                })
                .await
            }) {
                Some(ev) => {
                    let end = matches!(ev, FromHub::Closed(_));
                    out.push(ev);
                    if end {
                        return out;
                    }
                }
                None => return out,
            }
        }
    }

    #[test]
    fn talks_the_protocol() {
        let (url, hub) = fake_hub(None);
        let c = connect(url, "SwitchboardDesktop/test".into());
        c.tx.try_send(ToHub::Resize { cols: 100, rows: 30 }).unwrap();
        c.tx.try_send(ToHub::Focus).unwrap();
        c.tx.try_send(ToHub::Redraw).unwrap();
        c.tx.try_send(ToHub::Input(b"ls\r".to_vec())).unwrap();
        c.tx.try_send(ToHub::Input(b"q".to_vec())).unwrap();
        let evs = collect(&c.rx);
        assert_eq!(evs.first(), Some(&FromHub::Connected));
        assert!(evs.contains(&FromHub::Status("open".into())), "{evs:?}");
        assert!(evs.contains(&FromHub::Data(b"echo:ls\r".to_vec())), "{evs:?}");
        assert!(
            evs.contains(&FromHub::Exit { code: Some(0), note: Some("Detached. The session keeps running.".into()) }),
            "{evs:?}"
        );
        assert_eq!(evs.last(), Some(&FromHub::Closed(Closed::Normal)));
        let seen = hub.join().unwrap();
        assert_eq!(seen[0], r#"text:{"type":"resize","cols":100,"rows":30}"#);
        assert_eq!(seen[1], r#"text:{"type":"focus"}"#);
        assert_eq!(seen[2], r#"text:{"type":"redraw"}"#);
        assert_eq!(seen[3], "in:ls\r");
    }

    #[test]
    fn a_big_paste_goes_in_pieces_and_a_slow_ui_holds_the_socket() {
        let (url, hub) = fake_hub(None);
        let c = connect(url, "t".into());
        let paste = vec![b'p'; INPUT_FRAME * 2 + 10];
        c.tx.try_send(ToHub::Input(paste)).unwrap();
        c.tx.try_send(ToHub::Input(b"q".to_vec())).unwrap();
        // nobody reads for a while: the channel fills to its bound and the thread waits, losing nothing
        thread::sleep(Duration::from_millis(300));
        assert!(c.rx.len() <= UI_BACKLOG);
        let evs = collect(&c.rx);
        assert_eq!(evs.last(), Some(&FromHub::Closed(Closed::Normal)), "{evs:?}");
        let seen = hub.join().unwrap();
        let ins: Vec<usize> = seen.iter().filter(|s| s.starts_with("in:p")).map(|s| s.len() - 3).collect();
        assert_eq!(ins, vec![INPUT_FRAME, INPUT_FRAME, 10]);
    }

    #[test]
    fn close_codes_and_refusals() {
        let (url, hub) = fake_hub(Some(4429));
        let c = connect(url, "t".into());
        let evs = collect(&c.rx);
        assert_eq!(evs.last(), Some(&FromHub::Closed(Closed::Limit)), "{evs:?}");
        hub.join().unwrap();

        // Nothing listening: a clear failure, fast.
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let c = connect(format!("ws://127.0.0.1:{port}/term?t=x"), "t".into());
        let evs = collect(&c.rx);
        assert_eq!(
            evs,
            vec![FromHub::Closed(Closed::Failed(format!("Could not reach the terminal hub at 127.0.0.1:{port}.")))]
        );
    }

    #[test]
    fn detach_closes_cleanly() {
        let (url, hub) = fake_hub(None);
        let c = connect(url, "t".into());
        c.tx.try_send(ToHub::Close).unwrap();
        let evs = collect(&c.rx);
        assert_eq!(evs.last(), Some(&FromHub::Closed(Closed::ByClient)), "{evs:?}");
        hub.join().unwrap();
    }
}
