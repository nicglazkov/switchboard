//! The hub end to end on this machine, with a fake ssh (tests/fake-ssh.sh) standing in for the real one.

use futures_util::{SinkExt, StreamExt};
use sb_hub::config::Config;
use sb_hub::server::{self, Hub};
use sb_hub::ticket;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::Message;

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
    let dir = std::env::temp_dir().join(format!("sbhub-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let machines = dir.join("machines.json");
    std::fs::write(&machines, r#"{"machines":[{"name":"lab","label":"Lab","target":"me@lab","os":"linux"}],
        "origins":["http://10.0.0.5:8130","http://100.64.0.5:8130","http://10.0.0.5:8150"]}"#).unwrap();
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
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
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
    assert!(s.texts.contains(&serde_json::json!({"type": "size", "cols": 80, "rows": 24})), "the size comes back as a frame");

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
    assert_eq!(exit["note"], "Detached. The session keeps running.");
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

    for bad in [t.clone(), "nonsense".into(), ticket::mint(&[1u8; 32], "lab", "c03ef0d9", now(), 60),
        ticket::mint(&lab.key, "lab", "c03ef0d9", now() - 120, 60)] {
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
    for (origin, want) in [("http://evil.example", Some(4403)), ("null", Some(4403)), ("http://10.0.0.5:8150", None),
        ("http://localhost:8151", None), ("http://100.64.0.5:8130", None), ("http://10.0.0.9:8130", Some(4403))] {
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

fn alive(pid: &str) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
        && !std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default().contains(") Z ")
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
    assert!(exit["note"].as_str().unwrap().starts_with("Could not reach Lab"));
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
