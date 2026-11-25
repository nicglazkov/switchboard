use super::*;
use crate::identity::DeviceKey;
use crate::roster::{self, Body};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use tokio::io::{AsyncReadExt, DuplexStream};

/// A lab: a home that can be switched off, and computers that answer for their own chats as the real ones do.
struct Lab {
    up: AtomicBool,
    roster: Mutex<Option<Arc<Roster>>>,
    /// What each computer does when asked: Ok(its board), or the error opening a stream to it gives.
    computers: Mutex<HashMap<String, Result<Value, io::ErrorKind>>>,
    /// How long a computer takes to show its board.
    slow: Mutex<HashMap<String, Duration>>,
    /// Every request a computer was given: (machine, first line, body).
    asked: Asked,
    events: Mutex<Vec<Value>>,
    changes: Mutex<Vec<bool>>,
    now: AtomicU64,
}

type Asked = Arc<Mutex<Vec<(String, String, String)>>>;

fn unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

/// What a computer in the lab answers on one stream.
async fn computer(asked: Asked, wait: Option<Duration>, machine: String, board: Value, mut s: DuplexStream) {
    let Ok(head) = http1::read_head(&mut s).await else { return };
    let body = http1::read_request_body(&mut s, &head, 1 << 20).await.unwrap_or_default();
    let first = head.first.join(" ");
    asked.lock().unwrap().push((machine.clone(), first, String::from_utf8_lossy(&body).into_owned()));
    let (status, answer) = match (head.method(), head.path()) {
        ("GET", "/own/v1/board") => {
            if let Some(wait) = wait {
                tokio::time::sleep(wait).await;
            }
            (200, json!({"ok": true, "machine": board}))
        }
        ("POST", "/own/v1/term") => {
            (200, json!({"ok": true, "t": format!("ticket.{machine}"), "bg": "c03ef0d9", "title": "A chat"}))
        }
        ("POST", "/own/v1/action") => (200, json!({"ok": true, "by": machine})),
        ("GET", "/own/v1/term") => {
            // a terminal, for this lab: the head it was given comes back, then every byte
            let text = format!("HTTP/1.1 101 Switching Protocols\r\n\r\n{}", String::from_utf8_lossy(&head.raw));
            let _ = s.write_all(text.as_bytes()).await;
            let _ = s.write_all(&head.rest).await;
            let mut buf = [0u8; 1024];
            while let Ok(n) = s.read(&mut buf).await {
                if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            return;
        }
        _ => (404, json!({"ok": false, "kind": "not_here", "error": "This computer does not answer that."})),
    };
    let _ = s.write_all(&http1::answer(status, answer.to_string().as_bytes())).await;
}

impl Backend for Lab {
    fn home_up(&self) -> bool {
        self.up.load(Ordering::Relaxed)
    }

    fn open_home(&self, target: Target) -> Fut<'_, io::Result<Io>> {
        Box::pin(async move {
            if !self.home_up() {
                return Err(io::Error::new(io::ErrorKind::NotConnected, "home is not reached"));
            }
            let (near, mut far) = tokio::io::duplex(64 * 1024);
            tokio::spawn(async move {
                // the home computer of this lab says where the stream went and repeats what it is sent
                let _ = far.write_all(format!("home {}: ", target.name()).as_bytes()).await;
                let mut buf = [0u8; 1024];
                while let Ok(n) = far.read(&mut buf).await {
                    if n == 0 || far.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            });
            Ok(Box::new(near) as Io)
        })
    }

    fn roster(&self) -> Option<Arc<Roster>> {
        self.roster.lock().unwrap().clone()
    }

    fn open_own(&self, device: &RosterDevice) -> Fut<'_, io::Result<Io>> {
        let machine = device.machine.clone().unwrap_or_default();
        Box::pin(async move {
            let board = self.computers.lock().unwrap().get(&machine).cloned();
            match board {
                Some(Ok(board)) => {
                    let (near, far) = tokio::io::duplex(64 * 1024);
                    let wait = self.slow.lock().unwrap().get(&machine).copied();
                    tokio::spawn(computer(self.asked.clone(), wait, machine, board, far));
                    Ok(Box::new(near) as Io)
                }
                Some(Err(kind)) => Err(io::Error::new(kind, "no")),
                None => Err(io::Error::new(io::ErrorKind::NotFound, "no such computer")),
            }
        })
    }

    fn event(&self, body: &Value) {
        self.events.lock().unwrap().push(body.clone());
    }

    fn away_changed(&self, away: bool) {
        self.changes.lock().unwrap().push(away);
    }

    fn now(&self) -> u64 {
        match self.now.load(Ordering::Relaxed) {
            0 => unix(),
            n => n,
        }
    }
}

