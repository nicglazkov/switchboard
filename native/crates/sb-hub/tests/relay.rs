//! The hub end to end on this machine, with a fake ssh (tests/fake-ssh.sh) standing in for the real one.
//! The fake is a shell script, which Windows cannot start as a program, so these run on Unix only.
#![cfg(unix)]

use futures_util::{SinkExt, StreamExt};
use sb_hub::config::Config;
use sb_hub::server::{self, Hub};
use sb_hub::ticket;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Lab {
    addr: std::net::SocketAddr,
    key: Vec<u8>,
    dir: PathBuf,
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn now() -> u64 {
    server::unix_now()
}

async fn lab(tweak: impl FnOnce(&mut Config)) -> Lab {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "sbhub-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let machines = dir.join("machines.json");
    std::fs::write(
        &machines,
        r#"{"machines":[{"name":"lab","label":"Lab","target":"me@lab","os":"linux"}],
        "origins":["http://10.0.0.5:8130","http://100.64.0.5:8130","http://10.0.0.5:8150"]}"#,
    )
    .unwrap();
    let mut cfg = Config::defaults_in(&dir);
    cfg.listen = "127.0.0.1:0".parse().unwrap();
    cfg.machines_path = machines;
    cfg.key_path = dir.join("cfg/term.key");
    cfg.ssh = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-ssh.sh").into();
    tweak(&mut cfg);
    let hub = Hub::new(cfg.clone()).unwrap();
    let key = std::fs::read(&cfg.key_path).unwrap();
    let listener = TcpListener::bind(cfg.listen).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(server::serve(hub, listener));
    Lab { addr, key, dir }
}

/// `lab`, with the hub's tunnel listener (server.rs `public_router`) in place of the main one.
async fn tunnel_lab() -> Lab {
    let mut l = lab(|_| {}).await;
    let mut cfg = Config::defaults_in(&l.dir);
    cfg.machines_path = l.dir.join("machines.json");
    cfg.key_path = l.dir.join("cfg/term.key");
    cfg.ssh = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-ssh.sh").into();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.addr = listener.local_addr().unwrap();
    tokio::spawn(server::serve_public(Hub::new(cfg).unwrap(), listener));
    l
}

impl Lab {
    fn ticket(&self, bg: &str) -> String {
        ticket::mint(&self.key, "lab", bg, now(), ticket::TTL)
    }

    async fn connect(&self, t: &str, origin: Option<&str>, size: &str) -> Ws {
        let mut req = format!("ws://{}/term?t={t}{size}", self.addr).into_client_request().unwrap();
        if let Some(o) = origin {
            req.headers_mut().insert("Origin", o.parse().unwrap());
        }
        tokio_tungstenite::connect_async(req).await.unwrap().0
    }

    async fn get(&self, path: &str) -> serde_json::Value {
        let mut s = tokio::net::TcpStream::connect(self.addr).await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        let body = out.split("\r\n\r\n").nth(1).unwrap_or("");
        serde_json::from_str(body).unwrap_or(serde_json::Value::Null)
    }
}

/// Everything the hub sends, until the socket closes (or the wait runs out).
#[derive(Default, Debug)]
struct Seen {
    texts: Vec<serde_json::Value>,
    out: String,
    close: Option<u16>,
}

async fn read_until(ws: &mut Ws, seen: &mut Seen, wait: Duration, done: impl Fn(&Seen) -> bool) {
    let end = Instant::now() + wait;
    while !done(seen) {
        let left = end.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => seen.texts.push(serde_json::from_str(t.as_str()).unwrap()),
            Ok(Some(Ok(Message::Binary(b)))) => seen.out.push_str(&String::from_utf8_lossy(&b)),
            Ok(Some(Ok(Message::Close(f)))) => {
                seen.close = Some(f.map(|f| u16::from(f.code)).unwrap_or(1005));
            }
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_))) | Ok(None) => {
                if seen.close.is_none() {
                    seen.close = Some(1006);
                }
                return;
            }
            Err(_) => return,
        }
    }
}

