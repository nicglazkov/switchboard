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
    /// The same hub's tunnel listener (server.rs `public_router`): what a tunnel forwards to.
    tunnel: std::net::SocketAddr,
    key: Vec<u8>,
    dir: PathBuf,
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn lab() -> Lab {
    lab_with(|_| {}).await
}

async fn lab_with(tweak: impl FnOnce(&mut Config)) -> Lab {
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
    tweak(&mut cfg);
    let hub = Hub::new(cfg.clone()).unwrap();
    let key = std::fs::read(&cfg.key_path).unwrap();
    let listener = TcpListener::bind(cfg.listen).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let public = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tunnel = public.local_addr().unwrap();
    tokio::spawn(server::serve_public(hub.clone(), public));
    tokio::spawn(server::serve(hub, listener));
    Lab { addr, tunnel, key, dir }
}

async fn node_at(addr: std::net::SocketAddr, token: &str) -> Result<Ws, tungstenite::Error> {
    let mut req = format!("ws://{addr}/node").into_client_request().unwrap();
    req.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await?;
    let hello = FromNode::Hello { version: "9.9.9".into(), os: "linux".into(), label: "labhost".into() };
    ws.send(Message::Text(link_proto::encode(&hello).into())).await.unwrap();
    Ok(ws)
}

async fn term_at(addr: std::net::SocketAddr, key: &[u8], bg: &str) -> Ws {
    let t = ticket::mint(key, "lab", bg, server::unix_now(), ticket::TTL);
    let mut req = format!("ws://{addr}/term?t={t}&cols=90&rows=30").into_client_request().unwrap();
    req.headers_mut().insert("Origin", "http://10.0.0.5:8130".parse().unwrap());
    tokio_tungstenite::connect_async(req).await.unwrap().0
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
        node_at(self.addr, token).await
    }

    async fn term(&self, bg: &str) -> Ws {
        term_at(self.addr, &self.key, bg).await
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
async fn a_linked_machine_in_the_servers_list_pairs_again_under_its_name() {
    let lab = lab().await;
    let pair = async |body: serde_json::Value| {
        let (_, c) = lab.http("POST", "/internal/pair", "").await;
        let mut body = body;
        body["code"] = c["code"].clone();
        lab.http("POST", "/pair", &body.to_string()).await
    };
    let (st, v) = pair(json!({"name": "tower"})).await;
    assert_eq!(st, 200, "{v}");
    let first = v["token"].as_str().unwrap().to_string();
    // the server now lists it, as it does every linked machine
    std::fs::write(
        lab.dir.join("machines.json"),
        r#"{"machines":[{"name":"desk","label":"Desk","target":"me@desk","os":"mac"},
            {"name":"tower","label":"Tower","target":"node:tower","os":"windows"}]}"#,
    )
    .unwrap();

    // with its token it pairs again; a stranger with a fresh code still does not get the name
    let (st, v) = pair(json!({"name": "tower"})).await;
    assert_eq!(st, 409, "{v}");
    let (st, v) = pair(json!({"name": "tower", "token": first})).await;
    assert_eq!(st, 200, "the row in the server's list is not somebody else: {v}");

    // unlinked (its token file was lost, say): the row stays in the server's list, and a new code links it again
    let (st, _) = lab.http("POST", "/internal/unpair/tower", "").await;
    assert_eq!(st, 200);
    let (st, v) = pair(json!({"name": "tower"})).await;
    assert_eq!(st, 200, "{v}");
    assert_eq!(v["machine"], "tower");

    // an ssh machine's name is still its own
    let (st, _) = pair(json!({"name": "desk"})).await;
    assert_eq!(st, 409);
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
    assert_eq!(texts.last().unwrap(), &json!({"type": "exit", "code": 0, "note": "Detached. The chat keeps running."}));
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
    assert!(note.starts_with("Couldn't reach Lab box"), "{note}");

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