fn chat(sid: &str, activity: &str, more: Value) -> Value {
    let mut c = json!({"sid": sid, "title": sid, "activity": activity, "kind": "background", "status": "idle"});
    for (k, v) in more.as_object().unwrap() {
        c[k] = v.clone();
    }
    c
}

/// Home, a laptop and a tower that answer, a mini whose helper is old and does not (home found that out, and the
/// roster says so), a locked computer that refuses this device, and a phone. The tower runs an old version too,
/// and home never got to ask it: nothing is concluded from the number.
fn lab() -> (Arc<Lab>, Arc<StandIn>) {
    lab_settling(Duration::ZERO)
}

/// `settle`: for how long after home went away a round nobody answered is not believed.
fn lab_settling(settle: Duration) -> (Arc<Lab>, Arc<StandIn>) {
    let home = DeviceKey::generate();
    let device = |name: &str, kind, machine: Option<&str>, ver: Option<&str>, own: Option<bool>| RosterDevice {
        id: DeviceKey::generate().id(),
        name: name.into(),
        kind,
        machine: machine.map(str::to_string),
        ver: ver.map(str::to_string),
        own,
        addrs: vec![],
    };
    let mut devices = vec![
        device("Home", Kind::Computer, Some("home"), Some("0.8.23"), Some(true)),
        device("Laptop", Kind::Computer, Some("laptop"), Some("0.8.23"), Some(true)),
        device("Tower", Kind::Computer, Some("tower"), Some("0.8.9"), None),
        device("Mini", Kind::Computer, Some("mini"), Some("0.8.23"), Some(false)),
        device("Locked", Kind::Computer, Some("locked"), Some("0.8.23"), Some(false)),
        device("Phone", Kind::Phone, None, None, None),
    ];
    devices[0].id = home.id();
    let body = Body { home: home.id(), home_name: "Home".into(), epoch: 1, seq: 3, at: unix(), relay: None, devices };
    let signed = roster::sign(&home, body).unwrap();
    let laptop = json!({"name": "it-says-home", "label": "Mine", "online": true, "os": "macos", "replaced": [], "sessions": [
        chat("lap-1", "working", json!({})),
        chat("lap-2", "idle", json!({"cache": "cold", "heavy": true})),
        chat("lap-old", "idle", json!({"stale": true})),
    ]});
    let tower = json!({"name": "tower", "label": "Tower", "online": true, "os": "linux", "replaced": [], "sessions": [
        chat("tow-1", "working", json!({"bg_id": "c03ef0d9"})),
    ]});
    let computers = HashMap::from([
        ("laptop".to_string(), Ok(laptop)),
        ("tower".to_string(), Ok(tower)),
        ("mini".to_string(), Err(io::ErrorKind::ConnectionRefused)),
        ("locked".to_string(), Err(io::ErrorKind::PermissionDenied)),
    ]);
    let lab = Arc::new(Lab {
        up: AtomicBool::new(true),
        roster: Mutex::new(Some(Arc::new(signed))),
        computers: Mutex::new(computers),
        slow: Mutex::default(),
        asked: Arc::default(),
        events: Mutex::default(),
        changes: Mutex::default(),
        now: AtomicU64::new(0),
    });
    let opts = Options {
        away_after: Duration::from_millis(150),
        poll: Duration::from_millis(10),
        board_wait: Duration::from_millis(600),
        board_for: Duration::from_millis(300),
        rounds_every: None,
        settle,
        ..Options::default()
    };
    let standin = StandIn::new(lab.clone(), opts);
    standin.set_hub_port(8132);
    tokio::spawn(standin.clone().run());
    (lab, standin)
}

async fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < end, "never happened: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn go_away(lab: &Lab, s: &StandIn) {
    // (home was reached first, long enough to be seen)
    tokio::time::sleep(Duration::from_millis(40)).await;
    lab.up.store(false, Ordering::Relaxed);
    until("away", || s.away_since().is_some()).await;
}