fn has_type(s: &Seen, ty: &str) -> bool {
    s.texts.iter().any(|t| t["type"] == ty)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_from_open_to_exit() {
    let lab = lab(|_| {}).await;
    let t = lab.ticket("c03ef0d9");
    let mut ws = lab.connect(&t, Some("http://10.0.0.5:8130"), "&cols=100&rows=30").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.out.contains("TERM ")).await;
    assert_eq!(s.texts[0], serde_json::json!({"type": "status", "state": "connecting"}));
    assert_eq!(s.texts[1], serde_json::json!({"type": "size", "cols": 100, "rows": 30}));
    assert_eq!(s.texts[2], serde_json::json!({"type": "owner", "owner": true}));
    assert_eq!(s.texts[3], serde_json::json!({"type": "status", "state": "open"}));
    assert!(s.out.contains("-tt -e none -i "), "{}", s.out);
    assert!(s.out.contains("-o BatchMode=yes"), "{}", s.out);
    assert!(s.out.contains(" me@lab term c03ef0d9"), "{}", s.out);
    assert!(s.out.contains("SIZE 30 100"), "{}", s.out);
    assert!(s.out.contains("TERM xterm-256color"), "{}", s.out);

    // the reaper sees it while it is open
    let open = lab.get("/internal/open").await;
    assert_eq!(open["open"][0]["machine"], "lab");
    assert_eq!(open["open"][0]["bg"], "c03ef0d9");
    assert_eq!(lab.get("/healthz").await["open"], 1);

    ws.send(Message::Text(r#"{"type":"resize","cols":80,"rows":24}"#.into())).await.unwrap();
    ws.send(Message::Binary(b"size\n".to_vec().into())).await.unwrap();
    read_until(&mut ws, &mut s, Duration::from_secs(5), |s| s.out.contains("SIZE 24 80")).await;
    assert!(s.out.contains("SIZE 24 80"));
    assert!(
        s.texts.contains(&serde_json::json!({"type": "size", "cols": 80, "rows": 24})),
        "the size comes back as a frame"
    );

    ws.send(Message::Text(r#"{"type":"ping"}"#.into())).await.unwrap();
    ws.send(Message::Text(r#"{"type":"whatever"}"#.into())).await.unwrap(); // ignored
    ws.send(Message::Binary("hello h\u{e9}llo\n".as_bytes().to_vec().into())).await.unwrap();
    read_until(&mut ws, &mut s, Duration::from_secs(5), |s| s.out.contains("GOT hello") && has_type(s, "pong")).await;
    assert!(s.out.contains("GOT hello h\u{e9}llo"), "{}", s.out);
    assert!(has_type(&s, "pong"));

    ws.send(Message::Binary(b"exit\n".to_vec().into())).await.unwrap();
    read_until(&mut ws, &mut s, Duration::from_secs(5), |s| s.close.is_some()).await;
    let exit = s.texts.iter().find(|t| t["type"] == "exit").expect("exit frame");
    assert_eq!(exit["code"], 0);
    assert_eq!(exit["note"], "Detached. The chat keeps running.");
    assert_eq!(s.close, Some(1000));

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(lab.get("/internal/open").await["open"], serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn tickets_are_single_use_and_signed() {
    let lab = lab(|_| {}).await;
    let t = lab.ticket("c03ef0d9");
    let mut first = lab.connect(&t, None, "").await;
    let mut s = Seen::default();
    read_until(&mut first, &mut s, Duration::from_secs(10), |s| has_type(s, "status") && s.texts.len() >= 2).await;

    for bad in [
        t.clone(),
        "nonsense".into(),
        ticket::mint(&[1u8; 32], "lab", "c03ef0d9", now(), 60),
        ticket::mint(&lab.key, "lab", "c03ef0d9", now() - 120, 60),
    ] {
        let mut ws = lab.connect(&bad, None, "").await;
        let mut s = Seen::default();
        read_until(&mut ws, &mut s, Duration::from_secs(5), |s| s.close.is_some()).await;
        assert_eq!(s.close, Some(4401), "{bad}");
        assert!(has_type(&s, "error"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn other_web_pages_are_refused() {
    let lab = lab(|c| c.origins = vec!["http://localhost:8151".into()]).await;
    for (origin, want) in [
        ("http://evil.example", Some(4403)),
        ("null", Some(4403)),
        ("http://10.0.0.5:8150", None),
        ("http://localhost:8151", None),
        ("http://100.64.0.5:8130", None),
        ("http://10.0.0.9:8130", Some(4403)),
    ] {
        let t = lab.ticket("c03ef0d9");
        let mut ws = lab.connect(&t, Some(origin), "").await;
        let mut s = Seen::default();
        read_until(&mut ws, &mut s, Duration::from_secs(5), |s| s.close.is_some() || s.out.contains("PID")).await;
        assert_eq!(s.close, want, "{origin}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn at_most_max_terminals() {
    let lab = lab(|c| c.max_terms = 1).await;
    let mut a = lab.connect(&lab.ticket("c03ef0d9"), None, "").await;
    let mut s = Seen::default();
    read_until(&mut a, &mut s, Duration::from_secs(10), |s| s.out.contains("PID")).await;
    let t2 = lab.ticket("b52d78ef");
    let mut b = lab.connect(&t2, None, "").await;
    let mut s2 = Seen::default();
    read_until(&mut b, &mut s2, Duration::from_secs(5), |s| s.close.is_some()).await;
    assert_eq!(s2.close, Some(4429));
    assert!(s2.texts[0]["error"].as_str().unwrap().contains("1 terminals are open"));
    // closing the first frees the place, and the refused ticket was not spent (security M6)
    drop(a);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut c = lab.connect(&t2, None, "").await;
    let mut s3 = Seen::default();
    read_until(&mut c, &mut s3, Duration::from_secs(10), |s| s.out.contains("PID") || s.close.is_some()).await;
    assert!(s3.out.contains("PID"), "{s3:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_terminal_limit_counts_chats_not_the_devices_looking_at_them() {
    let lab = lab(|c| {
        c.max_terms = 1;
        c.max_viewers = 3;
    })
    .await;
    let opened = |s: &Seen| has_type(s, "snapshot_end") || s.close.is_some();
    // one chat, shown on three devices: one terminal, so the limit of one is not in the way
    let mut views = vec![];
    for _ in 0..3 {
        let mut v = lab.connect(&lab.ticket("c03ef0d9"), None, "&proto=2").await;
        let mut s = Seen::default();
        read_until(&mut v, &mut s, Duration::from_secs(10), opened).await;
        assert_eq!(s.close, None, "{s:?}");
        views.push(v);
    }
    let h = lab.get("/healthz").await;
    assert_eq!((h["open"].as_u64(), h["viewers"].as_u64(), h["max_viewers"].as_u64()), (Some(1), Some(3), Some(3)));
    // a fourth view is past the viewer limit, and says so
    let t4 = lab.ticket("c03ef0d9");
    let mut v = lab.connect(&t4, None, "&proto=2").await;
    let mut s = Seen::default();
    read_until(&mut v, &mut s, Duration::from_secs(5), |s| s.close.is_some()).await;
    assert_eq!(s.close, Some(4429));
    assert!(s.texts[0]["error"].as_str().unwrap().contains("3 views of terminals are open"), "{s:?}");
    // with a view to spare, another chat is still a second terminal
    drop(views.pop());
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut v = lab.connect(&lab.ticket("b52d78ef"), None, "&proto=2").await;
    let mut s = Seen::default();
    read_until(&mut v, &mut s, Duration::from_secs(5), |s| s.close.is_some()).await;
    assert_eq!(s.close, Some(4429));
    assert!(s.texts[0]["error"].as_str().unwrap().contains("1 terminals are open"), "{s:?}");
    // ... and the refused ticket for the open chat was not spent
    let mut v = lab.connect(&t4, None, "&proto=2").await;
    let mut s = Seen::default();
    read_until(&mut v, &mut s, Duration::from_secs(10), opened).await;
    assert_eq!(s.close, None, "{s:?}");
}

/// Running and not a zombie. `ps` rather than /proc, so the test also runs on macOS.
fn alive(pid: &str) -> bool {
    let out = std::process::Command::new("ps").args(["-o", "stat=", "-p", pid]).output();
    let stat = out.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    !stat.is_empty() && !stat.starts_with('Z')
}

#[tokio::test(flavor = "multi_thread")]
async fn leaving_hangs_up_the_process() {
    let lab = lab(|_| {}).await;
    let mut ws = lab.connect(&lab.ticket("c03ef0d9"), None, "").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.out.contains("PID ") && s.out.contains("\n")).await;
    read_until(&mut ws, &mut s, Duration::from_millis(300), |_| false).await;
    let pid = s.out.split("PID ").nth(1).unwrap().split_whitespace().next().unwrap().to_string();
    assert!(alive(&pid));
    ws.close(None).await.unwrap();
    drop(ws);
    let end = Instant::now() + Duration::from_secs(5);
    while alive(&pid) && Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!alive(&pid), "the fake ssh is still running");
    assert_eq!(lab.get("/internal/open").await["open"], serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_terminals_close() {
    let lab = lab(|c| c.idle = Duration::from_secs(1)).await;
    let mut ws = lab.connect(&lab.ticket("c03ef0d9"), None, "").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.close.is_some()).await;
    let exit = s.texts.iter().find(|t| t["type"] == "exit").expect("exit frame");
    assert_eq!(exit["code"], serde_json::Value::Null);
    assert!(exit["note"].as_str().unwrap().starts_with("Closed after 1 second with"), "{exit}");
    assert_eq!(s.close, Some(1000));
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_failures_say_so() {
    let lab = lab(|_| {}).await;
    let mut ws = lab.connect(&lab.ticket("c03ef0d9"), None, "").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.out.contains("PID")).await;
    ws.send(Message::Binary(b"exit 255\n".to_vec().into())).await.unwrap();
    read_until(&mut ws, &mut s, Duration::from_secs(5), |s| s.close.is_some()).await;
    let exit = s.texts.iter().find(|t| t["type"] == "exit").expect("exit frame");
    assert_eq!(exit["code"], 255);
    assert!(exit["note"].as_str().unwrap().starts_with("Couldn't reach Lab"));
}

#[tokio::test(flavor = "multi_thread")]
async fn output_is_paced() {
    let lab = lab(|c| c.rate = 1_000_000).await;
    let mut ws = lab.connect(&lab.ticket("c03ef0d9"), None, "").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.out.contains("PID")).await;
    let t0 = Instant::now();
    ws.send(Message::Binary(b"flood 1500000\n".to_vec().into())).await.unwrap();
    read_until(&mut ws, &mut s, Duration::from_secs(20), |s| s.out.contains("FLOOD-DONE")).await;
    let took = t0.elapsed();
    assert!(s.out.matches('x').count() >= 1_500_000);
    // 1.5 MB at 1 MB/s with a 250 KB burst: at least about 1.2 s
    assert!(took >= Duration::from_millis(1100), "took {took:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn internal_open_is_local_only_and_healthz_is_public() {
    let lab = lab(|_| {}).await;
    let h = lab.get("/healthz").await;
    assert_eq!(h["ok"], true);
    assert_eq!(h["max"], 16);
    // from 127.0.0.1 it answers; the remote case is covered by the is_loopback check (no second interface here)
    assert_eq!(lab.get("/internal/open").await["ok"], true);
    let _ = CloseCode::Normal;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tunnel_listener_opens_terminals_and_has_no_internal_open() {
    // What a tunnel forwards arrives from 127.0.0.1, which is all /internal/* asks of a caller on the main
    // listener. So here it is not found at all, and the rest works as it does there.
    let lab = tunnel_lab().await;
    assert_eq!(lab.get("/healthz").await["ok"], true);
    assert_eq!(lab.get("/internal/open").await, serde_json::json!({"ok": false, "error": "not found"}));
    let t = lab.ticket("c03ef0d9");
    let mut ws = lab.connect(&t, Some("http://10.0.0.5:8130"), "&cols=100&rows=30").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.out.contains("TERM ")).await;
    assert!(s.out.contains(" me@lab term c03ef0d9"), "{}", s.out);
    ws.send(Message::Binary(b"exit\n".to_vec().into())).await.unwrap();
    read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.close.is_some()).await;
    assert_eq!(s.close, Some(1000));
    // a used ticket and a stranger's page are refused there as on the main listener
    let mut ws = lab.connect(&t, Some("http://10.0.0.5:8130"), "").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(5), |s| s.close.is_some()).await;
    assert_eq!(s.close, Some(sb_hub::proto::CLOSE_TICKET));
    let mut ws = lab.connect(&lab.ticket("c03ef0d9"), Some("https://evil.example"), "").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(5), |s| s.close.is_some()).await;
    assert_eq!(s.close, Some(sb_hub::proto::CLOSE_ORIGIN));
}

fn pid_of(out: &str) -> String {
    out.split("PID ").nth(1).unwrap().split_whitespace().next().unwrap().to_string()
}

/// Texts of one type, in order.
fn of_type<'a>(s: &'a Seen, ty: &str) -> Vec<&'a serde_json::Value> {
    s.texts.iter().filter(|t| t["type"] == ty).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn viewers_share_one_terminal_and_the_focused_one_sets_its_size() {
    let lab = lab(|_| {}).await;
    let mut a = lab.connect(&lab.ticket("c03ef0d9"), None, "&cols=100&rows=30&proto=2").await;
    let mut sa = Seen::default();
    read_until(&mut a, &mut sa, Duration::from_secs(10), |s| s.out.contains("PID ") && s.out.contains("TERM ")).await;
    read_until(&mut a, &mut sa, Duration::from_millis(300), |_| false).await;
    let pid = pid_of(&sa.out);

    // a second viewer joins the same terminal: the size, that it does not set it, then the screen so far
    let mut b = lab.connect(&lab.ticket("c03ef0d9"), None, "&cols=80&rows=24&proto=2").await;
    let mut sb = Seen::default();
    read_until(&mut b, &mut sb, Duration::from_secs(5), |s| has_type(s, "snapshot_end")).await;
    let types: Vec<&str> = sb.texts.iter().map(|t| t["type"].as_str().unwrap()).collect();
    assert_eq!(types, vec!["status", "size", "owner", "status", "snapshot_begin", "snapshot_end"]);
    assert_eq!(sb.texts[1], serde_json::json!({"type": "size", "cols": 100, "rows": 30}));
    assert_eq!(sb.texts[2], serde_json::json!({"type": "owner", "owner": false}));
    assert!(sb.out.contains(&format!("PID {pid}")), "the same process: {}", sb.out);
    assert_eq!(lab.get("/internal/open").await["open"].as_array().unwrap().len(), 2, "two viewers count as two");

    // b types: it owns the size now, and both see the new size in the stream and the same output
    b.send(Message::Binary(b"size\n".to_vec().into())).await.unwrap();
    read_until(&mut b, &mut sb, Duration::from_secs(5), |s| s.out.contains("SIZE 24 80")).await;
    read_until(&mut a, &mut sa, Duration::from_secs(5), |s| s.out.contains("SIZE 24 80")).await;
    assert!(sa.texts.contains(&serde_json::json!({"type": "size", "cols": 80, "rows": 24})));
    assert!(sa.texts.contains(&serde_json::json!({"type": "owner", "owner": false})));
    assert!(sb.texts.contains(&serde_json::json!({"type": "owner", "owner": true})));

    // a asks for focus: back to its size
    a.send(Message::Text(r#"{"type":"focus"}"#.into())).await.unwrap();
    a.send(Message::Binary(b"size\n".to_vec().into())).await.unwrap();
    read_until(&mut a, &mut sa, Duration::from_secs(5), |s| s.out.matches("SIZE 30 100").count() >= 2).await;
    assert_eq!(of_type(&sa, "size").last().unwrap(), &&serde_json::json!({"type": "size", "cols": 100, "rows": 30}));

    // a redraw: the screen again, as a snapshot
    a.send(Message::Text(r#"{"type":"redraw"}"#.into())).await.unwrap();
    let before = of_type(&sa, "snapshot_end").len();
    read_until(&mut a, &mut sa, Duration::from_secs(5), |s| of_type(s, "snapshot_end").len() > before).await;

    // b leaves: the terminal stays for a
    b.close(None).await.unwrap();
    drop(b);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(alive(&pid));
    a.send(Message::Binary(b"still here\n".to_vec().into())).await.unwrap();
    read_until(&mut a, &mut sa, Duration::from_secs(5), |s| s.out.contains("GOT still here")).await;
    assert!(sa.out.contains("GOT still here"));

    // a leaves too: the last one hangs up
    a.close(None).await.unwrap();
    drop(a);
    let end = Instant::now() + Duration::from_secs(5);
    while alive(&pid) && Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!alive(&pid), "the fake ssh is still running");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_old_client_gets_a_reset_before_a_redraw() {
    let lab = lab(|_| {}).await;
    let mut a = lab.connect(&lab.ticket("c03ef0d9"), None, "").await;
    let mut s = Seen::default();
    read_until(&mut a, &mut s, Duration::from_secs(10), |s| s.out.contains("PID ")).await;
    let n = s.out.len();
    a.send(Message::Text(r#"{"type":"redraw"}"#.into())).await.unwrap();
    read_until(&mut a, &mut s, Duration::from_secs(5), |s| has_type(s, "snapshot_end")).await;
    assert!(s.out[n..].starts_with("\x1bc"), "{:?}", &s.out[n..]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_big_paste_waits_for_the_machine_instead_of_closing() {
    let lab = lab(|_| {}).await;
    let mut ws = lab.connect(&lab.ticket("c03ef0d9"), None, "&proto=2").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.out.contains("PID")).await;
    // the machine reads nothing for 2 s while 6 MiB arrives, more than the hub queues (4 MiB): it waits, it
    // does not hang up
    const SIZE: usize = 6 << 20;
    ws.send(Message::Binary(format!("swallow {SIZE}\n").into_bytes().into())).await.unwrap();
    let mut line = vec![b'p'; 1023];
    line.push(b'\n');
    let paste: Vec<u8> = line.iter().copied().cycle().take(SIZE).collect();
    for piece in paste.chunks(64 * 1024) {
        ws.send(Message::Binary(piece.to_vec().into())).await.unwrap();
    }
    read_until(&mut ws, &mut s, Duration::from_secs(60), |s| s.out.contains("SWALLOWED") || s.close.is_some()).await;
    assert!(s.out.contains("SWALLOWED"), "{:?}", s.close);
    assert_eq!(s.close, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_viewer_that_went_away_is_let_go() {
    let lab = lab(|c| c.silence = Duration::from_millis(600)).await;
    let mut ws = lab.connect(&lab.ticket("c03ef0d9"), None, "").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.out.contains("PID")).await;
    // still reading, so its pongs go back: it stays well past the silence
    read_until(&mut ws, &mut s, Duration::from_millis(1500), |_| false).await;
    assert_eq!(s.close, None);
    assert_eq!(lab.get("/internal/open").await["open"].as_array().map(Vec::len), Some(1));
    // it stops reading, as a laptop that slept does: no pongs, and the hub ends it without waiting for idle
    let end = Instant::now() + Duration::from_secs(5);
    while lab.get("/internal/open").await["open"] != serde_json::json!([]) && Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(lab.get("/internal/open").await["open"], serde_json::json!([]));
    drop(ws);
}

#[tokio::test(flavor = "multi_thread")]
async fn pings_are_answered_while_a_paste_waits_for_the_machine() {
    let lab = lab(|_| {}).await;
    let mut ws = lab.connect(&lab.ticket("c03ef0d9"), None, "&proto=2").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.out.contains("PID")).await;
    // the machine reads nothing for 6 s while 6 MiB arrives, more than the terminal queues (4 MiB): the rest
    // waits in the hub, and the viewer's ping behind it is still answered at once
    const SIZE: usize = 6 << 20;
    let began = Instant::now();
    ws.send(Message::Binary(format!("stall {SIZE}\n").into_bytes().into())).await.unwrap();
    let mut line = vec![b'p'; 1023];
    line.push(b'\n');
    let paste: Vec<u8> = line.iter().copied().cycle().take(SIZE).collect();
    for piece in paste.chunks(64 * 1024) {
        ws.send(Message::Binary(piece.to_vec().into())).await.unwrap();
    }
    ws.send(Message::Text(r#"{"type":"ping"}"#.into())).await.unwrap();
    read_until(&mut ws, &mut s, Duration::from_secs(4), |s| has_type(s, "pong")).await;
    assert!(has_type(&s, "pong"), "no pong in {:?}", began.elapsed());
    assert!(!s.out.contains("SWALLOWED"), "the pong came while the paste was still waiting");
    // and the paste still arrives whole
    read_until(&mut ws, &mut s, Duration::from_secs(60), |s| s.out.contains("SWALLOWED") || s.close.is_some()).await;
    assert!(s.out.contains("SWALLOWED"), "{:?}", s.close);
    assert_eq!(s.close, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_time_of_zero_means_never() {
    let lab = lab(|c| c.idle = Duration::ZERO).await;
    let mut ws = lab.connect(&lab.ticket("c03ef0d9"), None, "").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.out.contains("PID")).await;
    read_until(&mut ws, &mut s, Duration::from_millis(1200), |s| s.close.is_some()).await;
    assert_eq!(s.close, None, "{:?}", s.texts);
    ws.send(Message::Binary(b"hello\n".to_vec().into())).await.unwrap();
    read_until(&mut ws, &mut s, Duration::from_secs(5), |s| s.out.contains("GOT hello")).await;
    assert!(s.out.contains("GOT hello"));
}

#[tokio::test(flavor = "multi_thread")]
async fn two_viewers_opening_a_chat_at_once_share_one_terminal() {
    let lab = lab(|_| {}).await;
    let (ta, tb) = (lab.ticket("c03ef0d9"), lab.ticket("c03ef0d9"));
    let (mut a, mut b) = tokio::join!(lab.connect(&ta, None, "&proto=2"), lab.connect(&tb, None, "&proto=2"));
    let (mut sa, mut sb) = (Seen::default(), Seen::default());
    read_until(&mut a, &mut sa, Duration::from_secs(10), |s| s.out.contains("PID")).await;
    read_until(&mut b, &mut sb, Duration::from_secs(10), |s| s.out.contains("PID")).await;
    // one of them types: both see the one program answer
    a.send(Message::Binary(b"both\n".to_vec().into())).await.unwrap();
    read_until(&mut a, &mut sa, Duration::from_secs(5), |s| s.out.contains("GOT both")).await;
    read_until(&mut b, &mut sb, Duration::from_secs(5), |s| s.out.contains("GOT both")).await;
    assert!(sa.out.contains("GOT both") && sb.out.contains("GOT both"), "{:?} / {:?}", sa.out, sb.out);
    let pid = |s: &Seen| s.out.split("PID ").nth(1).map(|r| r.split_whitespace().next().unwrap_or("").to_string());
    assert_eq!(pid(&sa), pid(&sb));
}

// ---- a hub inside a helper: terminals on this computer, with no ssh and no link (phase 4) ----------------------

struct LocalLab {
    addr: std::net::SocketAddr,
    hub: std::sync::Arc<Hub>,
    key: Vec<u8>,
}

/// A local hub named "laptop" whose program for a chat is the fake ssh script, served at `/own/v1/term`.
async fn local_lab() -> LocalLab {
    let dir = std::env::temp_dir().join(format!("sbhub-local-{}", std::process::id()));
    let mut cfg = Config::defaults_in(&dir);
    cfg.max_terms = 2;
    let key = vec![7u8; ticket::KEY_LEN];
    let local = server::Local {
        machine: "laptop".into(),
        label: "Laptop".into(),
        command: Box::new(|bg| server::LocalCommand {
            argv: vec![concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fake-ssh.sh").into(), "term".into(), bg.into()],
            env: vec![("TERM".into(), "xterm-256color".into())],
            env_remove: vec![],
            cwd: None,
        }),
    };
    let hub = Hub::local(cfg, key.clone(), local);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = server::local_router(hub.clone(), "/own/v1/term");
    tokio::spawn(async move { axum::serve(listener, app).await });
    assert!(!dir.exists(), "a local hub writes no key and no list");
    LocalLab { addr, hub, key }
}

impl LocalLab {
    async fn connect(&self, t: &str, size: &str) -> Ws {
        let req = format!("ws://{}/own/v1/term?t={t}{size}", self.addr).into_client_request().unwrap();
        tokio_tungstenite::connect_async(req).await.unwrap().0
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_terminal_runs_the_program_here_and_two_viewers_share_it() {
    let lab = local_lab().await;
    let t = lab.hub.local_ticket("c03ef0d9").unwrap();
    let mut a = lab.connect(&t, "&cols=100&rows=30&proto=2").await;
    let mut sa = Seen::default();
    read_until(&mut a, &mut sa, Duration::from_secs(10), |s| s.out.contains("PID ") && s.out.contains("TERM ")).await;
    assert_eq!(sa.texts[0], serde_json::json!({"type": "status", "state": "connecting"}));
    assert!(sa.out.contains("ARGS term c03ef0d9"), "{}", sa.out);
    assert!(sa.out.contains("SIZE 30 100"), "{}", sa.out);
    let pid = pid_of(&sa.out);

    let mut b = lab.connect(&lab.hub.local_ticket("c03ef0d9").unwrap(), "&cols=80&rows=24&proto=2").await;
    let mut sb = Seen::default();
    read_until(&mut b, &mut sb, Duration::from_secs(5), |s| has_type(s, "snapshot_end")).await;
    assert_eq!(sb.texts[2], serde_json::json!({"type": "owner", "owner": false}));
    assert!(sb.out.contains(&format!("PID {pid}")), "the same process: {}", sb.out);
    assert_eq!(lab.hub.open_terms().len(), 2);
    assert_eq!(lab.hub.open_terms()[0].machine, "laptop");

    b.send(Message::Binary(b"hello\n".to_vec().into())).await.unwrap();
    read_until(&mut a, &mut sa, Duration::from_secs(5), |s| s.out.contains("GOT hello")).await;
    assert!(sa.out.contains("GOT hello"), "what one types the other sees");

    a.send(Message::Binary(b"exit\n".to_vec().into())).await.unwrap();
    read_until(&mut a, &mut sa, Duration::from_secs(5), |s| s.close.is_some()).await;
    let exit = sa.texts.iter().find(|t| t["type"] == "exit").expect("exit frame");
    assert_eq!(exit["note"], "Detached. The chat keeps running.");
    let end = Instant::now() + Duration::from_secs(5);
    while alive(&pid) && Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!alive(&pid));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_ticket_works_once_and_only_for_this_computer() {
    let lab = local_lab().await;
    assert!(lab.hub.local_ticket("not a chat id; rm -rf").is_none(), "an id is checked before a ticket is made");
    let t = lab.hub.local_ticket("c03ef0d9").unwrap();
    let mut first = lab.connect(&t, "").await;
    let mut s = Seen::default();
    read_until(&mut first, &mut s, Duration::from_secs(10), |s| s.out.contains("PID ")).await;
    assert!(s.out.contains("PID "));

    for bad in [
        t.clone(),
        "nonsense".into(),
        // signed with this hub's key, for another computer's name
        ticket::mint(&lab.key, "tower", "c03ef0d9", now(), 60),
        ticket::mint(&[1u8; 32], "laptop", "c03ef0d9", now(), 60),
        ticket::mint(&lab.key, "laptop", "c03ef0d9", now() - 120, 60),
    ] {
        let mut ws = lab.connect(&bad, "").await;
        let mut s = Seen::default();
        read_until(&mut ws, &mut s, Duration::from_secs(5), |s| s.close.is_some()).await;
        assert_eq!(s.close, Some(4401), "{bad}: {s:?}");
        assert!(s.out.is_empty());
    }
    assert_eq!(lab.hub.open_terms().len(), 1, "none of them opened anything");
    // a hub that is not local mints nothing this way
    let other = super_lab_hub().await;
    assert!(other.local_ticket("c03ef0d9").is_none());
}

async fn super_lab_hub() -> std::sync::Arc<Hub> {
    let dir = std::env::temp_dir().join(format!("sbhub-notlocal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::defaults_in(&dir);
    cfg.key_path = dir.join("term.key");
    Hub::new(cfg).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_local_hub_keeps_the_limit_on_terminals() {
    let lab = local_lab().await;
    let mut held = vec![];
    for bg in ["aaaaaaa1", "aaaaaaa2"] {
        let mut ws = lab.connect(&lab.hub.local_ticket(bg).unwrap(), "").await;
        let mut s = Seen::default();
        read_until(&mut ws, &mut s, Duration::from_secs(10), |s| s.out.contains("PID ")).await;
        held.push(ws);
    }
    let mut ws = lab.connect(&lab.hub.local_ticket("aaaaaaa3").unwrap(), "").await;
    let mut s = Seen::default();
    read_until(&mut ws, &mut s, Duration::from_secs(5), |s| s.close.is_some()).await;
    assert_eq!(s.close, Some(4429));
}
