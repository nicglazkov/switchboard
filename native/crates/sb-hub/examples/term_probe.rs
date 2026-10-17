//! An end-to-end check of shared terminals against a running test hub (spec 2026-10-14, terminal pipeline): two
//! viewers on one chat, sizes changing and focus moving while the program floods, every frame recorded in the
//! recorder's format, then a redraw on each. Replay the captures with sb-core's term_replay --check: every snapshot
//! must match the screen rebuilt from the stream.
//!
//!     cargo run -p sb-hub --example term_probe -- --hub 127.0.0.1:18232 --key <term.key> --out <dir> [--machine lab]
//!
//! Point the hub at native/scripts/term-fixture.py (SWITCHBOARD_HUB_SSH) and never at a real machine.

use futures_util::{SinkExt, StreamExt};
use sb_core::term::rec::{Kind, Recorder};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

enum Cmd {
    Text(String),
    Bin(Vec<u8>),
    Close,
}

struct Viewer {
    tx: mpsc::UnboundedSender<Cmd>,
    done: tokio::task::JoinHandle<(usize, usize, Vec<String>)>,
}

/// Connects one viewer and records everything both ways until it closes. Returns (output bytes, snapshots, sizes).
#[allow(clippy::too_many_arguments)] // A probe tool: one argument per connection setting.
async fn viewer(
    hub: &str,
    key: &[u8],
    machine: &str,
    bg: &str,
    cols: u16,
    rows: u16,
    out: &std::path::Path,
    label: &str,
) -> Viewer {
    let t = sb_hub::ticket::mint(key, machine, bg, sb_hub::server::unix_now(), sb_hub::ticket::TTL);
    let url = format!("ws://{hub}/term?t={t}&cols={cols}&rows={rows}&proto=2");
    let (ws, _) = tokio_tungstenite::connect_async(url).await.expect("the test hub answers");
    let mut rec = Recorder::create(out, label).expect("a capture file");
    rec.frame(Kind::Note, format!("connect cols={cols}&rows={rows}&proto=2").as_bytes());
    eprintln!("{label}: recording to {}", rec.path.display());
    let (mut sink, mut stream) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Cmd>();
    let done = tokio::spawn(async move {
        let (mut bytes, mut snaps, mut sizes) = (0usize, 0usize, Vec::new());
        loop {
            tokio::select! {
                cmd = rx.recv() => match cmd {
                    Some(Cmd::Text(t)) => {
                        rec.frame(Kind::TextOut, t.as_bytes());
                        let _ = sink.send(Message::Text(t.into())).await;
                    }
                    Some(Cmd::Bin(b)) => {
                        rec.frame(Kind::Input, &b);
                        let _ = sink.send(Message::Binary(b.into())).await;
                    }
                    Some(Cmd::Close) | None => {
                        let _ = sink.send(Message::Close(None)).await;
                        break;
                    }
                },
                msg = stream.next() => match msg {
                    Some(Ok(Message::Binary(b))) => {
                        bytes += b.len();
                        rec.frame(Kind::Out, &b);
                    }
                    Some(Ok(Message::Text(t))) => {
                        rec.frame(Kind::TextIn, t.as_bytes());
                        let v: serde_json::Value = serde_json::from_str(t.as_str()).unwrap_or_default();
                        match v["type"].as_str() {
                            Some("snapshot_end") => snaps += 1,
                            Some("size") => sizes.push(format!("{}x{}", v["cols"], v["rows"])),
                            _ => {}
                        }
                    }
                    Some(Ok(_)) => {}
                    _ => break,
                },
            }
        }
        rec.frame(Kind::Note, b"closed");
        (bytes, snaps, sizes)
    });
    Viewer { tx, done }
}

fn text(v: &Viewer, t: &str) {
    let _ = v.tx.send(Cmd::Text(t.to_string()));
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(hub), Some(key), Some(out)) = (arg(&args, "--hub"), arg(&args, "--key"), arg(&args, "--out")) else {
        eprintln!("usage: term_probe --hub HOST:PORT --key <term.key> --out <dir> [--machine lab] [--bg c0ffee01]");
        std::process::exit(2);
    };
    let machine = arg(&args, "--machine").unwrap_or_else(|| "lab".into());
    let bg = arg(&args, "--bg").unwrap_or_else(|| "c0ffee01".into());
    let key = std::fs::read(&key).expect("the hub's key");
    let out = std::path::PathBuf::from(out);

    // A opens it, the program starts flooding; B joins a moment later at another size
    let a = viewer(&hub, &key, &machine, &bg, 100, 30, &out, "a").await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let b = viewer(&hub, &key, &machine, &bg, 80, 24, &out, "b").await;
    // C: a client that knows nothing of size frames (an old app), only reads
    let c = {
        let t = sb_hub::ticket::mint(&key, &machine, &bg, sb_hub::server::unix_now(), sb_hub::ticket::TTL);
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{hub}/term?t={t}&cols=90&rows=20")).await.unwrap();
        ws
    };

    // while it floods: sizes change and focus moves between A and B
    let start = Instant::now();
    let sizes_a = [(100, 30), (120, 40), (72, 22), (132, 36), (100, 30)];
    let sizes_b = [(80, 24), (90, 28), (60, 18), (110, 33)];
    let mut i = 0usize;
    while start.elapsed() < Duration::from_secs(5) {
        let (ac, ar) = sizes_a[i % sizes_a.len()];
        let (bc, br) = sizes_b[i % sizes_b.len()];
        text(&a, &format!(r#"{{"type":"resize","cols":{ac},"rows":{ar}}}"#));
        text(&b, &format!(r#"{{"type":"resize","cols":{bc},"rows":{br}}}"#));
        if i % 7 == 5 {
            // a redraw in the middle of the flood: the snapshot must match the stream at that exact point
            text(&a, r#"{"type":"redraw"}"#);
            text(&b, r#"{"type":"redraw"}"#);
        }
        if i % 3 == 1 {
            text(&b, r#"{"type":"focus"}"#);
        } else if i % 3 == 2 {
            text(&a, r#"{"type":"focus"}"#);
        }
        i += 1;
        tokio::time::sleep(Duration::from_millis(230)).await;
    }
    // B types (it owns the size now), the program floods again, then both ask for the screen once it is quiet
    let _ = b.tx.send(Cmd::Bin(b"f".to_vec()));
    tokio::time::sleep(Duration::from_secs(7)).await;
    text(&a, r#"{"type":"redraw"}"#);
    text(&b, r#"{"type":"redraw"}"#);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    drop(c);
    let _ = a.tx.send(Cmd::Close);
    let _ = b.tx.send(Cmd::Close);
    let (ab, asnaps, asizes) = a.done.await.unwrap();
    let (bb, bsnaps, bsizes) = b.done.await.unwrap();
    println!("a: {} KB out, {asnaps} snapshots, sizes {}", ab / 1024, asizes.join(" "));
    println!("b: {} KB out, {bsnaps} snapshots, sizes {}", bb / 1024, bsizes.join(" "));
    if asnaps < 4 || bsnaps < 5 {
        eprintln!("expected four redraw snapshots on a, and a join plus four redraws on b");
        std::process::exit(1);
    }
}
