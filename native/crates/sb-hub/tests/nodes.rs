//! Linked machines end to end on this machine: pairing, the `/node` link, RPCs and terminals carried over it.
//! The node here is played by the test itself, so every message the hub sends can be checked.

use futures_util::{SinkExt, StreamExt};
use sb_hub::config::Config;
use sb_hub::link_proto::{self, FromNode, ToNode};
use sb_hub::server::{self, Hub};
use sb_hub::ticket;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{self, Message};

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

async fn lab() -> Lab {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "sbnode-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let machines = dir.join("machines.json");
    std::fs::write(&machines, r#"{"machines":[{"name":"desk","label":"Desk","target":"me@desk","os":"mac"}],"origins":["http://10.0.0.5:8130"]}"#).unwrap();
    let mut cfg = Config::defaults_in(&dir);
    cfg.listen = "127.0.0.1:0".parse().unwrap();
    cfg.machines_path = machines;
    cfg.nodes_path = dir.join("nodes.json");
    cfg.key_path = dir.join("cfg/term.key");
    cfg.ssh = "/bin/false".into();
    let hub = Hub::new(cfg.clone()).unwrap();
    let key = std::fs::read(&cfg.key_path).unwrap();
    let listener = TcpListener::bind(cfg.listen).await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(server::serve(hub, listener));
    Lab { addr, key, dir }
}

async fn http(addr: std::net::SocketAddr, method: &str, path: &str, body: &str) -> (u16, Value) {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

impl Lab {
    async fn http(&self, method: &str, path: &str, body: &str) -> (u16, Value) {
        http(self.addr, method, path, body).await
    }

    async fn pair(&self, name: &str) -> String {
        let (_, v) = self.http("POST", "/internal/pair", "").await;
        let code = v["code"].as_str().unwrap().to_string();
        let (st, v) = self
            .http("POST", "/pair", &json!({"code": code, "name": name, "label": "Lab box", "os": "linux"}).to_string())
            .await;
        assert_eq!(st, 200, "{v}");
        v["token"].as_str().unwrap().to_string()
    }

    async fn node(&self, token: &str) -> Result<Ws, tungstenite::Error> {
        let mut req = format!("ws://{}/node", self.addr).into_client_request().unwrap();
        req.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap());
        let (mut ws, _) = tokio_tungstenite::connect_async(req).await?;
        let hello = FromNode::Hello { version: "9.9.9".into(), os: "linux".into(), label: "labhost".into() };
        ws.send(Message::Text(link_proto::encode(&hello).into())).await.unwrap();
        Ok(ws)
    }

    async fn term(&self, bg: &str) -> Ws {
        let t = ticket::mint(&self.key, "lab", bg, server::unix_now(), ticket::TTL);
        let mut req = format!("ws://{}/term?t={t}&cols=90&rows=30", self.addr).into_client_request().unwrap();
        req.headers_mut().insert("Origin", "http://10.0.0.5:8130".parse().unwrap());
        tokio_tungstenite::connect_async(req).await.unwrap().0
    }
}

enum Got {
    Msg(ToNode),
    Bin(u32, Vec<u8>),
    Close(u16),
}

/// The next thing the hub sends the node (pings skipped).
async fn from_hub(ws: &mut Ws) -> Got {
    loop {
        match tokio::time::timeout(Duration::from_secs(8), ws.next()).await.expect("the hub said nothing") {
            Some(Ok(Message::Text(t))) => return Got::Msg(serde_json::from_str(t.as_str()).unwrap()),
            Some(Ok(Message::Binary(b))) => {
                let (ch, d) = link_proto::unframe(&b).unwrap();
                return Got::Bin(ch, d.to_vec());
            }
            Some(Ok(Message::Close(f))) => return Got::Close(f.map(|f| u16::from(f.code)).unwrap_or(1005)),
            Some(Ok(_)) => continue,
            Some(Err(_)) | None => return Got::Close(1006),
        }
    }
}

/// What a terminal viewer sees: JSON texts and output, until `until` matches or the socket closes.
async fn viewer(ws: &mut Ws, until: impl Fn(&[Value], &str) -> bool) -> (Vec<Value>, String, Option<u16>) {
    let (mut texts, mut out, mut close) = (vec![], String::new(), None);
    while !until(&texts, &out) && close.is_none() {
        match tokio::time::timeout(Duration::from_secs(8), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => texts.push(serde_json::from_str(t.as_str()).unwrap()),
            Ok(Some(Ok(Message::Binary(b)))) => out.push_str(&String::from_utf8_lossy(&b)),
            Ok(Some(Ok(Message::Close(f)))) => close = Some(f.map(|f| u16::from(f.code)).unwrap_or(1005)),
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(_))) | Ok(None) => close = Some(1006),
            Err(_) => break,
        }
    }
    (texts, out, close)
}