/// The bytes a program gets back from one of the two ports.
async fn raw(s: &Arc<StandIn>, target: Target, request: &[u8]) -> Vec<u8> {
    let (mut near, far) = tokio::io::duplex(64 * 1024);
    tokio::spawn(s.clone().carry(target, far));
    near.write_all(request).await.unwrap();
    near.shutdown().await.unwrap();
    let mut out = vec![];
    tokio::time::timeout(Duration::from_secs(10), near.read_to_end(&mut out)).await.expect("no answer").unwrap();
    out
}

fn request(method: &str, path: &str, body: Option<&Value>) -> Vec<u8> {
    let mut out = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:8130\r\n");
    if let Some(b) = body {
        let b = b.to_string();
        out += &format!("X-Switchboard: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{b}", b.len());
    } else {
        out += "\r\n";
    }
    out.into_bytes()
}

async fn parse(bytes: Vec<u8>) -> (u16, Value) {
    let mut r = &bytes[..];
    let head = http1::read_head(&mut r).await.unwrap_or_else(|e| panic!("{e}: {:?}", String::from_utf8_lossy(&bytes)));
    let body = http1::read_body(&mut r, &head, 16 << 20).await.unwrap();
    assert_eq!(head.header("connection"), Some("close"));
    (head.status(), serde_json::from_slice(&body).unwrap())
}

async fn api(s: &Arc<StandIn>, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    parse(raw(s, Target::Api, &request(method, path, body.as_ref())).await).await
}

#[tokio::test]
async fn while_home_answers_the_ports_are_the_tunnel_and_nothing_else() {
    let (lab, s) = lab();
    let text = raw(&s, Target::Api, b"GET /api/sessions HTTP/1.1\r\n\r\n").await;
    assert_eq!(text, b"home api: GET /api/sessions HTTP/1.1\r\n\r\n", "byte for byte");
    // (own= means nothing to anybody while home answers: the request is home's)
    let text = raw(&s, Target::Hub, b"GET /term?t=x&own=tower HTTP/1.1\r\n\r\n").await;
    assert_eq!(text, b"home hub: GET /term?t=x&own=tower HTTP/1.1\r\n\r\n");
    assert_eq!(s.away(), None);

    // home out of reach, for less than it takes to be away: a connection is closed, as it always was
    lab.up.store(false, Ordering::Relaxed);
    let text = raw(&s, Target::Api, b"GET /api/sessions HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").await;
    assert!(text.is_empty(), "{:?}", String::from_utf8_lossy(&text));
    assert!(lab.asked.lock().unwrap().is_empty() && lab.changes.lock().unwrap().is_empty());
}

