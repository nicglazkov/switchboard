use super::*;
use axum::extract::{Path as UrlPath, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// Long enough for a loaded build host: a stand-down that waits on a second dial took over 20 s once in 15 runs.
const LONG: Duration = Duration::from_secs(45);

fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!(
        "sb-node-keys-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A port nothing listens on, for a test to open later. Each one is handed out once in this process, and they
/// come from below the range the system gives to a socket that asks for "any port": every listener and every
/// outgoing connection of the other tests asks for one of those, so a port taken from that range and let go
/// could be somebody else's by the time this test listens on it (the control port then answered for another
/// test's server, or not at all). It is tried without listening on it: a program that some test starts at this
/// moment gets a copy of every open socket until it runs, and a copy of one that listens would keep the port
/// for those milliseconds.
async fn free_port() -> SocketAddr {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    const FIRST: u64 = 20_000;
    const PORTS: u64 = 10_000;
    // (another test program running at the same time starts somewhere else in the range)
    let start = u64::from(std::process::id()) * 257;
    for _ in 0..PORTS {
        let port = FIRST + (start + NEXT.fetch_add(1, Ordering::Relaxed)) % PORTS;
        let at = SocketAddr::from(([127, 0, 0, 1], port as u16));
        let probe = tokio::net::TcpSocket::new_v4().unwrap();
        #[cfg(unix)]
        probe.set_reuseaddr(true).unwrap();
        if probe.bind(at).is_ok() {
            return at;
        }
    }
    panic!("no port is free between {FIRST} and {}", FIRST + PORTS);
}

async fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = std::time::Instant::now() + LONG;
    while !f() {
        assert!(std::time::Instant::now() < end, "never happened: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

type Rpc = Box<dyn Fn(&str, &Value) -> (u16, Value) + Send + Sync>;

/// A stand-in for the hub on the home computer: the routes keys use, and a `/node` that notes who linked.
#[derive(Default)]
struct FakeHub {
    machines: Mutex<Vec<String>>,
    unpaired: Mutex<Vec<String>>,
    nodes: Mutex<Value>,
    rpc: Mutex<Option<Rpc>>,
    asked: Mutex<Vec<String>>,
    linked: Mutex<Vec<String>>,
    codes: AtomicU64,
}

async fn fake_hub() -> (Arc<FakeHub>, SocketAddr) {
    let hub = Arc::new(FakeHub::default());
    let app = Router::new()
        .route(
            "/internal/pair",
            post(|State(h): State<Arc<FakeHub>>| async move {
                Json(json!({"ok": true, "code": format!("code{}", h.codes.fetch_add(1, Ordering::Relaxed)), "expires": 1}))
            }),
        )
        .route(
            "/pair",
            post(|State(h): State<Arc<FakeHub>>, Json(v): Json<Value>| async move {
                let name = v["name"].as_str().unwrap_or("").to_string();
                let mut m = h.machines.lock().unwrap();
                if !v["code"].as_str().unwrap_or("").starts_with("code") {
                    return (axum::http::StatusCode::FORBIDDEN, Json(json!({"error": "bad code"})));
                }
                if m.contains(&name) {
                    return (axum::http::StatusCode::CONFLICT, Json(json!({"error": "That name is taken."})));
                }
                m.push(name.clone());
                (
                    axum::http::StatusCode::OK,
                    Json(json!({"ok": true, "machine": name, "label": v["label"], "token": format!("tok-{name}")})),
                )
            }),
        )
        .route(
            "/internal/nodes",
            get(|State(h): State<Arc<FakeHub>>| async move { Json(json!({"ok": true, "nodes": *h.nodes.lock().unwrap()})) }),
        )
        .route(
            "/internal/rpc/{machine}",
            post(|State(h): State<Arc<FakeHub>>, UrlPath(m): UrlPath<String>, Json(v): Json<Value>| async move {
                h.asked.lock().unwrap().push(m.clone());
                let (status, body) = match &*h.rpc.lock().unwrap() {
                    Some(f) => f(&m, &v),
                    None => (503, json!({"ok": false, "error": "not linked"})),
                };
                (axum::http::StatusCode::from_u16(status).unwrap(), Json(body))
            }),
        )
        .route(
            "/internal/unpair/{machine}",
            post(|State(h): State<Arc<FakeHub>>, UrlPath(m): UrlPath<String>| async move {
                h.machines.lock().unwrap().retain(|x| *x != m);
                h.unpaired.lock().unwrap().push(m);
                Json(json!({"ok": true}))
            }),
        )
        .route(
            "/node",
            get(|State(h): State<Arc<FakeHub>>, headers: axum::http::HeaderMap| async move {
                let auth = headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                h.linked.lock().unwrap().push(auth);
                // not a WebSocket here: the link sees a refusal and tries again, which is all this test needs
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            }),
        )
        .with_state(hub.clone());
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (hub, addr)
}

/// The server on the home computer: every request is answered with `body`.
async fn fake_server(body: &'static str) -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = l.accept().await.unwrap();
            tokio::spawn(async move {
                let mut got = vec![];
                let mut buf = [0u8; 4096];
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf).await {
                        Ok(n) if n > 0 => got.extend_from_slice(&buf[..n]),
                        _ => return,
                    }
                }
                let answer =
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(answer.as_bytes()).await;
            });
        }
    });
    addr
}

/// One GET through a port. The answer, or None when the connection was closed without one.
async fn get_through(port: SocketAddr) -> Option<String> {
    let mut c = TcpStream::connect(port).await.ok()?;
    c.write_all(b"GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:8130\r\n\r\n").await.ok()?;
    let mut answer = vec![];
    match tokio::time::timeout(LONG, c.read_to_end(&mut answer)).await {
        Ok(Ok(_)) if !answer.is_empty() => Some(String::from_utf8_lossy(&answer).into_owned()),
        Ok(_) => None,
        Err(_) => panic!("the connection was left hanging"),
    }
}

fn quick() -> Clocks {
    Clocks {
        backoff_first: Duration::from_millis(100),
        backoff_max: Duration::from_millis(400),
        dial: Duration::from_secs(5),
        ..Clocks::default()
    }
}

/// Keys for one computer whose files are all in `dir`, with ports of its own and its endpoint on loopback.
async fn computer(dir: &Path, hub: SocketAddr, server: SocketAddr) -> Arc<Shared> {
    computer_at(dir, hub, server, "127.0.0.1:0".parse().unwrap()).await
}

/// ... with its endpoint at `bind`, for a computer that has to be found again after a restart.
async fn computer_at(dir: &Path, hub: SocketAddr, server: SocketAddr, bind: SocketAddr) -> Arc<Shared> {
    let places = Places {
        config: dir.join("node.json"),
        key: dir.join("device.key"),
        lock: dir.join("keys.lock"),
        paired: dir.join("paired.json"),
        server_json: dir.join("server.json"),
        roster: dir.join("roster.json"),
        blocked: dir.join("blocked.json"),
        roster_state: dir.join("roster-state.json"),
    };
    let ports = Ports {
        control: free_port().await,
        api_target: server,
        hub_target: hub,
        api_forward: Some(free_port().await),
        hub_forward: Some(free_port().await),
        hub,
        bind: Some(bind),
        away_after: Duration::from_millis(600),
    };
    Shared::new(places, ports, None, "").with_clocks(quick())
}

fn linked(dir: &Path, machine: &str) -> NodeConfig {
    let cfg = NodeConfig {
        hub: "127.0.0.1:9".into(),
        machine: machine.into(),
        token: "old-token".into(),
        ..Default::default()
    };
    crate::save_to(&dir.join("node.json"), &cfg).unwrap();
    cfg
}