#[tokio::test(flavor = "multi_thread")]
async fn the_tunnel_listener_has_no_internal_routes() {
    // A connection a tunnel forwards arrives from 127.0.0.1. On the main listener that is all `/internal/*` asks
    // for, so the tunnel listener must not have those routes at all: 404, never 403, and nothing happens.
    let lab = lab().await;
    let token = lab.pair("lab").await;
    let mut node = lab.node(&token).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let json = "Content-Type: application/json\r\n";
    // each asked the way the server on this computer asks, which the main listener answers
    let asks = [
        ("GET", "/internal/open"),
        ("GET", "/internal/nodes"),
        ("POST", "/internal/pair"),
        ("POST", "/internal/rpc/lab?timeout=1"),
        ("POST", "/internal/unpair/nope"),
    ];
    for (method, path) in asks {
        let head = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:8132\r\n{json}");
        assert_eq!(raw_status(lab.tunnel, &head).await, 404, "{method} {path} on the tunnel listener");
    }
    // nothing was done: no command reached the linked computer, and it is still paired and linked
    let quiet = tokio::time::timeout(Duration::from_millis(300), node.next()).await;
    assert!(quiet.is_err(), "the node heard something: {quiet:?}");
    let (st, _) = http(lab.tunnel, "POST", "/internal/unpair/lab", "").await;
    assert_eq!(st, 404);
    let (_, v) = lab.http("GET", "/internal/nodes", "").await;
    assert_eq!((v["nodes"][0]["name"].clone(), v["nodes"][0]["online"].clone()), (json!("lab"), json!(true)));
    // ... and no code was made there: the answer has none
    let (st, v) = http(lab.tunnel, "POST", "/internal/pair", "").await;
    assert_eq!((st, v), (404, json!({"ok": false, "error": "not found"})));
    // other spellings and other methods find nothing either
    for head in [
        "GET //internal/open HTTP/1.1\r\nHost: 127.0.0.1\r\n",
        "GET /internal HTTP/1.1\r\nHost: 127.0.0.1\r\n",
        "GET /internal/ HTTP/1.1\r\nHost: 127.0.0.1\r\n",
        "GET /INTERNAL/open HTTP/1.1\r\nHost: 127.0.0.1\r\n",
        "GET /%69nternal/open HTTP/1.1\r\nHost: 127.0.0.1\r\n",
        "GET /term/../internal/open HTTP/1.1\r\nHost: 127.0.0.1\r\n",
        "GET /healthz/../internal/nodes HTTP/1.1\r\nHost: 127.0.0.1\r\n",
        "GET http://127.0.0.1:8132/internal/open HTTP/1.1\r\nHost: 127.0.0.1:8132\r\n",
        "OPTIONS /internal/pair HTTP/1.1\r\nHost: 127.0.0.1\r\n",
        "GET / HTTP/1.1\r\nHost: 127.0.0.1\r\n",
        "GET /api/sessions HTTP/1.1\r\nHost: 127.0.0.1\r\n",
    ] {
        assert_eq!(raw_status(lab.tunnel, head).await, 404, "{head}");
    }
    // the same asks on the main listener, from this computer: answered, as the server needs
    assert_eq!(raw_status(lab.addr, "GET /internal/open HTTP/1.1\r\nHost: 127.0.0.1:8132\r\n").await, 200);
    assert_eq!(raw_status(lab.addr, "GET /internal/nodes HTTP/1.1\r\nHost: 127.0.0.1:8132\r\n").await, 200);
    // what the tunnel listener does have
    let (st, v) = http(lab.tunnel, "GET", "/healthz", "").await;
    assert_eq!((st, v["ok"].clone(), v["linked"].clone()), (200, json!(true), json!(1)));
    let (st, v) = http(lab.tunnel, "POST", "/pair", r#"{"code":"ZZZZZZ","name":"other"}"#).await;
    assert_eq!(st, 403, "pairing is there, and a wrong code is a wrong code: {v}");
    assert_eq!(http_status(node_at(lab.tunnel, &"0".repeat(64)).await.err().unwrap()), 401, "and so is /node");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_computer_pairs_links_and_shows_a_terminal_through_the_tunnel_listener() {
    let lab = lab().await;
    // the code comes from the server on this computer (main listener); the new computer uses it through the tunnel
    let (_, v) = lab.http("POST", "/internal/pair", "").await;
    let body = json!({"code": v["code"], "name": "lab", "label": "Lab box", "os": "linux"}).to_string();
    let (st, v) = http(lab.tunnel, "POST", "/pair", &body).await;
    assert_eq!(st, 200, "{v}");
    let token = v["token"].as_str().unwrap().to_string();
    let mut node = node_at(lab.tunnel, &token).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    // one hub behind both listeners: the server sees the link, and its RPC goes down it
    let (_, v) = lab.http("GET", "/internal/nodes", "").await;
    assert_eq!(v["nodes"][0]["online"], true);
    let ask = tokio::spawn(http(lab.addr, "POST", "/internal/rpc/lab?timeout=20", r#"{"verb":"list"}"#));
    let Got::Msg(ToNode::Rpc { id, .. }) = from_hub(&mut node).await else { panic!("expected an rpc") };
    let res = FromNode::RpcResult { id, res: json!({"ok": true}) };
    node.send(Message::Text(link_proto::encode(&res).into())).await.unwrap();
    assert_eq!(ask.await.unwrap(), (200, json!({"ok": true})));
    // a terminal opened through the tunnel listener, with a ticket from the server as always
    let mut term = term_at(lab.tunnel, &lab.key, "c03ef0d9").await;
    let Got::Msg(ToNode::TermOpen { ch, bg, .. }) = from_hub(&mut node).await else { panic!("expected term.open") };
    assert_eq!(bg, "c03ef0d9");
    node.send(Message::Binary(link_proto::frame(ch, b"hello through the tunnel\r\n").into())).await.unwrap();
    let (_, out, _) = viewer(&mut term, |_, o| o.contains("hello through the tunnel")).await;
    assert!(out.contains("hello through the tunnel"), "{out}");
    // a ticket is still needed there, and a page that is not the server's own is still refused
    let mut req = format!("ws://{}/term?t=nope", lab.tunnel).into_client_request().unwrap();
    req.headers_mut().insert("Origin", "http://10.0.0.5:8130".parse().unwrap());
    let mut bad = tokio_tungstenite::connect_async(req).await.unwrap().0;
    let (_, _, close) = viewer(&mut bad, |_, _| false).await;
    assert_eq!(close, Some(sb_hub::proto::CLOSE_TICKET));
    let t = ticket::mint(&lab.key, "lab", "c03ef0d9", server::unix_now(), ticket::TTL);
    let mut req = format!("ws://{}/term?t={t}", lab.tunnel).into_client_request().unwrap();
    req.headers_mut().insert("Origin", "https://evil.example".parse().unwrap());
    let mut bad = tokio_tungstenite::connect_async(req).await.unwrap().0;
    let (_, _, close) = viewer(&mut bad, |_, _| false).await;
    assert_eq!(close, Some(sb_hub::proto::CLOSE_ORIGIN));
}

/// A port nothing listens on right now.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// The hub program itself, started with `home` as the user's folder. Stopped when dropped.
struct Program {
    child: std::process::Child,
    log: std::sync::Arc<std::sync::Mutex<String>>,
}

impl Drop for Program {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Program {
    fn start(home: &std::path::Path, listen: &str, tunnel: u16) -> Program {
        use std::io::BufRead;
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_switchboard-hub"))
            .env("HOME", home)
            .env("USERPROFILE", home)
            .env("SWITCHBOARD_HUB_LISTEN", listen)
            .env("SWITCHBOARD_HUB_TUNNEL_LISTEN", format!("127.0.0.1:{tunnel}"))
            .env_remove("SWITCHBOARD_CONFIG_DIR")
            .env_remove("SWITCHBOARD_STATE_DIR")
            .env_remove("SWITCHBOARD_SERVER_JSON")
            .env_remove("SWITCHBOARD_TERM_KEY")
            .env_remove("SWITCHBOARD_MACHINES")
            .env_remove("SWITCHBOARD_NODES")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let log = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let (err, into) = (child.stderr.take().unwrap(), log.clone());
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                into.lock().unwrap().push_str(&(line + "\n"));
            }
        });
        Program { child, log }
    }

    fn said(&self) -> String {
        self.log.lock().unwrap().clone()
    }

    /// Waits until the log has `text`. Panics with the log when it never does.
    async fn wait_for(&self, text: &str) {
        for _ in 0..100 {
            if self.said().contains(text) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the hub never said {text:?}. It said:\n{}", self.said());
    }
}

async fn listens(addr: std::net::SocketAddr) -> bool {
    matches!(tokio::time::timeout(Duration::from_secs(2), tokio::net::TcpStream::connect(addr)).await, Ok(Ok(_)))
}

/// An address of this computer that another computer could use, when it has one. No packet is sent.
fn lan_address() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn home_with(server_json: Option<&str>) -> PathBuf {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "sbhub-home-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".config/switchboard")).unwrap();
    std::fs::create_dir_all(dir.join(".local/share/switchboard")).unwrap();
    if let Some(text) = server_json {
        std::fs::write(dir.join(".config/switchboard/server.json"), text).unwrap();
    }
    dir
}

#[tokio::test(flavor = "multi_thread")]
async fn with_keys_only_the_hub_program_listens_on_this_computer_alone() {
    // asked for every address, as a service file from before keys does: `only` in server.json wins
    let home = home_with(Some(r#"{"bind": ["0.0.0.0"], "hosts": [], "origins": [], "peers": [], "keys": "only"}"#));
    let (port, tunnel) = (free_port(), free_port());
    let hub = Program::start(&home, &format!("0.0.0.0:{port}"), tunnel);
    hub.wait_for(&format!("tunnel connections on 127.0.0.1:{tunnel}")).await;
    let said = hub.said();
    assert!(said.contains(&format!(" on 127.0.0.1:{port}, keys only")), "{said}");
    assert!(said.contains(&format!("listens on 127.0.0.1:{port} and not on 0.0.0.0:{port}")), "{said}");
    let (main, tun): (std::net::SocketAddr, std::net::SocketAddr) =
        (([127, 0, 0, 1], port).into(), ([127, 0, 0, 1], tunnel).into());
    assert_eq!(http(main, "GET", "/internal/open", "").await.0, 200, "the server on this computer still asks");
    assert_eq!(http(tun, "GET", "/internal/open", "").await.0, 404);
    assert_eq!(http(tun, "GET", "/healthz", "").await.0, 200);
    if let Some(ip) = lan_address() {
        assert!(!listens((ip, port).into()).await, "the main port answers on {ip}");
        assert!(!listens((ip, tunnel).into()).await, "the tunnel port answers on {ip}");
    }
    drop(hub);
    let _ = std::fs::remove_dir_all(home);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_damaged_server_json_keeps_the_hub_program_on_this_computer() {
    let home = home_with(Some("\0\0\0\0\0\0\0\0"));
    let (port, tunnel) = (free_port(), free_port());
    let hub = Program::start(&home, &format!("0.0.0.0:{port}"), tunnel);
    hub.wait_for(&format!(" on 127.0.0.1:{port}, keys only")).await;
    let said = hub.said();
    assert!(said.contains("server.json could not be read and has no good copy"), "{said}");
    assert!(said.contains("answers this computer only"), "{said}");
    if let Some(ip) = lan_address() {
        assert!(!listens((ip, port).into()).await, "the main port answers on {ip}");
    }
    let untouched = std::fs::read(home.join(".config/switchboard/server.json")).unwrap();
    assert_eq!(untouched, b"\0\0\0\0\0\0\0\0", "the server's file is only read");
    drop(hub);
    let _ = std::fs::remove_dir_all(home);
}

#[tokio::test(flavor = "multi_thread")]
async fn without_a_keys_entry_the_hub_program_opens_no_tunnel_port() {
    // an install from before keys: where it was told to listen, and nothing new
    for server_json in [None, Some(r#"{"bind": ["0.0.0.0"], "hosts": ["studio"], "origins": [], "peers": []}"#)] {
        let home = home_with(server_json);
        let (port, tunnel) = (free_port(), free_port());
        let hub = Program::start(&home, &format!("127.0.0.1:{port}"), tunnel);
        hub.wait_for(&format!(" on 127.0.0.1:{port}, up to 16 terminals")).await;
        let main: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
        assert_eq!(http(main, "GET", "/healthz", "").await.0, 200);
        assert_eq!(http(main, "GET", "/internal/open", "").await.0, 200);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!listens(([127, 0, 0, 1], tunnel).into()).await, "a tunnel port with keys off");
        let said = hub.said();
        assert!(!said.contains("tunnel") && !said.contains("keys"), "{said}");
        drop(hub);
        let _ = std::fs::remove_dir_all(home);
    }
}

async fn open_with(lab: &Lab, t: &str) -> Ws {
    let mut req = format!("ws://{}/term?t={t}&cols=90&rows=30", lab.addr).into_client_request().unwrap();
    req.headers_mut().insert("Origin", "http://10.0.0.5:8130".parse().unwrap());
    tokio_tungstenite::connect_async(req).await.unwrap().0
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_link_is_not_used_up_while_the_machine_is_not_linked() {
    let lab = lab().await;
    let token = lab.pair("lab").await;
    let t = ticket::mint(&lab.key, "lab", "c03ef0d9", server::unix_now(), ticket::TTL);
    // the node is not linked yet: refused, with the reason
    let mut term = open_with(&lab, &t).await;
    let (texts, _, close) = viewer(&mut term, |_, _| false).await;
    assert_eq!(close, Some(1011));
    assert!(texts[0]["error"].as_str().unwrap().contains("is not linked to the hub right now"), "{texts:?}");
    // it links a moment later: the same link opens the terminal
    let mut node = lab.node(&token).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut term = open_with(&lab, &t).await;
    let Got::Msg(ToNode::TermOpen { ch, .. }) = from_hub(&mut node).await else { panic!("expected term.open") };
    node.send(Message::Binary(link_proto::frame(ch, b"here\r\n").into())).await.unwrap();
    let (_, out, close) = viewer(&mut term, |_, o| o.contains("here")).await;
    assert!(out.contains("here"), "{close:?}");
    // ... once
    let mut again = open_with(&lab, &t).await;
    let (texts, _, close) = viewer(&mut again, |_, _| false).await;
    assert_eq!(close, Some(4401), "{texts:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_that_stopped_reading_is_unlinked() {
    let lab = lab_with(|c| c.link_patience = Duration::from_millis(500)).await;
    let token = lab.pair("lab").await;
    let mut node = lab.node(&token).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut term = lab.term("c03ef0d9").await;
    let Got::Msg(ToNode::TermOpen { ch, .. }) = from_hub(&mut node).await else { panic!("expected term.open") };
    node.send(Message::Binary(link_proto::frame(ch, b"up\r\n").into())).await.unwrap();
    viewer(&mut term, |_, o| o.contains("up")).await;
    // The machine goes to sleep with its socket open: nothing is read from here on. A viewer keeps typing, which
    // fills the connection. The hub must give the link up, not wait on it for as long as the system does.
    let typing = tokio::spawn(async move {
        let piece = vec![b'x'; 256 * 1024];
        for _ in 0..128 {
            if term.send(Message::Binary(piece.clone().into())).await.is_err() {
                break;
            }
        }
        term
    });
    let end = std::time::Instant::now() + Duration::from_secs(15);
    let mut online = true;
    while online && std::time::Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (_, v) = lab.http("GET", "/internal/nodes", "").await;
        online = v["nodes"][0]["online"] == json!(true);
    }
    assert!(!online, "still shown as linked");
    typing.abort();
    drop(node);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_send_that_waits_on_a_slow_node_does_not_hold_up_what_the_node_says() {
    let lab = lab_with(|c| c.link_patience = Duration::from_secs(30)).await;
    let token = lab.pair("lab").await;
    let mut node = lab.node(&token).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut term = lab.term("c03ef0d9").await;
    let Got::Msg(ToNode::TermOpen { ch, .. }) = from_hub(&mut node).await else { panic!("expected term.open") };
    node.send(Message::Binary(link_proto::frame(ch, b"up\r\n").into())).await.unwrap();
    viewer(&mut term, |_, o| o.contains("up")).await;
    // The node is busy and takes nothing for a while (not for good: the hub's patience is long here), while a
    // viewer's paste fills the connection, so the hub's next send to it waits.
    let typing = tokio::spawn(async move {
        let piece = vec![b'x'; 256 * 1024];
        for _ in 0..128 {
            if term.send(Message::Binary(piece.clone().into())).await.is_err() {
                break;
            }
        }
        term
    });
    tokio::time::sleep(Duration::from_secs(1)).await;
    // What the node says meanwhile is still heard: the hub reads the link while that send waits.
    let hello = FromNode::Hello { version: "9.9.10".into(), os: "linux".into(), label: "labhost".into() };
    node.send(Message::Text(link_proto::encode(&hello).into())).await.unwrap();
    let end = std::time::Instant::now() + Duration::from_secs(5);
    let mut version = String::new();
    while version != "9.9.10" && std::time::Instant::now() < end {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (_, v) = lab.http("GET", "/internal/nodes", "").await;
        assert_eq!(v["nodes"][0]["online"], true, "the link is still up");
        version = v["nodes"][0]["version"].as_str().unwrap_or("").to_string();
    }
    assert_eq!(version, "9.9.10", "the hub did not read the link while its send waited");
    typing.abort();
    drop(node);
}