#[tokio::test]
async fn away_the_board_is_what_the_other_computers_say_and_who_is_missing_has_a_reason() {
    let (lab, s) = lab();
    go_away(&lab, &s).await;
    assert_eq!(*lab.changes.lock().unwrap(), vec![true]);
    let first = s.away().unwrap();
    assert_eq!(first["line"], "Home is away. Asking the other computers for their chats.");
    assert_eq!((first["showing"].clone(), first["home"]["name"].as_str()), (json!([]), Some("Home")));

    let (status, v) = api(&s, "GET", "/api/sessions", None).await;
    assert_eq!(status, 200);
    let machines = v["machines"].as_array().unwrap();
    let names: Vec<&str> = machines.iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["home", "laptop", "tower", "mini", "locked"], "every computer on the roster, in its order");
    assert_eq!(machines[1]["label"], "Laptop", "named as the home computer names it");
    assert_eq!(machines[1]["sessions"].as_array().unwrap().len(), 3);
    assert_eq!(machines[2]["sessions"][0]["sid"], "tow-1");
    for (i, why) in [(0, "is_home"), (3, "needs_update"), (4, "refused")] {
        let m = &machines[i];
        assert_eq!(
            (m["online"].as_bool(), m["why"].as_str(), m["sessions"].as_array().map(Vec::len)),
            (Some(false), Some(why), Some(0))
        );
        assert!(m["error"].as_str().is_some_and(|e| !e.is_empty()));
    }
    assert_eq!(machines[3]["error"], "Mini needs an update before it can answer while the home computer is away.");

    let away = &v["away"];
    assert_eq!(away["line"], "Home is away. Showing chats from Laptop and Tower.");
    assert_eq!(
        away["showing"],
        json!([{"machine": "laptop", "label": "Laptop"}, {"machine": "tower", "label": "Tower"}])
    );
    assert_eq!(
        away["missing"],
        json!([
            {"machine": "home", "label": "Home", "why": "is_home"},
            {"machine": "mini", "label": "Mini", "why": "needs_update"},
            {"machine": "locked", "label": "Locked", "why": "refused"},
        ])
    );
    assert_eq!(away["notes"], json!(["Mini needs an update before it can answer while the home computer is away."]));
    assert_eq!(
        away["lacks"],
        json!(["new_chat", "move", "history", "files", "usage", "library", "settings", "notifications"])
    );
    assert!(
        away["lacks_line"].as_str().unwrap().starts_with("Not available while the home computer is away: new chats")
    );
    assert_eq!(away["home"]["name"], "Home");
    assert!(away["home"]["last_seen"].as_u64().is_some_and(|t| t <= unix()) && away["since"].as_u64().is_some());
    assert_eq!(away["roster"]["expires"].as_u64().unwrap() - away["roster"]["at"].as_u64().unwrap(), 7 * 86400);
    assert_eq!(s.away().as_ref(), Some(away));

    let (status, sm) = api(&s, "GET", "/api/summary", None).await;
    assert_eq!(status, 200);
    assert_eq!(sm, v["summary"]);
    assert_eq!(
        (sm["sessions"].as_u64(), sm["running"].as_u64(), sm["cold"].as_u64()),
        (Some(3), Some(2), Some(1)),
        "{sm}"
    );
    assert_eq!((sm["online"].as_u64(), sm["machines"].as_u64()), (Some(2), Some(5)));
    assert_eq!(
        sm["machine_list"][1],
        json!({"name": "laptop", "label": "Laptop", "online": true, "os": "macos", "icon": null, "running": 1, "cold": 1})
    );
    assert_eq!(sm["away"]["line"], away["line"]);
    assert!(sm["five_hour"].is_null());
    // two looks, one round of asking: each computer that answers was asked once
    let boards = |lab: &Lab| lab.asked.lock().unwrap().iter().filter(|a| a.1.contains("/own/v1/board")).count();
    assert_eq!(boards(&lab), 2);
    // ... and a refresh asks again now
    let (status, v2) = api(&s, "POST", "/api/refresh", Some(json!({}))).await;
    assert_eq!((status, boards(&lab)), (200, 4));
    assert_eq!(v2["machines"], v["machines"]);

    // a computer that takes longer than it may is missing, and the others are shown
    lab.slow.lock().unwrap().insert("tower".into(), Duration::from_secs(3));
    let (_, v) = api(&s, "POST", "/api/refresh", Some(json!({}))).await;
    assert_eq!(v["away"]["line"], "Home is away. Showing chats from Laptop.");
    // (an old version number is no reason to say it needs an update: home never said its helper cannot answer)
    assert_eq!(v["away"]["missing"][1], json!({"machine": "tower", "label": "Tower", "why": "unreachable"}));
    assert_eq!(v["machines"][2]["error"], "Tower cannot be reached just now.");
    lab.computers.lock().unwrap().insert("laptop".into(), Err(io::ErrorKind::TimedOut));
    let (_, v) = api(&s, "POST", "/api/refresh", Some(json!({}))).await;
    assert_eq!(v["away"]["line"], "Home is away. No other computer can be reached.");
    assert_eq!(v["away"]["missing"][1], json!({"machine": "laptop", "label": "Laptop", "why": "unreachable"}));
    // one that refuses this device is said to refuse it, whatever its helper knows
    assert_eq!(v["away"]["missing"][4], json!({"machine": "locked", "label": "Locked", "why": "refused"}));
    assert_eq!(
        v["away"]["notes"],
        json!(["Mini needs an update before it can answer while the home computer is away."])
    );
    assert_eq!(v["summary"]["sessions"], 0);
}