fn keys_on(dir: &Path) {
    std::fs::write(dir.join("server.json"), br#"{"keys": "both"}"#).unwrap();
}

async fn control(k: &Shared, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    crate::http_json(&k.ports.control.to_string(), method, path, body.as_ref(), LONG).await.unwrap()
}

#[test]
fn which_computer_is_what() {
    let home = HomeConfig { id: DeviceId([7; 32]), relay: None, name: "studio".into(), addrs: vec![] };
    let plain = NodeConfig { hub: "h".into(), machine: "m".into(), token: "t".into(), ..Default::default() };
    let paired = NodeConfig { home: Some(home.clone()), ..plain.clone() };
    assert_eq!(role_of(Mode::Off, None), Role::Off);
    assert_eq!(role_of(Mode::Off, Some(&plain)), Role::Off);
    assert_eq!(role_of(Mode::Off, Some(&paired)), Role::Member(home));
    assert_eq!(role_of(Mode::Both, None), Role::Home);
    assert_eq!(role_of(Mode::Only, Some(&plain)), Role::Home);
    assert_eq!(role_of(Mode::Both, Some(&paired)), Role::Home, "the server's settings win");
}

#[test]
fn the_servers_settings_are_read_as_the_hub_reads_them() {
    let d = scratch("mode");
    let p = d.join("server.json");
    assert_eq!(read_mode(&p), Mode::Off, "no file");
    for (text, mode) in [
        (r#"{}"#, Mode::Off),
        (r#"{"keys": "off"}"#, Mode::Off),
        (r#"{"keys": "both"}"#, Mode::Both),
        (r#"{"keys": "only"}"#, Mode::Only),
        (r#"{"keys": "onyl"}"#, Mode::Only),
        (r#"{"keys": 1}"#, Mode::Only),
        ("", Mode::Only),
    ] {
        std::fs::write(&p, text).unwrap();
        assert_eq!(read_mode(&p), mode, "{text}");
    }
}

#[test]
fn every_address_can_be_moved_and_none_off_this_computer() {
    let none = |_: &str| None::<String>;
    let p = Ports::new(&none).unwrap();
    assert_eq!(p.control, local(8137));
    assert_eq!((p.api_target, p.hub_target), (local(8130), local(8136)));
    assert_eq!((p.api_forward, p.hub_forward), (Some(local(8130)), Some(local(8132))));
    assert_eq!((p.hub, p.bind), (local(8132), None));

    let set = |k: &str| match k {
        "SWITCHBOARD_NODE_CONTROL" => Some("http://127.0.0.1:9137/".to_string()),
        "SWITCHBOARD_NODE_API_TARGET" => Some("127.0.0.1:9130".to_string()),
        "SWITCHBOARD_NODE_HUB_TARGET" => Some("127.0.0.1:9136".to_string()),
        "SWITCHBOARD_NODE_API_FORWARD" => Some("off".to_string()),
        "SWITCHBOARD_NODE_HUB_FORWARD" => Some("[::1]:9132".to_string()),
        "SWITCHBOARD_NODE_HUB" => Some("127.0.0.1:9132".to_string()),
        "SWITCHBOARD_NODE_BIND" => Some("192.0.2.1:4433".to_string()),
        _ => None,
    };
    let p = Ports::new(&set).unwrap();
    assert_eq!((p.control, p.api_target, p.hub_target), (local(9137), local(9130), local(9136)));
    assert_eq!((p.api_forward, p.hub_forward), (None, Some("[::1]:9132".parse().unwrap())));
    assert_eq!((p.hub, p.bind), (local(9132), Some("192.0.2.1:4433".parse().unwrap())));

    for name in ["SWITCHBOARD_NODE_CONTROL", "SWITCHBOARD_NODE_API_TARGET", "SWITCHBOARD_NODE_HUB_FORWARD"] {
        let off = |k: &str| (k == name).then(|| "192.0.2.1:8137".to_string());
        assert!(Ports::new(&off).unwrap_err().contains("not on this computer"), "{name}");
    }
    // a tunnel never leads to the hub's main port, which answers this computer's own requests
    let main = |k: &str| (k == "SWITCHBOARD_NODE_HUB_TARGET").then(|| "127.0.0.1:8132".to_string());
    assert!(Ports::new(&main).unwrap_err().contains("tunnel listener"));
    let same = |k: &str| k.starts_with("SWITCHBOARD_NODE_HUB").then(|| "127.0.0.1:9000".to_string());
    assert!(Ports::new(&same).is_err());
}

#[test]
fn the_files_sit_next_to_node_json_and_can_be_moved() {
    let none = |_: &str| None::<String>;
    let p = Places::new(Path::new("/h/.config/switchboard/node.json"), Path::new("/h"), &none);
    assert_eq!(p.key, Path::new("/h/.config/switchboard/device.key"));
    assert_eq!(p.lock, Path::new("/h/.config/switchboard/keys.lock"));
    assert_eq!(p.paired, Path::new("/h/.local/share/switchboard/paired.json"));
    assert_eq!(p.server_json, Path::new("/h/.config/switchboard/server.json"));
    let set = |k: &str| match k {
        "SWITCHBOARD_PAIRED" => Some("/s/paired.json".to_string()),
        "SWITCHBOARD_SERVER_JSON" => Some("/s/server.json".to_string()),
        _ => None,
    };
    let p = Places::new(Path::new("/s/node.json"), Path::new("/h"), &set);
    assert_eq!((p.key.as_path(), p.paired.as_path()), (Path::new("/s/device.key"), Path::new("/s/paired.json")));
    assert_eq!(p.server_json, Path::new("/s/server.json"));
    // the two folders a server beside the installed one is given move the list and the settings with it
    let folders = |k: &str| match k {
        "SWITCHBOARD_CONFIG_DIR" => Some("/t/cfg".to_string()),
        "SWITCHBOARD_STATE_DIR" => Some("/t/state".to_string()),
        _ => None,
    };
    let p = Places::new(Path::new("/t/cfg/node.json"), Path::new("/h"), &folders);
    assert_eq!(p.paired, Path::new("/t/state/paired.json"));
    assert_eq!(p.server_json, Path::new("/t/cfg/server.json"));
    // and the list of devices with them: the signed list and the blocks beside node.json, its two numbers beside
    // the paired list
    assert_eq!(p.roster, Path::new("/t/cfg/roster.json"));
    assert_eq!(p.blocked, Path::new("/t/cfg/blocked.json"));
    assert_eq!(p.roster_state, Path::new("/t/state/roster-state.json"));
    let both = |k: &str| folders(k).or_else(|| set(k));
    let p = Places::new(Path::new("/t/cfg/node.json"), Path::new("/h"), &both);
    assert_eq!(
        (p.paired.as_path(), p.server_json.as_path()),
        (Path::new("/s/paired.json"), Path::new("/s/server.json"))
    );
}

#[test]
fn a_removed_computer_is_told_it_was_removed_and_not_that_its_key_is_in_use_twice() {
    assert_eq!(same_key_line(false, false), None);
    assert!(same_key_line(true, false).is_some_and(|l| l.starts_with("Another computer is using this computer's key")));
    assert_eq!(same_key_line(true, true), None, "the home computer no longer lists it: that line alone is said");
    assert_eq!(same_key_line(false, true), None);
}

#[test]
fn a_mac_that_blocks_the_local_network_is_recognised() {
    assert!(local_network_off("macos", "Failed to connect: No route to host (os error 65)"));
    assert!(!local_network_off("linux", "No route to host (os error 113)"));
    assert!(!local_network_off("macos", "connection refused"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn nothing_happens_until_keys_are_turned_on() {
    let (_hub, hub_addr) = fake_hub().await;
    let d = scratch("inert");
    let k = computer(&d, hub_addr, hub_addr).await;
    let quiet = |k: &Arc<Shared>| {
        let names: Vec<String> =
            std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| n.starts_with("node.json")), "only node.json and its copy: {names:?}");
        assert!(k.tunnel().is_none() && k.status().is_err());
        assert_eq!(k.role(), Role::Off);
    };
    // not paired with anything
    k.tick().await;
    quiet(&k);
    // linked to a hub by address, as every computer is today
    let cfg = linked(&d, "lab");
    k.tick().await;
    k.tick().await;
    quiet(&k);
    // the one thing that is there while keys are off: the control port, which says so to this computer
    let (status, v) = control(&k, "GET", "/status", None).await;
    assert_eq!((status, v["ok"].as_bool(), v["kind"].as_str()), (503, Some(false), Some("off")), "{v}");
    assert_eq!(v["error"], "Keys are off on this computer.");
    for path in ["/pair/new", "/devices/remove"] {
        let (status, v) = control(&k, "POST", path, Some(json!({"id": DeviceKey::generate().id().to_string()}))).await;
        assert_eq!((status, v["ok"].as_bool()), (409, Some(false)), "{path}: {v}");
    }
    let (status, v) = control(&k, "POST", "/pair/new", Some(json!({}))).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("off")), "{v}");
    quiet(&k);
    for port in [k.ports.api_forward.unwrap(), k.ports.hub_forward.unwrap()] {
        assert!(TcpStream::connect(port).await.is_err(), "no forwarder");
    }
    assert!(matches!(crate::link::Dialer::pick(&cfg, Some(&k)), crate::link::Dialer::Tcp), "the link goes by address");
    // and with the server's settings saying "off" in so many words
    std::fs::write(d.join("server.json"), br#"{"keys": "off", "theme": "dark"}"#).unwrap();
    k.tick().await;
    assert_eq!(k.role(), Role::Off);
    assert!(!d.join("device.key").exists() && !d.join("keys.lock").exists() && !d.join("paired.json").exists());
    // the old node.json still reads exactly as it did
    let back = crate::load_from(&d.join("node.json")).unwrap();
    assert_eq!(
        (back.hub, back.machine, back.token, back.home, back.relay),
        (cfg.hub, cfg.machine, cfg.token, None, None)
    );
    let text = std::fs::read_to_string(d.join("node.json")).unwrap();
    assert!(!text.contains("home") && !text.contains("relay"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_computer_pairs_with_a_link_reaches_home_and_is_cut_off_when_removed() {
    let (hub, hub_addr) = fake_hub().await;
    let server = fake_server("hello from home").await;
    let (hd, md) = (scratch("home"), scratch("member"));
    let home = computer(&hd, hub_addr, server).await;
    keys_on(&hd);
    home.tick().await;

    // home: on, with a key, a control port that answers this computer and no browser
    let (status, v) = control(&home, "GET", "/status", None).await;
    assert_eq!((status, v["ok"].as_bool(), v["role"].as_str()), (200, Some(true), Some("home")), "{v}");
    assert!(v["machine"].is_null() && v["devices"] == 0, "{v}");
    assert!(hd.join("device.key").exists() && hd.join("keys.lock").exists());
    let mut c = TcpStream::connect(home.ports.control).await.unwrap();
    let req = format!(
        "GET /status HTTP/1.1\r\nHost: {}\r\nOrigin: http://evil.example\r\nConnection: close\r\n\r\n",
        home.ports.control
    );
    c.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    c.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 403"), "{out}");
    let (status, _) = control(&home, "GET", "/nothing", None).await;
    assert_eq!(status, 404);

    // a link (no relay here, so no short code)
    let (status, v) = control(&home, "POST", "/pair/new", Some(json!({}))).await;
    assert_eq!(status, 200, "{v}");
    assert!(v["code"].is_null() && v["expires"].as_u64().unwrap() > 0, "{v}");
    let link = v["link"].as_str().unwrap().to_string();
    assert!(link.starts_with("switchboard://pair/1?"), "{link}");

    // the other computer joins with it
    let member = computer(&md, hub_addr, server).await;
    let no_address = member.join(&link, None, vec![], false).await.unwrap_err();
    assert!(no_address.contains("--addr"), "{no_address}");
    // a pairing that failed leaves no key behind on a computer that had none
    assert!(!md.join("device.key").exists() && !md.join("device.key.bak").exists(), "{:?}", std::fs::read_dir(&md));
    let welcome = member.join(&link, Some("Laptop".into()), home.addrs(), false).await.unwrap();
    assert_eq!((welcome.machine.as_deref(), welcome.token.as_deref()), (Some("laptop"), Some("tok-laptop")));
    assert_eq!(*hub.machines.lock().unwrap(), vec!["laptop".to_string()]);
    let cfg = crate::load_from(&md.join("node.json")).unwrap();
    assert_eq!((cfg.hub.as_str(), cfg.machine.as_str(), cfg.token.as_str()), ("", "laptop", "tok-laptop"));
    assert_eq!(cfg.home.as_ref().unwrap().id.to_string(), v_id(&home));
    let used = member.join(&link, None, home.addrs(), false).await.unwrap_err();
    assert!(used.contains("expired or was already used"), "a link works once: {used}");
    // ... and a failed try on a computer that is paired keeps the key it is paired with
    assert!(md.join("device.key").exists() && md.join("device.key.bak").exists());

    // its helper picks that up: a tunnel, two ports, and the hub link through the tunnel
    member.tick().await;
    until("the tunnel is up", || member.tunnel().is_some_and(|t| t.status().connected)).await;
    let answer = get_through(member.ports.api_forward.unwrap()).await.expect("an answer from home's server");
    assert!(answer.ends_with("hello from home"), "{answer}");
    let s = member.status().unwrap();
    assert_eq!((s.role.as_str(), s.machine.as_deref()), ("member", Some("laptop")));
    let h = s.home.unwrap();
    assert!(h.connected && h.path.as_deref() == Some("direct") && !h.refused, "{h:?}");
    assert_eq!(
        s.forwarders.iter().map(|f| (f.target.as_str(), f.ok)).collect::<Vec<_>>(),
        vec![("api", true), ("hub", true)]
    );
    assert!(s.problems.is_empty(), "{:?}", s.problems);
    assert!(matches!(crate::link::Dialer::pick(&cfg, Some(&member)), crate::link::Dialer::Iroh(_)));
    let (m2, lock, c2) = (member.clone(), md.join("node.lock"), cfg.clone());
    let link_task = tokio::spawn(async move { crate::link::run(c2, None, &lock, false, Some(&m2)).await });
    until("the hub sees the link", || hub.linked.lock().unwrap().iter().any(|a| a == "Bearer tok-laptop")).await;
    link_task.abort();
    member.tick().await; // (remembers where home was reached)
    assert!(!crate::load_raw(&md.join("node.json")).unwrap().home.unwrap().addrs.is_empty());

    // home lists it, connected
    let (_, v) = control(&home, "GET", "/devices", None).await;
    let d = &v["devices"][0];
    assert_eq!(
        (d["name"].as_str(), d["machine"].as_str(), d["kind"].as_str(), d["via"].as_str()),
        (Some("Laptop"), Some("laptop"), Some("computer"), Some("link"))
    );
    assert_eq!(d["path"], "direct", "{d}");
    assert!(d["last_seen"].as_u64().is_some() && d["paired"].as_u64().is_some(), "{d}");
    // a connection that is up counts as seen now, not when it last opened a stream
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    assert!(now - d["last_seen"].as_u64().unwrap() <= 1, "{d}");
    let id = d["id"].as_str().unwrap().to_string();

    // removed: cut off at once, gone from the hub, and told so
    let (status, v) = control(&home, "POST", "/devices/remove", Some(json!({"id": "nonsense"}))).await;
    assert_eq!(status, 400, "{v}");
    let (status, v) = control(&home, "POST", "/devices/remove", Some(json!({"id": id}))).await;
    assert_eq!((status, v["ok"].as_bool()), (200, Some(true)), "{v}");
    assert_eq!(*hub.unpaired.lock().unwrap(), vec!["laptop".to_string()]);
    until("the member hears it", || member.status().is_ok_and(|s| s.home.is_some_and(|h| h.refused))).await;
    assert_eq!(get_through(member.ports.api_forward.unwrap()).await, None);
    let s = member.status().unwrap();
    assert!(s.problems.iter().any(|p| p.contains("no longer lists this computer")), "{:?}", s.problems);
    let (status, _) = control(&home, "POST", "/devices/remove", Some(json!({"id": id}))).await;
    assert_eq!(status, 404);
    let (_, v) = control(&home, "GET", "/devices", None).await;
    assert_eq!(v["devices"], json!([]));

    // keys turned off again at home: the endpoint and the key's lock go, and the control port says keys are off
    std::fs::write(hd.join("server.json"), br#"{"keys": "off"}"#).unwrap();
    home.tick().await;
    assert!(home.status().is_err());
    assert!(!home.held_elsewhere());
    let (status, v) = control(&home, "GET", "/status", None).await;
    assert_eq!((status, v["kind"].as_str()), (503, Some("off")), "{v}");
    member.shutdown().await;
    home.shutdown().await;
    until("the control port closes", || std::net::TcpStream::connect(home.ports.control).is_err()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_computer_with_keys_off_joins_through_its_helper_and_every_refusal_has_a_kind() {
    let (_hub, hub_addr) = fake_hub().await;
    let server = fake_server("hello from home").await;
    let (hd, md) = (scratch("home-kinds"), scratch("member-kinds"));
    let home = computer(&hd, hub_addr, server).await;
    keys_on(&hd);
    home.tick().await;
    let member = computer(&md, hub_addr, server).await;
    member.tick().await;
    let files = || -> Vec<String> {
        std::fs::read_dir(&md).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()
    };
    assert!(files().is_empty(), "nothing is written while keys are off: {:?}", files());
    let join = |body: Value| {
        let m = member.clone();
        async move { control(&m, "POST", "/pair/join", Some(body)).await }
    };
    let said = |v: &Value| (v["ok"].as_bool(), v["kind"].as_str().map(str::to_string), v["error"].as_str().is_some());

    // what is typed is checked first
    let (status, v) = join(json!({})).await;
    assert_eq!((status, said(&v)), (400, (Some(false), Some("bad_input".into()), true)), "{v}");
    let (status, v) = join(json!({"input": "not a code"})).await;
    assert_eq!((status, said(&v)), (400, (Some(false), Some("bad_input".into()), true)), "{v}");
    let (status, v) = join(json!({"input": "ABCD-EFGH", "addrs": ["nowhere"]})).await;
    assert_eq!((status, said(&v)), (400, (Some(false), Some("bad_input".into()), true)), "{v}");
    // a short code needs a relay, and a link that names none needs an address
    let (status, v) = join(json!({"input": "ABCD-EFGH"})).await;
    assert_eq!((status, said(&v)), (409, (Some(false), Some("no_relay".into()), true)), "{v}");
    let link = home.pair_new().await.unwrap().link;
    let (status, v) = join(json!({"input": link})).await;
    assert_eq!((status, said(&v)), (409, (Some(false), Some("link_no_relay".into()), true)), "{v}");
    // a home computer nobody answers at
    let (status, v) = join(json!({"input": link, "addrs": ["127.0.0.1:9"]})).await;
    assert_eq!((status, said(&v)), (409, (Some(false), Some("unreachable".into()), true)), "{v}");
    // none of those left a key, an endpoint or a port behind
    assert!(!md.join("device.key").exists() && !md.join("device.key.bak").exists(), "{:?}", files());
    member.tick().await;
    assert!(member.tunnel().is_none() && member.status().is_err() && !member.held_elsewhere());
    for port in [member.ports.api_forward.unwrap(), member.ports.hub_forward.unwrap()] {
        assert!(TcpStream::connect(port).await.is_err(), "no forwarder");
    }

    // the real thing: it pairs, and at the next look it is a member
    let addrs: Vec<String> = home.addrs().iter().map(|a| a.to_string()).collect();
    let (status, v) = join(json!({"input": link, "name": "Laptop", "addrs": addrs})).await;
    assert_eq!((status, v["ok"].as_bool(), v["machine"].as_str()), (200, Some(true), Some("laptop")), "{v}");
    assert_eq!(v["home"]["id"].as_str().map(str::to_string), Some(v_id(&home)));
    member.tick().await;
    until("the tunnel is up", || member.tunnel().is_some_and(|t| t.status().connected)).await;
    let (status, v) = control(&member, "GET", "/status", None).await;
    assert_eq!((status, v["role"].as_str()), (200, Some("member")), "{v}");
    // a link works once; and only the home computer hands out codes
    let (status, v) = join(json!({"input": link, "addrs": addrs})).await;
    assert_eq!((status, said(&v)), (409, (Some(false), Some("expired".into()), true)), "{v}");
    member.tick().await;
    let (status, v) = control(&member, "POST", "/pair/new", Some(json!({}))).await;
    assert_eq!((status, said(&v)), (409, (Some(false), Some("not_home".into()), true)), "{v}");
    let (status, v) = control(&home, "POST", "/pair/join", Some(json!({"input": link}))).await;
    assert_eq!((status, said(&v)), (409, (Some(false), Some("is_home".into()), true)), "{v}");
    member.shutdown().await;
    home.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_computer_that_has_a_home_is_not_moved_to_another_unless_told_to_replace_it() {
    let (hub_a, hub_a_addr) = fake_hub().await;
    let (hub_b, hub_b_addr) = fake_hub().await;
    let server = fake_server("x").await;
    let (ad, bd, md) = (scratch("home-a"), scratch("home-b"), scratch("mover"));
    let (a, b) = (computer(&ad, hub_a_addr, server).await, computer(&bd, hub_b_addr, server).await);
    for (k, d) in [(&a, &ad), (&b, &bd)] {
        keys_on(d);
        k.tick().await;
    }
    let member = computer(&md, hub_a_addr, server).await;
    let link_of = |k: Arc<Shared>| async move { k.pair_new().await.unwrap().link };
    member.join(&link_of(a.clone()).await, Some("Laptop".into()), a.addrs(), false).await.unwrap();
    member.tick().await;
    until("it reaches its home", || member.tunnel().is_some_and(|t| t.status().connected)).await;
    until("it holds its home's list", || seq_of(&member) > 0).await;
    let before = std::fs::read(md.join("node.json")).unwrap();
    let home_of = |d: &Path| crate::load_raw(&d.join("node.json")).unwrap().home.unwrap().id.to_string();
    assert_eq!(home_of(&md), v_id(&a));

    // a link from another home computer: turned down before anything is stopped, and nothing changes anywhere
    let link_b = link_of(b.clone()).await;
    let addrs_b: Vec<String> = b.addrs().iter().map(|x| x.to_string()).collect();
    let (status, v) = control(&member, "POST", "/pair/join", Some(json!({"input": link_b, "addrs": addrs_b}))).await;
    assert_eq!((status, v["ok"].as_bool(), v["kind"].as_str()), (409, Some(false), Some("other_home")), "{v}");
    let said = v["error"].as_str().unwrap();
    assert!(said.starts_with("This computer is paired with the home computer "), "{said}");
    assert!(said.contains("nothing was changed") && said.contains("--replace") && said.contains("Replace"), "{said}");
    assert_eq!(std::fs::read(md.join("node.json")).unwrap(), before);
    assert!(member.tunnel().is_some_and(|t| t.status().connected), "its tunnel was left alone");
    assert!(b.devices().unwrap().is_empty() && hub_b.machines.lock().unwrap().is_empty());
    assert_eq!(a.devices().unwrap().len(), 1);

    // a link from its own home computer needs no such word: pairing again is the way back in
    let again = link_of(a.clone()).await;
    let w = member.join(&again, Some("Laptop".into()), a.addrs(), false).await.unwrap();
    assert_eq!((w.home_id.to_string(), w.machine.as_deref()), (v_id(&a), Some("laptop")));

    // told to replace: it moves, the old home computer's list and blocks do not come along, and the link that
    // was turned down still works
    std::fs::write(md.join("blocked.json"), "{}").unwrap();
    let body = json!({"input": link_b, "name": "Laptop", "addrs": addrs_b, "replace": true});
    member.tick().await;
    let (status, v) = control(&member, "POST", "/pair/join", Some(body)).await;
    assert_eq!((status, v["ok"].as_bool()), (200, Some(true)), "{v}");
    assert_eq!(v["home"]["id"].as_str().map(str::to_string), Some(v_id(&b)));
    assert_eq!(home_of(&md), v_id(&b));
    assert!(!md.join("roster.json").exists() && !md.join("blocked.json").exists());
    assert_eq!(b.devices().unwrap().len(), 1);
    assert_eq!(*hub_b.machines.lock().unwrap(), vec!["laptop".to_string()]);
    member.tick().await;
    until("it reaches its new home", || member.tunnel().is_some_and(|t| t.status().connected)).await;
    until("and holds the new home's list", || member.roster().is_some_and(|r| r.body.home.to_string() == v_id(&b)))
        .await;
    let _ = hub_a;
    member.shutdown().await;
    a.shutdown().await;
    b.shutdown().await;
}

#[test]
fn a_code_from_another_home_is_turned_down_in_this_computers_words() {
    // (a code does not say whose it is: the home computer it leads to turns the computer down, sb-link
    // pairing.rs, and this is the sentence the person then reads)
    let old = HomeConfig { id: DeviceId([7; 32]), relay: None, name: "Studio".into(), addrs: vec![] };
    let no = other_home(&old);
    assert_eq!(no.kind, "other_home");
    assert_eq!(
        no.text,
        "This computer is paired with the home computer Studio already, so nothing was changed. To pair it with \
         another home computer instead, run the same command again with --replace at the end, or choose Replace in \
         the app."
    );
    let unnamed = other_home(&HomeConfig { name: " ".into(), ..old.clone() });
    assert!(unnamed.text.contains(&format!("the home computer {} already", old.id.short())), "{}", unnamed.text);
}

#[test]
fn every_way_pairing_fails_has_a_kind() {
    use sb_link::pairing::PairError;
    let io = || std::io::Error::other("x");
    for (e, kind) in [
        (PairError::WrongCode, "wrong_code"),
        (PairError::Gone, "expired"),
        (PairError::Busy, "busy"),
        (PairError::Refused("no".into()), "refused"),
        (PairError::WrongHome, "wrong_home"),
        (PairError::OtherHome, "other_home"),
        (PairError::Version(9), "version"),
        (PairError::Protocol("x"), "protocol"),
        (PairError::Timeout, "timeout"),
        (PairError::Io(io()), "connection_lost"),
    ] {
        assert_eq!(e.kind(), kind, "{}", e.message());
    }
}

#[test]
fn removing_a_device_is_one_line_in_the_log() {
    let key = DeviceKey::generate();
    let id = key.id();
    let d = Device {
        id,
        name: "Laptop".into(),
        kind: Kind::Computer,
        machine: Some("laptop".into()),
        paired: 0,
        via: sb_link::paired::Via::Link,
    };
    assert_eq!(
        removed_line(Some(&d), &id),
        format!("keys: removed Laptop ({}), the computer laptop: it is refused from now on", id.short())
    );
    let phone = Device { name: String::new(), machine: None, kind: Kind::Phone, ..d };
    assert_eq!(
        removed_line(Some(&phone), &id),
        format!("keys: removed a device ({}): it is refused from now on", id.short())
    );
    assert_eq!(removed_line(None, &id), format!("keys: removed a device ({}): it is refused from now on", id.short()));
}

fn v_id(k: &Shared) -> String {
    k.status().unwrap().id.to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_computer_that_pairs_again_keeps_its_name_and_a_taken_name_gets_a_number() {
    let (hub, hub_addr) = fake_hub().await;
    let server = fake_server("hi").await;
    let (hd, ad, bd) = (scratch("home"), scratch("a"), scratch("b"));
    let home = computer(&hd, hub_addr, server).await;
    keys_on(&hd);
    home.tick().await;
    let link = || async { home.pair_new().await.unwrap().link };
    let (a, b) = (computer(&ad, hub_addr, server).await, computer(&bd, hub_addr, server).await);
    assert_eq!(
        a.join(&link().await, Some("Desk".into()), home.addrs(), false).await.unwrap().machine.as_deref(),
        Some("desk")
    );
    assert_eq!(
        b.join(&link().await, Some("Desk".into()), home.addrs(), false).await.unwrap().machine.as_deref(),
        Some("desk2")
    );
    // the first one again: its old link at the hub is taken away and it is "desk" still
    assert_eq!(
        a.join(&link().await, Some("Other".into()), home.addrs(), false).await.unwrap().machine.as_deref(),
        Some("desk")
    );
    assert_eq!(*hub.unpaired.lock().unwrap(), vec!["desk".to_string()]);
    assert_eq!(home.devices().unwrap().len(), 2);
    // the home computer itself pairs with nobody
    let err = home.join(&link().await, None, vec![], false).await.unwrap_err();
    assert!(err.contains("This is the home computer"), "{err}");
    // and a short code is no use where no relay is set
    let err = a.join("ABCD-EFGH", None, vec![], false).await.unwrap_err();
    assert!(err.contains("relay"), "{err}");
    assert!(a.join("not a code", None, vec![], false).await.unwrap_err().contains("not a pairing code or link"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn computers_the_hub_already_lists_are_asked_once_to_switch() {
    let (hub, hub_addr) = fake_hub().await;
    let server = fake_server("hello from home").await;
    let (hd, nd) = (scratch("home"), scratch("new"));
    let home = computer(&hd, hub_addr, server).await;
    keys_on(&hd);
    home.tick().await;
    let newer = computer(&nd, hub_addr, server).await;
    linked(&nd, "new");
    newer.tick().await;
    assert!(newer.status().is_err(), "off until home asks");

    *hub.nodes.lock().unwrap() = json!([
        {"name": "old", "label": "Old PC", "online": true, "version": "0.8.1"},
        {"name": "new", "label": "New PC", "online": true, "version": crate::VERSION},
        {"name": "away", "label": "Away", "online": false, "version": "0.8.1"},
        {"name": "busy", "label": "Busy", "online": true, "version": "0.8.1"},
    ]);
    let answering = newer.clone();
    *hub.rpc.lock().unwrap() = Some(Box::new(move |machine, req| match machine {
        "new" => {
            assert_eq!(req["verb"], "node.keys");
            (200, answering.node_keys(req))
        }
        "busy" => (504, json!({"ok": false, "error": "timed out"})),
        // what an older helper's sb.py says to a verb it does not know
        _ => (200, json!({"ok": false, "error": "unknown verb: node.keys"})),
    }));
    home.walk_now().await;
    assert_eq!(*hub.asked.lock().unwrap(), vec!["old".to_string(), "new".to_string(), "busy".to_string()]);

    let devices = home.devices().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(
        (devices[0].name.as_str(), devices[0].machine.as_deref(), devices[0].via),
        ("New PC", Some("new"), Via::Upgrade)
    );
    let s = home.status().unwrap();
    assert_eq!(
        s.updates.iter().map(|u| (u.machine.as_str(), u.state.as_str())).collect::<Vec<_>>(),
        vec![("new", "paired"), ("old", "needs update")]
    );
    assert!(s.problems.iter().any(|p| p.contains("old needs an update")), "{:?}", s.problems);

    // asked once: only the one that could not answer is asked again
    home.walk_now().await;
    assert_eq!(hub.asked.lock().unwrap()[3..], ["busy".to_string()]);

    // the new one kept its hub link settings, noted home, and now reaches it by key
    let cfg = crate::load_from(&nd.join("node.json")).unwrap();
    assert_eq!((cfg.hub.as_str(), cfg.machine.as_str(), cfg.token.as_str()), ("127.0.0.1:9", "new", "old-token"));
    assert_eq!(cfg.home.as_ref().unwrap().id, home.status().unwrap().id);
    newer.tick().await;
    until("the tunnel is up", || newer.tunnel().is_some_and(|t| t.status().connected)).await;
    assert!(get_through(newer.ports.api_forward.unwrap()).await.unwrap().ends_with("hello from home"));

    // another home asking is turned down, and home itself answers that it is home
    let other =
        json!({"verb": "node.keys", "home": {"id": DeviceKey::generate().id().to_string(), "name": "x", "addrs": []}});
    let no = newer.node_keys(&other);
    assert!(no["ok"] == false && no["error"].as_str().unwrap().contains("another home computer"), "{no}");
    assert_eq!(home.node_keys(&other)["is_home"], true);
    assert_eq!(newer.node_keys(&json!({"verb": "node.keys"}))["ok"], false);
    newer.shutdown().await;
    home.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_port_that_is_taken_is_said_and_everything_else_runs() {
    let (_hub, hub_addr) = fake_hub().await;
    let d = scratch("taken");
    let k = computer(&d, hub_addr, hub_addr).await;
    let holder = TcpListener::bind(k.ports.api_forward.unwrap()).await.unwrap();
    let home = HomeConfig {
        id: DeviceKey::generate().id(),
        relay: None,
        name: "studio".into(),
        addrs: vec!["127.0.0.1:9".parse().unwrap()],
    };
    let cfg = NodeConfig { machine: "lab".into(), token: "t".into(), home: Some(home), ..Default::default() };
    crate::save_to(&d.join("node.json"), &cfg).unwrap();
    k.tick().await;
    let s = k.status().unwrap();
    assert_eq!(
        s.forwarders.iter().map(|f| (f.target.as_str(), f.ok)).collect::<Vec<_>>(),
        vec![("api", false), ("hub", true)]
    );
    let port = holder.local_addr().unwrap().to_string();
    assert!(
        s.problems.iter().any(|p| p.contains(&port) && p.contains("in use by another program")),
        "{:?}",
        s.problems
    );
    assert!(TcpStream::connect(k.ports.hub_forward.unwrap()).await.is_ok(), "the other port is open");
    assert!(
        matches!(crate::link::Dialer::pick(&cfg, Some(&k)), crate::link::Dialer::Iroh(_)),
        "no address to fall back to"
    );

    // a second helper on this computer leaves the key alone
    let twin = Shared::new(k.places.clone(), Ports { control: free_port().await, ..k.ports.clone() }, None, "");
    twin.tick().await;
    let why = twin.status().unwrap_err();
    assert!(why.contains("already using this computer's key"), "{why}");
    assert!(twin.held_elsewhere() && !k.held_elsewhere());
    // It opened no control port. Asked of the twin itself, which keeps the one listener it ever opens: a
    // connection to that port says nothing either way, since the port was only free when it was picked and any
    // other program (another test, most often) may be listening there by now.
    assert!(twin.control.lock().unwrap().is_none(), "the twin listens for the app although it leaves keys alone");
    k.shutdown().await;
    // (asked more than once: a program some other test starts at this moment holds a copy of the first one's
    // open files until it runs)
    let end = std::time::Instant::now() + LONG;
    loop {
        twin.tick().await;
        if twin.status().is_ok() {
            break;
        }
        assert!(std::time::Instant::now() < end, "and takes over when the first is gone");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    twin.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_relay_is_set_through_the_helper_and_checked() {
    let (_hub, hub_addr) = fake_hub().await;
    let d = scratch("relay");
    let k = computer(&d, hub_addr, hub_addr).await;
    keys_on(&d);
    k.tick().await;
    let s = k.status().unwrap();
    assert_eq!((s.relay.url.as_deref(), s.relay.source.as_str()), (None, "none"));
    assert!(s.problems.iter().any(|p| p.contains("No relay is set")), "{:?}", s.problems);
    let id = s.id;

    let (status, v) = control(&k, "POST", "/relay", Some(json!({"url": "ftp://nope"}))).await;
    assert_eq!(status, 400, "{v}");
    // (a relay nobody answers at: this only checks that the setting is taken and the endpoint starts again)
    let (status, _) = control(&k, "POST", "/relay", Some(json!({"url": "http://127.0.0.1:9"}))).await;
    assert_eq!(status, 200);
    assert_eq!(crate::load_raw(&d.join("node.json")).unwrap().relay.as_deref(), Some("http://127.0.0.1:9"));
    assert!(crate::load_from(&d.join("node.json")).is_err(), "a file with only a relay links nothing");
    k.tick().await;
    let s = k.status().unwrap();
    assert_eq!(
        (s.relay.url.as_deref(), s.relay.source.as_str(), s.relay.connected),
        (Some("http://127.0.0.1:9"), "settings", false)
    );
    assert_eq!(s.id, id, "the same key");
    let (status, _) = control(&k, "POST", "/relay", Some(json!({"url": ""}))).await;
    assert_eq!(status, 200);
    k.tick().await;
    assert_eq!(k.status().unwrap().relay.url, None);
    k.shutdown().await;
}

// ---- the signed list of devices (keys phase 4a) ---------------------------------------------------------------

fn seq_of(k: &Shared) -> u64 {
    k.roster().map_or(0, |r| r.body.seq)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn home_signs_its_list_at_every_change_and_the_other_computers_take_it() {
    let (hub, hub_addr) = fake_hub().await;
    let server = fake_server("hello from home").await;
    let (hd, md) = (scratch("home"), scratch("member"));
    // What the hub link carries to each computer: the laptop's helper answers for itself, the tower's is an older
    // one that hands the request to its script, which knows no such verb.
    let pushed: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let laptop: Arc<Mutex<Option<Arc<Shared>>>> = Arc::default();
    let tower = DeviceKey::generate().id();
    {
        let (pushed, laptop) = (pushed.clone(), laptop.clone());
        *hub.rpc.lock().unwrap() = Some(Box::new(move |m, req| match req["verb"].as_str() {
            Some("node.keys") if m == "tower" => {
                (200, json!({"ok": true, "id": tower.to_string(), "name": "Tower", "os": "linux", "version": "0.8.0"}))
            }
            Some("node.roster") => {
                pushed.lock().unwrap().push((m.to_string(), req["roster"].clone()));
                match (m, laptop.lock().unwrap().clone()) {
                    ("laptop", Some(k)) => (200, crate::link::answered_here(Some(&k), req).unwrap()),
                    ("tower", _) => (200, json!({"ok": false, "error": "unknown verb: node.roster"})),
                    _ => (503, json!({"ok": false, "error": "not linked"})),
                }
            }
            _ => (503, json!({"ok": false, "error": "not linked"})),
        }));
    }
    let sent = |to: &str| pushed.lock().unwrap().iter().filter(|(m, _)| m == to).count();

    let home = computer(&hd, hub_addr, server).await;
    keys_on(&hd);
    home.tick().await;
    until("the first list is signed", || seq_of(&home) == 1).await;
    let first = home.roster().unwrap();
    assert_eq!((first.body.epoch, first.body.devices.len()), (1, 1), "the home computer alone");
    assert_eq!(first.body.devices[0].id.to_string(), v_id(&home));
    assert!(hd.join("roster.json").exists() && hd.join("roster-state.json").exists());
    assert!(pushed.lock().unwrap().is_empty());

    // a computer pairs: one more on the list, a new number, and the list is sent to it once
    let (_, v) = control(&home, "POST", "/pair/new", Some(json!({}))).await;
    let member = computer(&md, hub_addr, server).await;
    member.join(v["link"].as_str().unwrap(), Some("Laptop".into()), home.addrs(), false).await.unwrap();
    until("the list has the laptop", || seq_of(&home) == 2).await;
    until("it was sent to the laptop", || sent("laptop") == 1).await;
    assert_eq!(seq_of(&member), 0, "its helper was not running, so it holds none yet");

    // its helper starts: it asks home for the list as soon as it reaches it
    member.tick().await;
    until("the laptop asked for the list", || seq_of(&member) == 2).await;
    assert!(md.join("roster.json").exists());
    assert_eq!(member.pull_now().await, Ok(false), "asking again brings nothing new");
    let (status, v) = control(&member, "GET", "/status", None).await;
    assert_eq!((status, v["roster"]["seq"].as_u64(), v["roster"]["epoch"].as_u64()), (200, Some(2), Some(1)), "{v}");
    assert_eq!(v["roster"]["expires"].as_u64().unwrap() - v["roster"]["at"].as_u64().unwrap(), 7 * 86400);
    *laptop.lock().unwrap() = Some(member.clone());

    // a computer the hub already listed switches to keys: sent once to each, and the old helper's error is no
    // trouble to anybody
    *hub.nodes.lock().unwrap() = json!([
        {"name": "laptop", "online": true, "version": "9.9.9", "label": "Laptop"},
        {"name": "tower", "online": true, "version": "0.8.0", "label": "Tower"},
    ]);
    home.walk_now().await;
    home.publish_now().await;
    until("the list has the tower", || seq_of(&home) == 3).await;
    until("it was sent to both", || sent("laptop") == 2 && sent("tower") == 1).await;
    assert_eq!(seq_of(&member), 3, "the laptop took what the hub link brought");
    assert!(member.roster().unwrap().lists(&tower));
    until("home knows who took it", || {
        let rows = home.devices().unwrap();
        let of = |m: &str| rows.iter().find(|d| d.machine.as_deref() == Some(m)).unwrap().clone();
        of("laptop").answers_away == Some(true) && of("tower").answers_away == Some(false)
    })
    .await;
    let rows = home.devices().unwrap();
    let ver = |m: &str| rows.iter().find(|d| d.machine.as_deref() == Some(m)).unwrap().ver.clone();
    assert_eq!((ver("laptop").as_deref(), ver("tower").as_deref()), (Some("9.9.9"), Some("0.8.0")));

    // What home found out goes onto the list at its next look: the same number, signed again and sent once more,
    // so every computer knows which helpers answer while home is away, and which need an update first.
    let own = |r: &Roster, m: &str| r.body.devices.iter().find(|d| d.machine.as_deref() == Some(m)).unwrap().own;
    home.publish_now().await;
    until("the list says whose helper answers", || {
        let r = home.roster().unwrap();
        (r.body.seq, own(&r, "laptop"), own(&r, "tower")) == (3, Some(true), Some(false))
    })
    .await;
    assert_eq!(home.roster().unwrap().body.devices[0].own, Some(true), "the home computer's own helper does");
    until("it was sent to both again", || sent("laptop") == 3 && sent("tower") == 2).await;
    until("the laptop holds it", || member.roster().is_some_and(|r| own(&r, "tower") == Some(false))).await;

    // nothing changed: nothing is signed, nothing is sent
    let before = home.roster().unwrap();
    home.publish_now().await;
    home.publish_now().await;
    assert_eq!(home.roster().unwrap().envelope, before.envelope);
    assert_eq!((sent("laptop"), sent("tower")), (3, 2));

    // a device is removed: a new number, sent once to the computers that stay, and they drop it
    let (status, _) = control(&home, "POST", "/devices/remove", Some(json!({"id": tower.to_string()}))).await;
    assert_eq!(status, 200);
    until("the list is signed without the tower", || seq_of(&home) == 4).await;
    until("the laptop has it", || seq_of(&member) == 4).await;
    assert!(!member.roster().unwrap().lists(&tower));
    assert_eq!((sent("laptop"), sent("tower")), (4, 2));
    let seqs: Vec<u64> = pushed
        .lock()
        .unwrap()
        .iter()
        .map(|(_, e)| {
            roster::verify(&serde_json::from_value(e.clone()).unwrap(), &home.roster().unwrap().body.home, None)
        })
        .map(|r| r.unwrap().body.seq)
        .collect();
    assert_eq!(seqs, vec![2, 3, 3, 3, 3, 4], "every list that was sent is one home signed");

    // an older list never comes back, and a list somebody else signed is not one
    let old = json!({"verb": "node.roster", "roster": before.envelope});
    let a = member.node_roster(&old);
    assert_eq!((a["ok"].as_bool(), a["took"].as_bool(), a["have"]["seq"].as_u64()), (Some(true), Some(false), Some(4)));
    let stranger = DeviceKey::generate();
    let forged = roster::sign(&stranger, Body { seq: 99, ..home.roster().unwrap().body.clone() }).unwrap();
    let a = member.node_roster(&json!({"verb": "node.roster", "roster": forged.envelope}));
    assert_eq!((a["ok"].as_bool(), a["kind"].as_str()), (Some(false), Some("bad_request")), "{a}");
    assert_eq!(seq_of(&member), 4);
    let a = member.node_roster(&json!({"verb": "node.roster", "roster": "x"}));
    assert_eq!(a["kind"], "bad_request");
    // the home computer takes no list from anybody
    let a = home.node_roster(&old);
    assert_eq!((a["ok"].as_bool(), a["is_home"].as_bool()), (Some(false), Some(true)));

    // the helper answers these itself: everything else is the script's
    assert!(crate::link::answered_here(Some(&member), &old).is_some());
    assert!(crate::link::answered_here(Some(&member), &json!({"verb": "list"})).is_none());
    assert!(crate::link::answered_here(None, &old).is_none());

    // the home computer starts again: the same list, the same number, nothing signed for nothing
    let held = home.roster().unwrap();
    member.shutdown().await;
    home.shutdown().await;
    let home = computer(&hd, hub_addr, server).await;
    home.tick().await;
    until("the list is read back", || seq_of(&home) == 4).await;
    home.publish_now().await;
    assert_eq!(home.roster().unwrap().envelope, held.envelope);
    // ... and it still says who answers while it is away, before it has sent anybody anything
    assert_eq!(home.devices().unwrap()[0].answers_away, Some(true));
    let (_, v) = control(&home, "GET", "/devices", None).await;
    assert_eq!(
        (v["devices"][0]["machine"].as_str(), v["devices"][0]["answers_away"].as_bool()),
        (Some("laptop"), Some(true))
    );
    // ... and with its list lost, the number on disk still never goes back
    home.shutdown().await;
    std::fs::remove_file(hd.join("roster.json")).unwrap();
    let _ = std::fs::remove_file(hd.join("roster.json.bak"));
    let home = computer(&hd, hub_addr, server).await;
    home.tick().await;
    until("a list is signed again", || seq_of(&home) == 5).await;
    home.shutdown().await;
}

#[test]
fn only_a_helper_that_does_not_know_the_verb_is_said_to_need_an_update() {
    let have = json!({"epoch": 1, "seq": 2, "at": 3});
    assert_eq!(took_of(&json!({"ok": true, "took": true, "have": have})), Some(true));
    assert_eq!(took_of(&json!({"ok": true, "took": false, "have": have})), Some(true), "it held a newer one");
    // its own refusals have a kind: the helper knows what a list is
    let refused = json!({"ok": false, "kind": "bad_request", "error": "That list of devices was not taken: x."});
    assert_eq!(took_of(&refused), Some(true));
    assert_eq!(took_of(&json!({"ok": false, "kind": "not_here", "error": "x"})), Some(true));
    // what an older helper's script says to a verb it does not know
    assert_eq!(took_of(&json!({"ok": false, "error": "unknown verb: node.roster"})), Some(false));
    // nothing either way
    assert_eq!(took_of(&json!({"ok": false, "is_home": true, "error": "This is the home computer."})), None);
    assert_eq!(took_of(&json!({"ok": true})), None);
    assert_eq!(took_of(&Value::Null), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_home_computer_is_on_its_list_under_the_name_the_server_shows() {
    let (hub, hub_addr) = fake_hub().await;
    let server = fake_server("x").await;
    let hd = scratch("home-label");
    let home = computer(&hd, hub_addr, server).await;
    linked(&hd, "studio");
    keys_on(&hd);
    *hub.nodes.lock().unwrap() = json!([{"name": "studio", "label": "Studio Mac", "online": true, "version": "1"}]);
    home.tick().await;
    until("the first list is signed", || seq_of(&home) == 1).await;
    let r = home.roster().unwrap();
    assert_eq!(
        (r.body.home_name.as_str(), r.body.devices[0].name.as_str(), r.body.devices[0].machine.as_deref()),
        ("Studio Mac", "Studio Mac", Some("studio"))
    );
    // a hub that does not say just now changes nothing: no new list for a slow answer
    *hub.nodes.lock().unwrap() = json!([]);
    home.publish_now().await;
    assert_eq!(home.roster().unwrap().envelope, r.envelope);
    // the label is changed in the app: the next list carries it, under a new number
    *hub.nodes.lock().unwrap() = json!([{"name": "studio", "label": " Desk ", "online": false, "version": "1"}]);
    home.publish_now().await;
    let r = home.roster().unwrap();
    assert_eq!((r.body.seq, r.body.home_name.as_str(), r.body.devices[0].name.as_str()), (2, "Desk", "Desk"));
    home.shutdown().await;

    // a home computer that runs no chats has no label anywhere: its host name stands
    let (_hub, hub_addr) = fake_hub().await;
    let hd = scratch("home-no-label");
    let home = computer(&hd, hub_addr, server).await;
    keys_on(&hd);
    home.tick().await;
    until("the first list is signed", || seq_of(&home) == 1).await;
    let r = home.roster().unwrap();
    assert_eq!(r.body.home_name, sb_link_clean(&crate::hostname_label()));
    assert_eq!(r.body.devices[0].name, r.body.home_name);
    home.shutdown().await;
}

/// A name as a signed list carries it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_member_calls_the_home_computer_what_its_newest_list_does() {
    let (hub, hub_addr) = fake_hub().await;
    let server = fake_server("x").await;
    let (hd, md) = (scratch("home-renamed"), scratch("member-renamed"));
    let home = computer(&hd, hub_addr, server).await;
    linked(&hd, "studio");
    keys_on(&hd);
    *hub.nodes.lock().unwrap() = json!([{"name": "studio", "label": "Studio Mac", "online": true, "version": "1"}]);
    home.tick().await;
    until("the first list is signed", || seq_of(&home) == 1).await;
    let (_, v) = control(&home, "POST", "/pair/new", Some(json!({}))).await;
    let member = computer(&md, hub_addr, server).await;
    member.join(v["link"].as_str().unwrap(), Some("Laptop".into()), home.addrs(), false).await.unwrap();
    until("the list has the laptop", || seq_of(&home) == 2).await;
    member.tick().await;
    until("the laptop holds the list", || seq_of(&member) == 2).await;
    let named = |k: &Shared| k.status().unwrap().home.unwrap().name;
    assert_eq!(named(&member), "Studio Mac");

    // the home computer is renamed (or moved, with its key, to a computer with another name)
    *hub.nodes.lock().unwrap() = json!([{"name": "studio", "label": "Desk", "online": true, "version": "1"}]);
    home.publish_now().await;
    assert_eq!(home.roster().unwrap().body.home_name, "Desk");
    assert_eq!(member.pull_now().await, Ok(true));
    assert_eq!(named(&member), "Desk", "the name written down at pairing does not stick");
    member.shutdown().await;
    home.shutdown().await;
}

fn sb_link_clean(name: &str) -> String {
    let key = DeviceKey::generate();
    let body = Body { home: key.id(), home_name: name.into(), epoch: 1, seq: 1, at: 1, relay: None, devices: vec![] };
    roster::sign(&key, body).unwrap().body.home_name
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_list_sent_while_keys_are_not_running_is_kept_for_when_they_are() {
    let (_hub, hub_addr) = fake_hub().await;
    let server = fake_server("x").await;
    let d = scratch("asleep");
    let home_key = DeviceKey::generate();
    let k = computer(&d, hub_addr, server).await;
    // not paired with any home computer: there is nobody whose list it could be
    linked(&d, "desk");
    let body = Body {
        home: home_key.id(),
        home_name: "Home".into(),
        epoch: 1,
        seq: 7,
        at: unix_now(),
        relay: None,
        devices: vec![],
    };
    let signed = roster::sign(&home_key, body).unwrap();
    let req = json!({"verb": "node.roster", "roster": signed.envelope});
    assert_eq!(k.node_roster(&req)["kind"], "not_here");
    // paired, its helper's keys not started yet
    let mut cfg = linked(&d, "desk");
    cfg.home = Some(HomeConfig { id: home_key.id(), relay: None, name: "Home".into(), addrs: vec![] });
    crate::save_to(&d.join("node.json"), &cfg).unwrap();
    let a = k.node_roster(&req);
    assert_eq!(
        (a["ok"].as_bool(), a["took"].as_bool(), a["have"]["seq"].as_u64()),
        (Some(true), Some(true), Some(7)),
        "{a}"
    );
    k.tick().await;
    assert_eq!(seq_of(&k), 7, "and it is the list its keys start with");
    k.shutdown().await;
}

// ---- with the home computer away (keys phase 4a) ----------------------------------------------------------------

/// One request to a computer's API port, as a program of Switchboard's makes it. None: the port closed the
/// connection without an answer.
#[cfg(unix)]
async fn door(addr: SocketAddr, method: &str, path: &str, body: Option<Value>) -> Option<(u16, Value)> {
    let mut c = TcpStream::connect(addr).await.ok()?;
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n", addr.port());
    match &body {
        Some(b) => {
            let b = b.to_string();
            req += &format!(
                "X-Switchboard: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{b}",
                b.len()
            );
        }
        None => req += "\r\n",
    }
    c.write_all(req.as_bytes()).await.ok()?;
    let head = tokio::time::timeout(LONG, sb_link::http1::read_head(&mut c)).await.expect("left hanging").ok()?;
    let body = sb_link::http1::read_body(&mut c, &head, 16 << 20).await.ok()?;
    Some((head.status(), serde_json::from_slice(&body).unwrap_or(Value::Null)))
}

/// Asks again until the answer is the one waited for.
#[cfg(unix)]
async fn door_until(
    what: &str,
    addr: SocketAddr,
    method: &str,
    path: &str,
    body: Option<Value>,
    f: impl Fn(&Value) -> bool,
) -> Value {
    let end = std::time::Instant::now() + LONG;
    loop {
        let got = door(addr, method, path, body.clone()).await;
        if let Some((200, v)) = &got
            && f(v)
        {
            return v.clone();
        }
        assert!(std::time::Instant::now() < end, "never happened: {what}; the last answer was {got:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Gives the computer in `dir` a helper script: a shell script that notes what it is asked, shows one chat that
/// runs in the background, and as a terminal says what it was started for.
#[cfg(unix)]
fn lab_script(dir: &Path, chat: &str, bg: &str) {
    let sb = dir.join("sb.sh");
    let text = r#"
if [ -n "$SSH_ORIGINAL_COMMAND" ]; then echo "ATTACHED $SSH_ORIGINAL_COMMAND"; sleep 20; exit 0; fi
req=$(cat)
printf '%s\n' "$req" >> "$0.log"
case "$req" in
  *'"verb":"own"'*) cat "$0.board" ;;
  *) echo '{"ok":true,"did":"it"}' ;;
esac
"#;
    std::fs::write(&sb, text).unwrap();
    let board = json!({"ok": true, "machine": {"name": "?", "label": "?", "online": true, "os": "linux", "replaced": [], "sessions": [
        {"sid": chat, "title": chat, "kind": "background", "status": "idle", "activity": "working", "bg_id": bg},
    ]}});
    std::fs::write(dir.join("sb.sh.board"), board.to_string()).unwrap();
    let mut cfg = crate::load_raw(&dir.join("node.json")).unwrap();
    (cfg.sb, cfg.python) = (Some(sb.display().to_string()), Some("/bin/sh".into()));
    crate::save_to(&dir.join("node.json"), &cfg).unwrap();
}

#[cfg(unix)]
fn script_saw(dir: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(dir.join("sb.sh.log")).unwrap_or_default();
    text.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
}

/// Opens the terminal a `ws` address leads to and reads until it has said `want`.
#[cfg(unix)]
async fn terminal(
    ws: &str,
    want: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>> {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    let (mut ws, _) = tokio_tungstenite::connect_async(format!("{ws}&cols=80&rows=24&proto=2")).await.unwrap();
    let mut out = String::new();
    let end = std::time::Instant::now() + LONG;
    while !out.contains(want) {
        let left = end.checked_duration_since(std::time::Instant::now()).expect("the terminal never said what it runs");
        match tokio::time::timeout(left, ws.next()).await.expect("the terminal never said what it runs") {
            Some(Ok(Message::Binary(b))) => out.push_str(&String::from_utf8_lossy(&b)),
            Some(Ok(_)) => {}
            other => panic!("the terminal ended: {other:?} after {out:?}"),
        }
    }
    ws
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_home_away_a_computer_shows_its_own_chats_and_the_other_computers_and_home_takes_over_again() {
    use futures_util::StreamExt;
    let (_hub, hub_addr) = fake_hub().await;
    let server = fake_server("hello from home").await;
    let (hd, ld, td) = (scratch("home"), scratch("laptop"), scratch("tower"));
    // (home keeps its address across a restart: no relay here, so the others find it where it was)
    let home_at = free_port().await;
    let home = computer_at(&hd, hub_addr, server, home_at).await;
    // (the home computer is one of its own hub's computers too, as it is wherever it has chats of its own)
    linked(&hd, "studio");
    keys_on(&hd);
    home.tick().await;
    let mut members = vec![];
    for (dir, name, chat, bg) in [(&ld, "Laptop", "laptop-chat", "aaaaaaa1"), (&td, "Tower", "tower-chat", "bbbbbbb2")]
    {
        let (_, v) = control(&home, "POST", "/pair/new", Some(json!({}))).await;
        let m = computer(dir, hub_addr, server).await;
        m.join(v["link"].as_str().unwrap(), Some(name.into()), home.addrs(), false).await.unwrap();
        lab_script(dir, chat, bg);
        m.tick().await;
        members.push(m);
    }
    let (laptop, tower) = (members[0].clone(), members[1].clone());
    let reaches_home = |k: &Shared| k.status().is_ok_and(|s| s.home.is_some_and(|h| h.connected));
    until("both reach home", || reaches_home(&laptop) && reaches_home(&tower)).await;
    // home says in its list where it sees each of them, and each takes the list
    until("home sees both", || home.devices().unwrap().iter().all(|d| d.path.is_some())).await;
    home.publish_now().await;
    let listed = home.roster().unwrap();
    assert_eq!(listed.body.devices.len(), 3);
    assert!(listed.body.devices[1..].iter().all(|d| !d.addrs.is_empty()), "{:?}", listed.body.devices);
    laptop.pull_now().await.unwrap();
    tower.pull_now().await.unwrap();
    assert_eq!((seq_of(&laptop), seq_of(&tower)), (listed.body.seq, listed.body.seq));
    let (api, hub_door) = (laptop.ports.api_forward.unwrap(), laptop.ports.hub_forward.unwrap());

    // home answers: the two ports are home's, whatever is asked
    let through = get_through(api).await.unwrap_or_else(|| panic!("no answer from home: {:?}", laptop.status()));
    assert!(through.contains("hello from home"));
    let (_, v) = door(api, "GET", "/api/sessions", None).await.unwrap();
    assert_eq!(v, Value::Null, "the server's own answer, which is no JSON in this lab");
    assert!(laptop.status().unwrap().away.is_none());

    // something is open through home's hub when it goes: a terminal, as a program here would hold one
    let mut held = {
        use tokio::io::AsyncWriteExt;
        let mut c = TcpStream::connect(hub_door).await.unwrap();
        c.write_all(b"GET /term?t=held HTTP/1.1\r\n").await.unwrap();
        c
    };

    // ---- home goes away ----
    home.shutdown().await;
    until("the laptop is away from home", || laptop.status().unwrap().away.is_some()).await;
    // what was open to home is closed, not left hanging: the program behind it asks for its terminal again,
    // and is then answered by the computer the chat is on (below)
    let ended = async {
        use tokio::io::AsyncReadExt;
        let mut buf = [0u8; 256];
        while matches!(held.read(&mut buf).await, Ok(n) if n > 0) {}
    };
    tokio::time::timeout(LONG, ended).await.expect("a connection open to home stayed open after home went away");
    let shows_both = |v: &Value| {
        v["machines"].as_array().is_some_and(|m| m.len() == 3 && m[1]["online"] == true && m[2]["online"] == true)
    };
    let v =
        door_until("both computers show their chats", api, "POST", "/api/refresh", Some(json!({})), shows_both).await;
    let m = v["machines"].as_array().unwrap();
    assert_eq!((m[0]["online"].as_bool(), m[0]["why"].as_str()), (Some(false), Some("is_home")));
    assert_eq!(
        (m[1]["name"].as_str(), m[1]["label"].as_str(), m[1]["sessions"][0]["sid"].as_str()),
        (Some("laptop"), Some("Laptop"), Some("laptop-chat"))
    );
    assert_eq!(
        (m[2]["name"].as_str(), m[2]["label"].as_str(), m[2]["sessions"][0]["sid"].as_str()),
        (Some("tower"), Some("Tower"), Some("tower-chat"))
    );
    assert_eq!(v["away"]["line"], "Home is away. Showing chats from Laptop and Tower.");
    assert_eq!((v["summary"]["sessions"].as_u64(), v["summary"]["running"].as_u64()), (Some(2), Some(2)));
    let (status, sm) = door(api, "GET", "/api/summary", None).await.unwrap();
    assert_eq!(
        (status, sm["online"].as_u64(), sm["away"]["line"].as_str()),
        (200, Some(2), v["away"]["line"].as_str())
    );
    until("the helper says so too", || laptop.status().unwrap().away.is_some_and(|a| a["line"] == v["away"]["line"]))
        .await;
    let said = crate::control::lines_of(&laptop.status().unwrap()).join("\n");
    assert!(said.contains("Home is away. Showing chats from Laptop and Tower."), "{said}");
    assert!(said.contains("List of devices: signed by the home computer just now."), "{said}");
    let (status, no) = door(api, "GET", "/api/usage", None).await.unwrap();
    assert_eq!((status, no["kind"].as_str()), (503, Some("home_away")));
    let (status, no) = door(api, "POST", "/api/new", Some(json!({"machine": "tower"}))).await.unwrap();
    assert_eq!((status, no["error"].as_str()), (503, Some("The home computer is away. This needs the home computer.")));

    // a terminal on the other computer's chat, and on this computer's own
    let (status, t) = door(api, "POST", "/api/term", Some(json!({"sid": "tower-chat"}))).await.unwrap();
    assert_eq!((status, t["ok"].as_bool(), t["machine"].as_str()), (200, Some(true), Some("tower")), "{t}");
    let ws = t["ws"].as_str().unwrap();
    assert!(
        ws.starts_with(&format!("ws://127.0.0.1:{}/term?t=", hub_door.port())) && ws.ends_with("&own=tower"),
        "{ws}"
    );
    let mut far = terminal(ws, "ATTACHED term bbbbbbb2").await;
    let (_, t) = door(api, "POST", "/api/term", Some(json!({"sid": "laptop-chat"}))).await.unwrap();
    let near = terminal(t["ws"].as_str().unwrap(), "ATTACHED term aaaaaaa1").await;
    drop(near);

    // a hook on this computer is answered at once, and its word reaches the next look at the board
    let ping = json!({"event": "ping", "machine": "laptop", "session_id": "laptop-chat", "why": "stop"});
    let mut fastest = Duration::MAX;
    for _ in 0..5 {
        let began = std::time::Instant::now();
        assert_eq!(door(api, "POST", "/api/event", Some(ping.clone())).await, Some((200, json!({"ok": true}))));
        fastest = fastest.min(began.elapsed());
    }
    assert!(fastest < Duration::from_millis(100), "a hook waited {fastest:?}");
    door(api, "POST", "/api/refresh", Some(json!({}))).await.unwrap();
    let told = script_saw(&ld).into_iter().rfind(|r| r["verb"] == "own").unwrap();
    assert_eq!(told["pings"]["laptop-chat"]["status"], "idle", "{told}");

    // an action goes to the computer the chat is on, and only what it may carry
    let (status, a) =
        door(api, "POST", "/api/action", Some(json!({"verb": "stop", "sid": "tower-chat", "when": "idle"})))
            .await
            .unwrap();
    assert_eq!((status, a), (200, json!({"ok": true, "did": "it"})));
    assert!(script_saw(&td).contains(&json!({"verb": "stop", "sid": "tower-chat"})), "{:?}", script_saw(&td));
    let (status, _) =
        door(api, "POST", "/api/action", Some(json!({"verb": "handoff", "sid": "tower-chat"}))).await.unwrap();
    assert_eq!(status, 503);

    // the tower stops answering the laptop, here and now, and then answers it again
    let laptop_id = laptop.status().unwrap().id;
    let (status, r) = control(&tower, "GET", "/roster", None).await;
    assert_eq!(
        (status, r["devices"].as_array().unwrap().len(), r["roster"]["seq"].as_u64()),
        (200, 3, Some(listed.body.seq))
    );
    for (id, why) in [
        (listed.body.home, "The home computer cannot be blocked."),
        (tower.status().unwrap().id, "That is this computer."),
    ] {
        let (status, v) = control(&tower, "POST", "/devices/block", Some(json!({"id": id.to_string()}))).await;
        assert_eq!((status, v["error"].as_str()), (409, Some(why)));
    }
    let nobody = DeviceKey::generate().id().to_string();
    assert_eq!(control(&tower, "POST", "/devices/block", Some(json!({"id": nobody}))).await.0, 409);
    assert_eq!(control(&tower, "POST", "/devices/block", Some(json!({"id": "x"}))).await.0, 400);
    let (status, v) = control(&tower, "POST", "/devices/block", Some(json!({"id": laptop_id.to_string()}))).await;
    assert_eq!((status, v["name"].as_str()), (200, Some("Laptop")), "{v}");
    assert!(td.join("blocked.json").exists());
    let (_, r) = control(&tower, "GET", "/roster", None).await;
    let row = r["devices"].as_array().unwrap().iter().find(|d| d["name"] == "Laptop").unwrap().clone();
    assert_eq!(
        (row["blocked"].as_bool(), row["kind"].as_str(), row["machine"].as_str()),
        (Some(true), Some("computer"), Some("laptop"))
    );
    let ended = async { while let Some(Ok(_)) = far.next().await {} };
    tokio::time::timeout(LONG, ended).await.expect("the terminal of a blocked device stayed open");
    let refused = |v: &Value| v["machines"][2]["why"] == "refused";
    let v = door_until("the tower refuses the laptop", api, "POST", "/api/refresh", Some(json!({})), refused).await;
    assert_eq!(v["away"]["line"], "Home is away. Showing chats from Laptop.");
    assert_eq!(v["away"]["missing"][1], json!({"machine": "tower", "label": "Tower", "why": "refused"}));
    let (status, v) = control(&tower, "POST", "/devices/unblock", Some(json!({"id": laptop_id.to_string()}))).await;
    assert_eq!((status, v["was"].as_bool()), (200, Some(true)));
    let (_, v) = control(&tower, "POST", "/devices/unblock", Some(json!({"id": laptop_id.to_string()}))).await;
    assert_eq!(v["was"], false);
    door_until("the tower answers the laptop again", api, "POST", "/api/refresh", Some(json!({})), shows_both).await;
    // (the home computer blocks nobody: it removes)
    let (_, t) = door(api, "POST", "/api/term", Some(json!({"sid": "tower-chat"}))).await.unwrap();
    let mut far = terminal(t["ws"].as_str().unwrap(), "ATTACHED term bbbbbbb2").await;

    // ---- home is back ----
    let home = computer_at(&hd, hub_addr, server, home_at).await;
    home.tick().await;
    until("the laptop reaches home again", || {
        laptop.status().is_ok_and(|s| s.away.is_none() && s.home.is_some_and(|h| h.connected))
    })
    .await;
    let ended = async { while let Some(Ok(_)) = far.next().await {} };
    tokio::time::timeout(LONG, ended).await.expect("a terminal the stand-in carried outlived it");
    assert!(get_through(api).await.unwrap().contains("hello from home"));
    let (status, v) = control(&home, "POST", "/devices/block", Some(json!({"id": laptop_id.to_string()}))).await;
    assert_eq!(status, 409, "{v}");
    assert!(v["error"].as_str().unwrap().contains("switchboard-node forget"), "{v}");
    for k in [&laptop, &tower, &home] {
        k.shutdown().await;
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_computer_that_was_off_learns_of_a_removal_from_another_computer_while_home_is_away() {
    let (_hub, hub_addr) = fake_hub().await;
    let server = fake_server("hello from home").await;
    let (hd, ld, td, sd) = (scratch("home"), scratch("laptop"), scratch("tower"), scratch("spare"));
    let home = computer(&hd, hub_addr, server).await;
    keys_on(&hd);
    home.tick().await;
    let mut members = vec![];
    for (dir, name, chat, bg) in [
        (&ld, "Laptop", "laptop-chat", "aaaaaaa1"),
        (&td, "Tower", "tower-chat", "bbbbbbb2"),
        (&sd, "Spare", "spare-chat", "ccccccc3"),
    ] {
        let (_, v) = control(&home, "POST", "/pair/new", Some(json!({}))).await;
        let m = computer(dir, hub_addr, server).await;
        m.join(v["link"].as_str().unwrap(), Some(name.into()), home.addrs(), false).await.unwrap();
        lab_script(dir, chat, bg);
        m.tick().await;
        members.push(m);
    }
    let (laptop, tower, spare) = (members[0].clone(), members[1].clone(), members[2].clone());
    let reaches_home = |k: &Shared| k.status().is_ok_and(|s| s.home.is_some_and(|h| h.connected));
    until("all reach home", || members.iter().all(|m| reaches_home(m))).await;
    until("home sees all", || home.devices().unwrap().iter().all(|d| d.path.is_some())).await;
    home.publish_now().await;
    let before = home.roster().unwrap();
    assert_eq!(before.body.devices.len(), 4);
    for m in &members {
        m.pull_now().await.unwrap();
        assert_eq!(seq_of(m), before.body.seq);
    }

    // the tower is off when home removes the spare computer; the laptop learns of it from home
    let spare_id = spare.status().unwrap().id;
    tower.shutdown().await;
    let (status, v) = control(&home, "POST", "/devices/remove", Some(json!({"id": spare_id.to_string()}))).await;
    assert_eq!(status, 200, "{v}");
    until("home signed a list without it", || seq_of(&home) > before.body.seq).await;
    let after = home.roster().unwrap();
    assert!(after.device(&spare_id).is_none());
    // (in this lab no hub passes the list on, so the laptop asks home for it, as it does every minute)
    for _ in 0..50 {
        if laptop.pull_now().await.is_ok() && seq_of(&laptop) == after.body.seq {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(seq_of(&laptop), after.body.seq, "the laptop took it");

    // home goes away, and the tower returns holding the list that still names the spare computer
    home.shutdown().await;
    let tower = computer(&td, hub_addr, server).await;
    tower.tick().await;
    assert_eq!(seq_of(&tower), before.body.seq);
    assert!(tower.roster().unwrap().device(&spare_id).is_some());
    until("the tower is away from home", || tower.status().is_ok_and(|s| s.away.is_some())).await;
    // asking the laptop for its chats is enough: each says which list it holds, and the newer one stands
    let api = tower.ports.api_forward.unwrap();
    let learnt = |_: &Value| seq_of(&tower) == after.body.seq;
    door_until("the tower holds the newer list", api, "POST", "/api/refresh", Some(json!({})), learnt).await;
    assert!(tower.roster().unwrap().device(&spare_id).is_none());
    // and from then on the removed computer is neither asked nor answered by the tower
    let gone = |v: &Value| v["machines"].as_array().is_some_and(|m| m.iter().all(|m| m["name"] != "spare"));
    door_until("the spare computer is not on the board", api, "POST", "/api/refresh", Some(json!({})), gone).await;
    assert_eq!(seq_of(&laptop), after.body.seq, "the older list never replaces the newer one");
    for k in [&laptop, &tower, &spare] {
        k.shutdown().await;
    }
}

/// Moving the home computer, a copy of it, and bringing it back (home.rs).
#[path = "home_tests.rs"]
mod home_tests;