fn http_status(e: tungstenite::Error) -> u16 {
    match e {
        tungstenite::Error::Http(r) => r.status().as_u16(),
        e => panic!("not an HTTP refusal: {e}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn pairing_rules() {
    let lab = lab().await;
    let (st, v) = lab.http("POST", "/internal/pair", "").await;
    assert_eq!(st, 200);
    let code = v["code"].as_str().unwrap();
    assert_eq!(code.len(), 6);
    assert!(v["expires"].as_u64().unwrap() > server::unix_now() + 500);

    let (st, v) = lab.http("POST", "/pair", r#"{"code":"ZZZZZZ","name":"lab"}"#).await;
    assert_eq!((st, v["ok"].clone()), (403, json!(false)));
    assert!(v["error"].as_str().unwrap().contains("wrong or has expired"));
    let (st, v) = lab.http("POST", "/pair", &json!({"code": code, "name": "desk"}).to_string()).await;
    assert_eq!(st, 409, "an ssh machine's name stays its own");
    assert!(v["error"].as_str().unwrap().starts_with("Desk already uses that name"));
    let (st, _) = lab.http("POST", "/pair", "not json").await;
    assert_eq!(st, 400);
    let (st, v) = lab.http("POST", "/pair", &json!({"code": code.to_lowercase(), "name": "lab"}).to_string()).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["machine"], "lab");
    assert_eq!(v["label"], "lab", "no label: the name");
    let (st, _) = lab.http("POST", "/pair", &json!({"code": code, "name": "lab"}).to_string()).await;
    assert_eq!(st, 403, "a code works once");

    // security C2: a fresh code never takes over a linked machine's name; its own token does
    let first = v["token"].as_str().unwrap().to_string();
    let (_, c2) = lab.http("POST", "/internal/pair", "").await;
    let (st, v2) = lab.http("POST", "/pair", &json!({"code": c2["code"], "name": "lab"}).to_string()).await;
    assert_eq!(st, 409, "{v2}");
    assert!(v2["error"].as_str().unwrap().starts_with("lab already uses that name"));
    let (st, _) = lab
        .http("POST", "/pair", &json!({"code": c2["code"], "name": "lab", "token": "0".repeat(64)}).to_string())
        .await;
    assert_eq!(st, 409, "a wrong token is no token");
    let (st, v2) =
        lab.http("POST", "/pair", &json!({"code": c2["code"], "name": "lab", "token": first}).to_string()).await;
    assert_eq!(st, 200, "the machine itself pairs again: {v2}");
    let v = v2;

    // the token is the only way in, and pages never get in
    let token = v["token"].as_str().unwrap();
    assert_eq!(http_status(lab.node("0".repeat(64).as_str()).await.err().unwrap()), 401);
    let mut req = format!("ws://{}/node", lab.addr).into_client_request().unwrap();
    req.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap());
    req.headers_mut().insert("Origin", "http://10.0.0.5:8130".parse().unwrap());
    assert_eq!(http_status(tokio_tungstenite::connect_async(req).await.err().unwrap()), 403);

    // listed, offline until it links
    let (_, v) = lab.http("GET", "/internal/nodes", "").await;
    assert_eq!(v["nodes"][0]["name"], "lab");
    assert_eq!(v["nodes"][0]["online"], false);
    let (st, v) = lab.http("POST", "/internal/rpc/lab", r#"{"verb":"list"}"#).await;
    assert_eq!(st, 503);
    assert_eq!(v["error"], "lab is not linked to the hub right now.");
    let (st, _) = lab.http("POST", "/internal/rpc/nope", r#"{"verb":"list"}"#).await;
    assert_eq!(st, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn rpcs_and_terminals_over_the_link() {
    let lab = lab().await;
    let token = lab.pair("lab").await;
    let mut node = lab.node(&token).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (_, v) = lab.http("GET", "/internal/nodes", "").await;
    assert_eq!(v["nodes"][0]["online"], true);
    assert_eq!(v["nodes"][0]["version"], "9.9.9");
    assert_eq!(v["nodes"][0]["label"], "Lab box");
    assert_eq!(lab.http("GET", "/healthz", "").await.1["linked"], 1);

    // an RPC: the hub forwards the request and returns the node's answer as is
    let ask = tokio::spawn(http(lab.addr, "POST", "/internal/rpc/lab?timeout=20", r#"{"verb":"list"}"#));
    let Got::Msg(ToNode::Rpc { id, req, timeout }) = from_hub(&mut node).await else { panic!("expected an rpc") };
    assert_eq!((req, timeout), (json!({"verb": "list"}), 20));
    let res = FromNode::RpcResult { id, res: json!({"ok": true, "sessions": [{"sid": "s1"}]}) };
    node.send(Message::Text(link_proto::encode(&res).into())).await.unwrap();
    let (st, v) = ask.await.unwrap();
    assert_eq!((st, v), (200, json!({"ok": true, "sessions": [{"sid": "s1"}]})));

    // a terminal: opened on the node, bytes both ways, resize, exit
    let mut term = lab.term("c03ef0d9").await;
    let Got::Msg(ToNode::TermOpen { ch, bg, cols, rows }) = from_hub(&mut node).await else {
        panic!("expected term.open")
    };
    assert_eq!((bg.as_str(), cols, rows), ("c03ef0d9", 90, 30));
    node.send(Message::Binary(link_proto::frame(ch, b"hello from lab\r\n").into())).await.unwrap();
    let (texts, out, _) = viewer(&mut term, |_, o| o.contains("hello from lab")).await;
    assert_eq!(
        texts,
        vec![
            json!({"type": "status", "state": "connecting"}),
            json!({"type": "size", "cols": 90, "rows": 30}),
            json!({"type": "owner", "owner": true}),
            json!({"type": "status", "state": "open"}),
        ]
    );
    assert!(out.contains("hello from lab"));
    let (_, v) = lab.http("GET", "/internal/open", "").await;
    assert_eq!(v["open"][0]["machine"], "lab", "the reaper sees it");
    term.send(Message::Binary(b"typed".to_vec().into())).await.unwrap();
    let Got::Bin(c, data) = from_hub(&mut node).await else { panic!("expected input") };
    assert_eq!((c, data.as_slice()), (ch, &b"typed"[..]));
    term.send(Message::Text(r#"{"type":"resize","cols":70,"rows":20}"#.into())).await.unwrap();
    let Got::Msg(m) = from_hub(&mut node).await else { panic!("expected resize") };
    assert_eq!(m, ToNode::TermResize { ch, cols: 70, rows: 20 });
    node.send(Message::Text(link_proto::encode(&FromNode::TermExit { ch, code: Some(0) }).into())).await.unwrap();
    let (texts, _, close) = viewer(&mut term, |t, _| t.iter().any(|t| t["type"] == "exit")).await;
    assert_eq!(
        texts.last().unwrap(),
        &json!({"type": "exit", "code": 0, "note": "Detached. The session keeps running."})
    );
    let (_, _, close) =
        if close.is_none() { viewer(&mut term, |_, _| false).await } else { (vec![], String::new(), close) };
    assert_eq!(close, Some(1000));

    // the viewer leaves: the node is told to hang up
    let mut term = lab.term("c03ef0d9").await;
    let Got::Msg(ToNode::TermOpen { ch, .. }) = from_hub(&mut node).await else { panic!("expected term.open") };
    term.close(None).await.unwrap();
    let Got::Msg(m) = from_hub(&mut node).await else { panic!("expected term.close") };
    assert_eq!(m, ToNode::TermClose { ch });

    // the link drops with a terminal open: the viewer is told, the session keeps running
    let mut term = lab.term("c03ef0d9").await;
    let Got::Msg(ToNode::TermOpen { ch, .. }) = from_hub(&mut node).await else { panic!("expected term.open") };
    node.send(Message::Binary(link_proto::frame(ch, b"up\r\n").into())).await.unwrap();
    viewer(&mut term, |_, o| o.contains("up")).await;
    drop(node);
    let (texts, _, _) = viewer(&mut term, |t, _| t.iter().any(|t| t["type"] == "exit")).await;
    let note = texts.last().unwrap()["note"].as_str().unwrap().to_string();
    assert!(note.starts_with("Could not reach Lab box"), "{note}");

    // no link: terminals and RPCs say so plainly
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut term = lab.term("c03ef0d9").await;
    let (texts, _, close) = viewer(&mut term, |_, _| false).await;
    assert_eq!(close, Some(1011));
    assert!(texts[0]["error"].as_str().unwrap().starts_with("Lab box is not linked to the hub right now"));
}

#[tokio::test(flavor = "multi_thread")]
async fn rpc_timeout_replaced_and_unpaired_links() {
    let lab = lab().await;
    let token = lab.pair("lab").await;
    let mut first = lab.node(&token).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // a node that never answers
    let (st, v) = lab.http("POST", "/internal/rpc/lab?timeout=1", r#"{"verb":"list"}"#).await;
    assert_eq!(st, 504);
    assert_eq!(v["error"], "Lab box did not answer in 1 s.");
    assert!(matches!(from_hub(&mut first).await, Got::Msg(ToNode::Rpc { .. })));

    // a second node with the same token takes over; the first is told why
    let mut second = lab.node(&token).await.unwrap();
    assert!(matches!(from_hub(&mut first).await, Got::Close(4409)));

    // unpaired: the link closes with 4401 and the token no longer opens one
    let (st, _) = lab.http("POST", "/internal/unpair/lab", "").await;
    assert_eq!(st, 200);
    assert!(matches!(from_hub(&mut second).await, Got::Close(4401)));
    assert_eq!(http_status(lab.node(&token).await.err().unwrap()), 401);
    let (st, _) = lab.http("POST", "/internal/unpair/lab", "").await;
    assert_eq!(st, 404);
}

/// The status line's code for a request written out by hand.
async fn raw_status(addr: std::net::SocketAddr, head: &str) -> u16 {
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(format!("{head}Content-Length: 2\r\nConnection: close\r\n\r\n{{}}").as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    out.split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn internal_routes_refuse_pages() {
    // security H3: a page in a browser on the hub's machine could POST to /internal/* or read it by DNS rebinding
    let lab = lab().await;
    let json = "Content-Type: application/json\r\n";
    let ok = format!("POST /internal/pair HTTP/1.1\r\nHost: 127.0.0.1:8132\r\n{json}");
    assert_eq!(raw_status(lab.addr, &ok).await, 200);
    for (what, head) in [
        (
            "a page's Origin",
            format!("POST /internal/pair HTTP/1.1\r\nHost: 127.0.0.1:8132\r\nOrigin: https://evil.example\r\n{json}"),
        ),
        (
            "a fetch",
            format!("POST /internal/pair HTTP/1.1\r\nHost: 127.0.0.1:8132\r\nSec-Fetch-Site: cross-site\r\n{json}"),
        ),
        ("a rebound name", format!("POST /internal/pair HTTP/1.1\r\nHost: evil.example:8132\r\n{json}")),
        ("a rebound read", "GET /internal/nodes HTTP/1.1\r\nHost: rebind.evil.example\r\n".to_string()),
        (
            "text/plain",
            "POST /internal/rpc/lab HTTP/1.1\r\nHost: localhost:8132\r\nContent-Type: text/plain\r\n".to_string(),
        ),
    ] {
        assert_eq!(raw_status(lab.addr, &head).await, 403, "{what}");
    }
    assert_eq!(raw_status(lab.addr, "GET /internal/nodes HTTP/1.1\r\nHost: [::1]:8132\r\n").await, 200);
    assert_eq!(
        raw_status(lab.addr, "GET /healthz HTTP/1.1\r\nHost: studio.local\r\n").await,
        200,
        "other routes as before"
    );
}