#[tokio::test]
async fn away_what_needs_home_says_so_and_only_this_device_is_answered() {
    let (lab, s) = lab();
    go_away(&lab, &s).await;
    let gone =
        json!({"ok": false, "kind": "home_away", "error": "The home computer is away. This needs the home computer."});
    for (method, path, body) in [
        ("GET", "/api/usage", None),
        ("GET", "/api/history?sid=lap-1", None),
        ("GET", "/", None),
        ("GET", "/api/sessions/extra", None),
        ("POST", "/api/new", Some(json!({"machine": "laptop"}))),
        ("POST", "/api/summary", Some(json!({}))),
        ("POST", "/api/action", Some(json!({"verb": "handoff", "sid": "lap-1"}))),
        ("POST", "/api/action", Some(json!({"verb": "fresh", "sid": "lap-1"}))),
        ("POST", "/api/action", Some(json!({"sid": "lap-1"}))),
        ("POST", "/api/action", Some(json!({"verb": "stop", "sid": "x", "machine": "home"}))),
        ("POST", "/api/term", Some(json!({"sid": "x", "machine": "home"}))),
        ("DELETE", "/api/sessions", None),
    ] {
        assert_eq!(api(&s, method, path, body).await, (503, gone.clone()), "{method} {path}");
    }
    assert_eq!(parse(raw(&s, Target::Hub, &request("GET", "/node", None)).await).await, (503, gone.clone()));
    assert_eq!(parse(raw(&s, Target::Hub, &request("GET", "/term?t=x", None)).await).await, (503, gone.clone()));
    assert_eq!(
        parse(raw(&s, Target::Hub, &request("GET", "/term?t=x&own=home", None)).await).await,
        (503, gone.clone())
    );
    assert_eq!(parse(raw(&s, Target::Hub, &request("GET", "/term?t=x&own=nobody", None)).await).await, (503, gone));

    // another site's name for this port, and a POST no program of Switchboard's made
    let other = b"GET /api/sessions HTTP/1.1\r\nHost: evil.example\r\n\r\n";
    assert_eq!(parse(raw(&s, Target::Api, other).await).await.0, 403);
    assert_eq!(parse(raw(&s, Target::Api, b"GET /api/sessions HTTP/1.1\r\n\r\n").await).await.0, 403);
    assert_eq!(
        parse(raw(&s, Target::Hub, b"GET /term?t=x&own=tower HTTP/1.1\r\nHost: evil.example\r\n\r\n").await).await.0,
        503
    );
    let plain =
        b"POST /api/action HTTP/1.1\r\nHost: localhost:8130\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\n{}";
    assert_eq!(parse(raw(&s, Target::Api, plain).await).await.0, 403);
    let no_mark = b"POST /api/action HTTP/1.1\r\nHost: localhost:8130\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}";
    assert_eq!(parse(raw(&s, Target::Api, no_mark).await).await.0, 403);
    let (status, v) = api(&s, "POST", "/api/action", Some(json!(["stop"]))).await;
    assert_eq!((status, v["kind"].as_str()), (400, Some("bad_request")));
    let (status, v) = api(&s, "POST", "/api/action", Some(json!({"verb": "stop", "sid": "no good; id"}))).await;
    assert_eq!((status, v["kind"].as_str()), (400, Some("bad_request")));
    assert!(lab.asked.lock().unwrap().iter().all(|a| a.1.contains("/own/v1/board")), "{:?}", lab.asked.lock().unwrap());
}

#[tokio::test]
async fn away_a_hooks_word_is_answered_at_once_whatever_the_other_computers_do() {
    let (lab, s) = lab();
    lab.slow.lock().unwrap().insert("tower".into(), Duration::from_millis(1500));
    go_away(&lab, &s).await;
    // somebody is looking at the board, and one computer is slow to show its own
    let looking = tokio::spawn({
        let s = s.clone();
        async move { api(&s, "GET", "/api/sessions", None).await }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let ping = json!({"event": "ping", "machine": "laptop", "session_id": "lap-1", "why": "stop"});
    let mut slowest = Duration::ZERO;
    for _ in 0..20 {
        let began = Instant::now();
        let (status, v) = api(&s, "POST", "/api/event", Some(ping.clone())).await;
        slowest = slowest.max(began.elapsed());
        assert_eq!((status, v), (200, json!({"ok": true})));
    }
    assert!(slowest < Duration::from_millis(100), "a hook waited {slowest:?}");
    assert!(!looking.is_finished(), "the board was still being asked for");
    assert_eq!(lab.events.lock().unwrap().len(), 20);
    assert_eq!(lab.events.lock().unwrap()[0], ping);
    assert_eq!(looking.await.unwrap().0, 200);
}

#[tokio::test]
async fn away_a_terminal_and_an_action_go_to_the_computer_the_chat_is_on() {
    let (lab, s) = lab();
    go_away(&lab, &s).await;
    let last = |lab: &Lab| lab.asked.lock().unwrap().last().cloned().unwrap();

    // by the chat's id alone: the board says whose it is
    let (status, v) = api(&s, "POST", "/api/term", Some(json!({"sid": "tow-1"}))).await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["ws"], "ws://127.0.0.1:8132/term?t=ticket.tower&own=tower");
    assert_eq!(
        (v["ok"].as_bool(), v["machine"].as_str(), v["label"].as_str(), v["bg"].as_str()),
        (Some(true), Some("tower"), Some("Tower"), Some("c03ef0d9"))
    );
    assert!(v.get("t").is_none());
    assert_eq!(last(&lab), ("tower".into(), "POST /own/v1/term HTTP/1.1".into(), r#"{"sid":"tow-1"}"#.into()));
    let (_, v) = api(&s, "POST", "/api/term", Some(json!({"bg": "c03ef0d9"}))).await;
    assert_eq!(v["ws"], "ws://127.0.0.1:8132/term?t=ticket.tower&own=tower");
    assert_eq!(last(&lab).2, r#"{"bg":"c03ef0d9"}"#);
    let named = b"POST /api/term HTTP/1.1\r\nHost: localhost:9\r\nX-Switchboard: 1\r\nContent-Type: application/json\r\nContent-Length: 34\r\n\r\n{\"sid\":\"lap-1\",\"machine\":\"laptop\"}";
    let (_, v) = parse(raw(&s, Target::Api, named).await).await;
    assert_eq!(
        v["ws"], "ws://localhost:8132/term?t=ticket.laptop&own=laptop",
        "the name the program used for this device"
    );
    let (status, v) = api(&s, "POST", "/api/term", Some(json!({"sid": "nobody-has-it"}))).await;
    assert_eq!(
        (status, v["kind"].as_str(), v["error"].as_str()),
        (404, Some("not_here"), Some("That chat isn't on the list anymore."))
    );
    let (status, v) = api(&s, "POST", "/api/term", Some(json!({"sid": "x", "machine": "mini"}))).await;
    assert_eq!((status, v["kind"].as_str()), (502, Some("failed")), "{v}");

    // the terminal itself: the hub port, to the computer `own` names, with the path its own surface serves
    let (mut near, far) = tokio::io::duplex(64 * 1024);
    tokio::spawn(s.clone().carry(Target::Hub, far));
    let ask = "GET /term?t=ticket.tower&own=tower&cols=80 HTTP/1.1\r\nHost: 127.0.0.1:8132\r\nUpgrade: websocket\r\n\
               Origin: http://127.0.0.1:8130\r\nSec-WebSocket-Key: abc\r\n\r\nfirst bytes";
    near.write_all(ask.as_bytes()).await.unwrap();
    let want = "HTTP/1.1 101 Switching Protocols\r\n\r\nGET /own/v1/term?t=ticket.tower&cols=80 HTTP/1.1\r\n\
                Host: 127.0.0.1:8132\r\nUpgrade: websocket\r\nSec-WebSocket-Key: abc\r\n\r\nfirst bytes";
    let mut got = vec![0u8; want.len()];
    tokio::time::timeout(Duration::from_secs(5), near.read_exact(&mut got)).await.unwrap().unwrap();
    assert_eq!(String::from_utf8_lossy(&got), want);
    near.write_all(b"typed").await.unwrap();
    let mut got = [0u8; 5];
    tokio::time::timeout(Duration::from_secs(5), near.read_exact(&mut got)).await.unwrap().unwrap();
    assert_eq!(&got, b"typed");

    // an action, by id and by name; what else the request carries stays here
    let (status, v) =
        api(&s, "POST", "/api/action", Some(json!({"verb": "stop", "sid": "lap-2", "when": "idle", "cmd": "x"}))).await;
    assert_eq!((status, v), (200, json!({"ok": true, "by": "laptop"})));
    let (to, first, sent) = last(&lab);
    assert_eq!((to.as_str(), first.as_str()), ("laptop", "POST /own/v1/action HTTP/1.1"));
    assert_eq!(serde_json::from_str::<Value>(&sent).unwrap(), json!({"verb": "stop", "sid": "lap-2"}));
    for verb in VERBS {
        let (status, v) =
            api(&s, "POST", "/api/action", Some(json!({"verb": verb, "sid": "any", "machine": "tower"}))).await;
        assert_eq!((status, v["by"].as_str()), (200, Some("tower")));
    }

    // home answers again: the terminal's connection is dropped, and the next one is home's
    lab.up.store(true, Ordering::Relaxed);
    let mut rest = vec![];
    tokio::time::timeout(Duration::from_secs(5), near.read_to_end(&mut rest))
        .await
        .expect("the terminal stayed open")
        .ok();
    assert_eq!(s.away(), None);
    assert_eq!(*lab.changes.lock().unwrap(), vec![true, false]);
    let text = raw(&s, Target::Api, b"GET /api/sessions HTTP/1.1\r\n\r\n").await;
    assert_eq!(text, b"home api: GET /api/sessions HTTP/1.1\r\n\r\n");
}

#[tokio::test]
async fn just_after_home_went_away_nobody_answering_is_not_believed_yet() {
    let (lab, s) = lab_settling(Duration::from_millis(1500));
    for name in ["laptop", "tower"] {
        lab.computers.lock().unwrap().insert(name.into(), Err(io::ErrorKind::TimedOut));
    }
    go_away(&lab, &s).await;
    // the connections to the other computers are only being made: the line does not say nobody is there
    let (_, v) = api(&s, "POST", "/api/refresh", Some(json!({}))).await;
    assert_eq!(v["away"]["line"], "Home is away. Asking the other computers for their chats.");
    assert_eq!(v["away"]["showing"], json!([]));
    assert_eq!(s.away().unwrap()["line"], "Home is away. Asking the other computers for their chats.");
    // one that says no will not say yes a moment later: with only such computers the line is the plain one
    let unreachable = lab.computers.lock().unwrap().clone();
    for name in ["laptop", "tower"] {
        lab.computers.lock().unwrap().insert(name.into(), Err(io::ErrorKind::PermissionDenied));
    }
    let (_, v) = api(&s, "POST", "/api/refresh", Some(json!({}))).await;
    assert_eq!(v["away"]["line"], "Home is away. No other computer can be reached.");
    // still nobody once that time is over: then it is said
    *lab.computers.lock().unwrap() = unreachable;
    tokio::time::sleep(Duration::from_millis(1600)).await;
    let (_, v) = api(&s, "POST", "/api/refresh", Some(json!({}))).await;
    assert_eq!(v["away"]["line"], "Home is away. No other computer can be reached.");
}

#[tokio::test]
async fn away_with_a_roster_past_its_time_nobody_is_asked_and_the_line_says_why() {
    let (lab, s) = lab();
    lab.now.store(unix() + 8 * 86400, Ordering::Relaxed);
    go_away(&lab, &s).await;
    let (status, v) = api(&s, "GET", "/api/sessions", None).await;
    assert_eq!(status, 200);
    assert_eq!(
        v["away"]["line"],
        "What this device knows about your computers is 7 days old, so they no longer answer it. Start the home \
         computer, or replace it."
    );
    assert_eq!(v["away"]["showing"], json!([]));
    assert!(v["machines"].as_array().unwrap().iter().all(|m| m["online"] == false));
    assert!(lab.asked.lock().unwrap().is_empty(), "nobody was asked");

    // and with no roster at all there is nobody to show
    *lab.roster.lock().unwrap() = None;
    let (_, v) = api(&s, "POST", "/api/refresh", Some(json!({}))).await;
    assert_eq!(v["away"]["line"], "Home is away. No other computer can be reached.");
    assert_eq!((v["machines"].clone(), v["away"]["roster"].clone()), (json!([]), Value::Null));
}

#[test]
fn lists_of_names() {
    let names = |n: &[&str]| listed(&n.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(names(&["A"]), "A");
    assert_eq!(names(&["A", "B"]), "A and B");
    assert_eq!(names(&["A", "B", "C"]), "A, B and C");
}
