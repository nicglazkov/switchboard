//! The phone's side against a real home endpoint in the same process: no relay, loopback addresses only, the
//! way sb-link's own endpoint tests run. The calls under test block, as they do for Swift and Kotlin, so these
//! are plain tests and home lives on a runtime of its own.

use super::*;
use futures_util::{SinkExt, StreamExt};
use sb_link::code::{PairLink, ShortCode};
use sb_link::endpoint::{OwnIncoming, PairIncoming};
use sb_link::http1;
use sb_link::paired::{Device, Paired, paired_path};
use sb_link::pairing::{Grant, Home, Offers, Select};
use sb_link::roster::{self, Blocks, Body, Envelope, RosterDevice, RosterGate};
use sb_link::tunnel::Targets;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const LONG: Duration = Duration::from_secs(20);

/// A new empty folder for one test.
pub(crate) fn scratch(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!(
        "sb-mobile-{tag}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn quick() -> Clocks {
    Clocks {
        backoff_first: Duration::from_millis(100),
        backoff_max: Duration::from_millis(400),
        dial: Duration::from_secs(5),
        ..Clocks::default()
    }
}

fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + LONG;
    while !f() {
        assert!(Instant::now() < end, "never happened: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Home: an endpoint with no relay on a loopback port, its paired list, the links it has given out, and behind
/// it a server that answers every request with "hello from home" and a hub that echoes.
struct HomeSide {
    rt: Runtime,
    key: [u8; KEY_LEN],
    key_id: DeviceId,
    ep: Arc<LinkEndpoint>,
    paired: Arc<Paired>,
    offers: Arc<Offers>,
    served: Arc<AtomicU64>,
    /// The list of devices home hands to a device that asks (`GET /own/v1/roster`). None: 404.
    list: Arc<Mutex<Option<Envelope>>>,
}

/// Answers one request on a stream of a computer's own surface and ends the stream.
async fn own_answer(stream: &mut sb_link::endpoint::LinkStream, status: u16, body: Value) {
    let _ = stream.write_all(&http1::answer(status, body.to_string().as_bytes())).await;
    let _ = stream.shutdown().await;
    // (the other side reads the answer and lets go; dropping first would cut it short)
    let _ = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut [0u8; 16])).await;
}

async fn host_one(
    mut inc: PairIncoming,
    offers: Arc<Offers>,
    paired: Arc<Paired>,
    home: Home,
    code: Option<ShortCode>,
) {
    let channel = inc.channel;
    let binding = Binding { host: inc.host.0, joiner: inc.joiner.0, channel: &channel };
    let select = match &code {
        Some(c) => Select::Code(c),
        None => Select::Link,
    };
    let _ = pairing::host(&mut inc.stream, &offers, select, &binding, &home, unix_now(), |req| async move {
        let device = Device {
            id: req.id,
            name: req.about.name.clone(),
            kind: req.about.kind,
            machine: None,
            paired: unix_now(),
            via: req.via,
        };
        paired.add(device).map_err(|e| e.to_string())?;
        Ok(Grant::default())
    })
    .await;
    inc.finish().await;
}

async fn http_server(body: &'static str) -> (SocketAddr, Arc<AtomicU64>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    let count = Arc::new(AtomicU64::new(0));
    let counted = count.clone();
    tokio::spawn(async move {
        loop {
            let (mut s, _) = l.accept().await.unwrap();
            counted.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(async move {
                let mut got = vec![];
                let mut buf = [0u8; 4096];
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf).await {
                        Ok(n) if n > 0 => got.extend_from_slice(&buf[..n]),
                        _ => return,
                    }
                }
                // the header that let the request through the phone's port arrives too, and does no harm
                let gated = String::from_utf8_lossy(&got).to_ascii_lowercase().contains("x-switchboard-gate");
                let body = if gated { body.to_string() } else { format!("{body} (no gate header)") };
                let answer =
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(answer.as_bytes()).await;
            });
        }
    });
    (addr, count)
}

async fn ws_echo_server() -> SocketAddr {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (s, _) = l.accept().await.unwrap();
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(s).await else { return };
                while let Some(Ok(msg)) = ws.next().await {
                    if (msg.is_text() || msg.is_binary()) && ws.send(msg).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    addr
}

fn home() -> HomeSide {
    let paired = Arc::new(Paired::load(&paired_path(&scratch("home"))));
    home_at(DeviceKey::generate(), 0, paired, Arc::default())
}

/// A home computer with `key` on loopback `port` (0: any). With the key, port and paired list of one that was
/// stopped, it is that home computer started again.
fn home_at(key: DeviceKey, port: u16, paired: Arc<Paired>, list: Arc<Mutex<Option<Envelope>>>) -> HomeSide {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let key_id = key.id();
    let offers = Arc::new(Offers::new());
    let held = list.clone();
    let (ep, served) = rt.block_on(async {
        let (api, served) = http_server("hello from home").await;
        let hub = ws_echo_server().await;
        let opts = || Options {
            bind: Some(SocketAddr::from(([127, 0, 0, 1], port))),
            tunnel: true,
            pair: true,
            clocks: quick(),
            ..Options::default()
        };
        // (a port that was just let go can take a moment to be free again)
        let mut tries = 0;
        let ep = loop {
            match LinkEndpoint::bind(&key, opts(), paired.clone()).await {
                Ok(ep) => break ep,
                Err(e) if port != 0 && tries < 40 => {
                    let _ = e;
                    tries += 1;
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(e) => panic!("home could not start: {e}"),
            }
        };
        let (tx, mut pairs) = mpsc::channel(4);
        let (own_tx, mut owns) = mpsc::channel::<OwnIncoming>(16);
        let serving = ep.clone();
        tokio::spawn(
            async move { serving.serve_with(Targets { api: Some(api), hub: Some(hub) }, tx, Some(own_tx)).await },
        );
        // home's own surface, as far as a phone uses it: the list of devices it signed
        tokio::spawn(async move {
            while let Some(mut inc) = owns.recv().await {
                let held = held.clone();
                tokio::spawn(async move {
                    let Ok(head) = http1::read_head(&mut inc.stream).await else { return };
                    let list = held.lock().unwrap().clone();
                    match (head.method(), head.path(), list) {
                        ("GET", "/own/v1/roster", Some(e)) => {
                            own_answer(&mut inc.stream, 200, serde_json::to_value(&e).unwrap()).await
                        }
                        _ => own_answer(&mut inc.stream, 404, json!({"ok": false, "kind": "not_here"})).await,
                    }
                });
            }
        });
        let (offers, paired) = (offers.clone(), paired.clone());
        tokio::spawn(async move {
            while let Some(inc) = pairs.recv().await {
                let about = Home { id: key_id, name: "Studio".into(), relay: None };
                tokio::spawn(host_one(inc, offers.clone(), paired.clone(), about, None));
            }
        });
        (ep, served)
    });
    HomeSide { rt, key: *key.secret_bytes(), key_id, ep, paired, offers, served, list }
}

/// A computer that is not home, as a phone meets it while home is away: an endpoint that lets in whoever is on
/// the list it holds, and answers for its own chats.
struct Laptop {
    rt: Runtime,
    ep: Arc<LinkEndpoint>,
    id: DeviceId,
    cache: Arc<RosterCache>,
    /// The head of every request it was given, as it arrived.
    asked: Arc<Mutex<Vec<String>>>,
}

fn laptop(home: DeviceId) -> Laptop {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let key = DeviceKey::generate();
    let dir = scratch("laptop");
    let cache = Arc::new(RosterCache::load(&dir.join("roster.json"), home));
    let gate = Arc::new(RosterGate::new(cache.clone(), Arc::new(Blocks::load(&dir.join("blocked.json")))));
    let asked: Arc<Mutex<Vec<String>>> = Arc::default();
    let (heads, lists) = (asked.clone(), cache.clone());
    let ep = rt.block_on(async {
        let opts = Options {
            bind: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
            tunnel: true,
            clocks: quick(),
            ..Options::default()
        };
        let ep = LinkEndpoint::bind(&key, opts, gate).await.unwrap();
        let (tx, _pairs) = mpsc::channel(1);
        let (own_tx, mut owns) = mpsc::channel::<OwnIncoming>(16);
        let serving = ep.clone();
        tokio::spawn(async move { serving.serve_with(Targets::default(), tx, Some(own_tx)).await });
        tokio::spawn(async move {
            while let Some(mut inc) = owns.recv().await {
                let (heads, lists) = (heads.clone(), lists.clone());
                tokio::spawn(async move {
                    let s = &mut inc.stream;
                    let Ok(head) = http1::read_head(s).await else { return };
                    let body = http1::read_request_body(s, &head, 1 << 20).await.unwrap_or_default();
                    heads.lock().unwrap().push(String::from_utf8_lossy(&head.raw).into_owned());
                    match (head.method(), head.path()) {
                        ("GET", "/own/v1/board") => {
                            let chat = json!({"sid": "lap-1", "title": "A chat on the laptop", "activity": "working",
                                              "kind": "background", "status": "idle"});
                            let machine = json!({"name": "laptop", "label": "Laptop", "online": true, "os": "macos",
                                                 "replaced": [], "sessions": [chat]});
                            own_answer(s, 200, json!({"ok": true, "machine": machine})).await
                        }
                        ("GET", "/own/v1/roster") => match lists.get() {
                            Some(r) => own_answer(s, 200, serde_json::to_value(&r.envelope).unwrap()).await,
                            None => own_answer(s, 404, json!({"ok": false})).await,
                        },
                        ("POST", "/own/v1/roster") => {
                            let took = serde_json::from_slice::<Envelope>(&body)
                                .ok()
                                .and_then(|e| lists.offer(&e, unix_now()).ok())
                                .unwrap_or(false);
                            own_answer(s, 200, json!({"ok": true, "took": took, "have": lists.stamp()})).await
                        }
                        _ => own_answer(s, 404, json!({"ok": false, "kind": "not_here"})).await,
                    }
                });
            }
        });
        ep
    });
    Laptop { rt, id: key.id(), ep, cache, asked }
}

impl HomeSide {
    /// A link as Add a device shows it (and as its QR code holds it). No relay in it: this home has none.
    fn link(&self) -> String {
        let (secret, _) = self.offers.mint_link(unix_now());
        PairLink { home: self.key_id, relay: None, secret, name: "Studio".into() }.to_string()
    }

    fn addrs(&self) -> Vec<SocketAddr> {
        self.ep.addrs()
    }

    /// Signs the list of devices: home, the laptop where it can be reached, and the phone. Home hands it to
    /// whoever asks from then on.
    fn sign(&self, laptop: &Laptop, phone: &str, seq: u64) -> Envelope {
        let device = |id: DeviceId, name: &str, kind, machine: Option<&str>, addrs: Vec<SocketAddr>| RosterDevice {
            id,
            name: name.into(),
            kind,
            machine: machine.map(str::to_string),
            ver: None,
            own: machine.map(|_| true),
            addrs,
        };
        let devices = vec![
            device(self.key_id, "Studio", Kind::Computer, Some("studio"), self.addrs()),
            device(laptop.id, "Laptop", Kind::Computer, Some("laptop"), laptop.ep.addrs()),
            device(phone.parse().unwrap(), "Sam's iPhone", Kind::Phone, None, vec![]),
        ];
        let body =
            Body { home: self.key_id, home_name: "Studio".into(), epoch: 1, seq, at: unix_now(), relay: None, devices };
        let envelope = roster::sign(&DeviceKey::from_bytes(self.key), body).unwrap().envelope.clone();
        *self.list.lock().unwrap() = Some(envelope.clone());
        envelope
    }

    /// The home computer goes: its endpoint closes, and what it was serving with it. What is handed back is
    /// what `home_at` needs to start it again where it was.
    fn stop(self) -> (DeviceKey, u16, Arc<Paired>, Arc<Mutex<Option<Envelope>>>) {
        let port = self.addrs()[0].port();
        self.rt.block_on(self.ep.close());
        let HomeSide { rt, key, ep, paired, list, .. } = self;
        drop(ep);
        rt.shutdown_timeout(Duration::from_secs(2));
        (DeviceKey::from_bytes(key), port, paired, list)
    }

    /// A short code, and the place it leads to: an endpoint whose key comes from the code. Its address.
    fn code(&self) -> (String, Vec<SocketAddr>) {
        let (code, _) = self.offers.mint_code(unix_now());
        let key = DeviceKey::from_bytes(code.rendezvous_key());
        let (offers, paired, key_id) = (self.offers.clone(), self.paired.clone(), self.key_id);
        let shown = code.to_string();
        let addrs = self.rt.block_on(async move {
            let opts = Options {
                bind: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
                pair: true,
                clocks: quick(),
                ..Options::default()
            };
            let ep = LinkEndpoint::bind(&key, opts, Arc::new(Nobody)).await.unwrap();
            let addrs = ep.addrs();
            let (tx, mut pairs) = mpsc::channel(4);
            tokio::spawn(async move {
                let serving = ep.clone();
                tokio::spawn(async move { serving.serve(Targets::default(), tx).await });
                while let Some(inc) = pairs.recv().await {
                    let about = Home { id: key_id, name: "Studio".into(), relay: None };
                    host_one(inc, offers.clone(), paired.clone(), about, Some(code.clone())).await;
                }
            });
            addrs
        });
        (shown, addrs)
    }
}

fn phone() -> Arc<HomeLink> {
    let dir = scratch("phone").to_string_lossy().into_owned();
    HomeLink::with_clocks(new_device_key(), dir, None, quick()).unwrap()
}

/// A phone that has paired with `home` by link.
fn paired_phone(home: &HomeSide) -> (Arc<HomeLink>, HomeInfo) {
    let phone = phone();
    let info = phone.pair_at(&home.link(), "Sam's iPhone", home.addrs()).expect("pairing");
    (phone, info)
}

/// One GET through a loopback port, with `gate` as the header's value when there is one. The status line and
/// the body, or None when the connection was closed without an answer.
fn get(port: u16, gate: Option<&str>) -> Option<(String, String)> {
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).ok()?;
    c.set_read_timeout(Some(LONG)).unwrap();
    let gate = gate.map(|g| format!("X-Switchboard-Gate: {g}\r\n")).unwrap_or_default();
    c.write_all(format!("GET /api/sessions HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{gate}\r\n").as_bytes()).ok()?;
    let mut answer = vec![];
    c.read_to_end(&mut answer).expect("the connection was left hanging");
    if answer.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(&answer).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    Some((head.lines().next().unwrap_or("").to_string(), body.to_string()))
}

fn ok(port: u16, gate: &str) -> bool {
    get(port, Some(gate)).is_some_and(|(status, body)| status == "HTTP/1.1 200 OK" && body == "hello from home")
}

#[test]
fn a_phone_pairs_with_a_link_and_reaches_the_server_through_its_own_port() {
    let home = home();
    let phone = phone();
    assert_eq!(phone.status().state, LinkState::Stopped);
    assert_eq!(phone.device_id().len(), 64);

    let info = phone.pair_at(&home.link(), "  Sam's iPhone ", home.addrs()).expect("pairing");
    assert_eq!(info, HomeInfo { id: home.key_id.to_string(), name: "Studio".into(), relay: None });
    let listed = home.paired.list();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id.to_string(), phone.device_id());
    assert_eq!((listed[0].kind, listed[0].name.as_str(), &listed[0].machine), (Kind::Phone, "Sam's iPhone", &None));
    assert_eq!(phone.status().state, LinkState::Stopped, "pairing leaves the link stopped");

    let ports = phone.start(info.clone()).expect("start");
    assert!(ports.api != 0 && ports.hub != 0 && ports.api != ports.hub);
    assert_eq!(ports.gate.len(), 64);
    assert!(ok(ports.api, &ports.gate), "the first request waits for the connection and is answered");
    assert_eq!(home.served.load(Ordering::Relaxed), 1);

    until("connected", || phone.status().state == LinkState::Connected && phone.status().path.is_some());
    let s = phone.status();
    assert_eq!((s.path, s.line.as_str(), &s.fix, &s.detail), (Some(LinkPath::Direct), "Direct", &None, &None));
    assert!(s.rtt_ms.is_some());
    assert_eq!(phone.start(info).unwrap(), ports, "starting again with the same home changes nothing");
    assert!(ok(ports.api, &ports.gate));
}

#[test]
fn a_websocket_echoes_through_the_hub_port() {
    let home = home();
    let (phone, info) = paired_phone(&home);
    let ports = phone.start(info).unwrap();
    home.rt.block_on(async {
        let tcp = tokio::net::TcpStream::connect(("127.0.0.1", ports.hub)).await.unwrap();
        let mut req = format!("ws://127.0.0.1:{}/term", ports.hub).into_client_request().unwrap();
        req.headers_mut().insert(gate::GATE_HEADER, ports.gate.parse().unwrap());
        let (mut ws, response) = tokio_tungstenite::client_async(req, tcp).await.unwrap();
        assert_eq!(response.status().as_u16(), 101);
        ws.send(Message::Text("typed".into())).await.unwrap();
        let got = tokio::time::timeout(LONG, ws.next()).await.unwrap().unwrap().unwrap();
        assert!(matches!(&got, Message::Text(t) if t.as_str() == "typed"), "{got:?}");
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        ws.send(Message::Binary(big.clone().into())).await.unwrap();
        let got = tokio::time::timeout(LONG, ws.next()).await.unwrap().unwrap().unwrap();
        assert!(matches!(&got, Message::Binary(b) if b[..] == big[..]));
        ws.close(None).await.unwrap();

        // without the header the handshake is refused before anything reaches the hub
        let tcp = tokio::net::TcpStream::connect(("127.0.0.1", ports.hub)).await.unwrap();
        let req = format!("ws://127.0.0.1:{}/term", ports.hub).into_client_request().unwrap();
        assert!(tokio_tungstenite::client_async(req, tcp).await.is_err());
    });
}

#[test]
fn another_app_on_the_phone_gets_nothing_from_the_ports() {
    let home = home();
    let (phone, info) = paired_phone(&home);
    let ports = phone.start(info).unwrap();
    assert!(ok(ports.api, &ports.gate));
    let before = home.served.load(Ordering::Relaxed);

    let refused = Some(("HTTP/1.1 403 Forbidden".to_string(), String::new()));
    assert_eq!(get(ports.api, None), refused, "no header");
    assert_eq!(get(ports.api, Some("")), refused);
    assert_eq!(get(ports.api, Some(&"0".repeat(64))), refused, "a guess");
    assert_eq!(get(ports.hub, None), refused, "the hub's port asks too");
    // another phone's secret is not this one's
    assert_eq!(get(ports.api, Some(&self::phone().gate)), refused);
    assert_eq!(home.served.load(Ordering::Relaxed), before, "none of it reached home");
    assert!(ok(ports.api, &ports.gate), "and the app itself is still let through");
}

#[test]
fn removing_the_phone_at_home_cuts_it_off() {
    let home = home();
    let (phone, info) = paired_phone(&home);
    let ports = phone.start(info).unwrap();
    assert!(ok(ports.api, &ports.gate));
    let before = home.served.load(Ordering::Relaxed);

    let id: DeviceId = phone.device_id().parse().unwrap();
    assert!(home.paired.remove(&id).unwrap());
    assert_eq!(home.ep.sweep(), 1);
    until("the removal shows", || phone.status().state == LinkState::Removed);
    let s = phone.status();
    assert_eq!(s.line, "Can't reach home: it no longer lists this phone.");
    assert!(s.fix.as_deref().is_some_and(|f| f.contains("Add a device")), "{s:?}");
    assert!(s.path.is_none() && s.rtt_ms.is_none() && s.detail.is_some(), "{s:?}");
    assert_eq!(get(ports.api, Some(&ports.gate)), None, "its requests are closed with no answer");
    assert_eq!(home.served.load(Ordering::Relaxed), before);
}

#[test]
fn a_phone_that_never_paired_is_refused() {
    let home = home();
    let phone = phone();
    store::save(&phone.dir, &home.key_id.to_string(), &home.addrs());
    let info = HomeInfo { id: home.key_id.to_string(), name: "Studio".into(), relay: None };
    let ports = phone.start(info).unwrap();
    assert_eq!(get(ports.api, Some(&ports.gate)), None);
    until("the refusal shows", || phone.status().state == LinkState::Removed);
    assert_eq!(home.served.load(Ordering::Relaxed), 0);
}

#[test]
fn stop_closes_the_ports_and_start_brings_them_back() {
    let home = home();
    let (phone, info) = paired_phone(&home);
    let ports = phone.start(info.clone()).unwrap();
    assert!(ok(ports.api, &ports.gate));

    phone.stop();
    assert_eq!(phone.status(), LinkStatus::stopped());
    assert!(std::net::TcpStream::connect(("127.0.0.1", ports.api)).is_err(), "the port is closed");
    until("home sees it leave", || home.ep.peers().is_empty());
    phone.stop(); // stopping twice is nothing
    phone.network_changed(); // and so is this while stopped
    assert_eq!(phone.status().state, LinkState::Stopped);

    let began = Instant::now();
    let again = phone.start(info).unwrap();
    assert!(ok(again.api, &again.gate));
    let took = began.elapsed();
    assert!(took < Duration::from_secs(5), "back in {took:?}");
    assert_eq!(again.gate, ports.gate, "the secret is this launch's, not this start's");
    assert_eq!((again.api, again.hub), (ports.api, ports.hub), "the same ports, since nothing took them");
    until("connected again", || phone.status().state == LinkState::Connected);
}

#[test]
fn a_changed_network_dials_again_on_the_same_ports() {
    let home = home();
    let (phone, info) = paired_phone(&home);
    let ports = phone.start(info.clone()).unwrap();
    assert!(ok(ports.api, &ports.gate));
    phone.network_changed();
    assert!(ok(ports.api, &ports.gate), "the next request waits for the new connection");
    assert_eq!(phone.start(info).unwrap(), ports);
    until("one connection at home", || home.ep.peers().len() == 1);
}

#[test]
fn home_going_away_and_coming_back_is_followed() {
    let home = home();
    let (phone, info) = paired_phone(&home);
    let ports = phone.start(info).unwrap();
    assert!(ok(ports.api, &ports.gate));
    // the phone's own connection is cut at home, as when home restarts: it dials again by itself
    let id: DeviceId = phone.device_id().parse().unwrap();
    assert_eq!(home.ep.cut(&id), 1);
    until("it noticed, and came back", || home.ep.peers().len() == 1 && phone.status().state == LinkState::Connected);
    assert!(ok(ports.api, &ports.gate));
}

/// One request through a loopback port with the phone's secret: the status line, and the body as JSON.
fn ask(port: u16, gate: &str, method: &str, path: &str) -> (String, Value) {
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(LONG)).unwrap();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Switchboard-Gate: {gate}\r\nX-Switchboard: 1\r\n\
         Content-Length: 0\r\n\r\n"
    );
    c.write_all(head.as_bytes()).unwrap();
    let mut answer = vec![];
    c.read_to_end(&mut answer).expect("the connection was left hanging");
    let text = String::from_utf8_lossy(&answer).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    (head.lines().next().unwrap_or("").to_string(), serde_json::from_str(body).unwrap_or(Value::Null))
}

#[test]
fn with_home_away_the_same_ports_show_the_other_computers_and_home_takes_over_again() {
    let home = home();
    let dir = scratch("phone");
    let key = new_device_key();
    let soon = Duration::from_millis(400);
    let make = || HomeLink::with_timing(key.clone(), dir.to_string_lossy().into_owned(), None, quick(), soon).unwrap();
    let phone = make();
    let info = phone.pair_at(&home.link(), "Sam's iPhone", home.addrs()).expect("pairing");
    let laptop = laptop(home.key_id);
    let list = home.sign(&laptop, &phone.device_id(), 2);
    assert!(laptop.cache.offer(&list, unix_now()).unwrap());

    let heard = Arc::new(Heard(Mutex::new(vec![])));
    phone.set_listener(Box::new(heard.clone()));
    let ports = phone.start(info.clone()).unwrap();
    assert!(ok(ports.api, &ports.gate));
    until("the phone holds the list home signed", || dir.join("roster.json").exists());
    let s = phone.status();
    assert!(!s.away && s.state == LinkState::Connected, "{s:?}");
    assert!(laptop.asked.lock().unwrap().is_empty(), "nobody else is asked while home answers");

    // The home computer goes. A few moments later the same two ports answer without it.
    let (home_key, port, paired, held) = home.stop();
    until("away is said", || phone.status().away);
    until("the laptop's chats are showing", || phone.status().line == "Home is away. Showing chats from Laptop.");
    let s = phone.status();
    assert_eq!((s.state, s.path, s.rtt_ms), (LinkState::Connecting, None, None), "{s:?}");
    assert_eq!(phone.start(info.clone()).unwrap(), ports, "the same ports and the same secret");

    let (status, v) = ask(ports.api, &ports.gate, "GET", "/api/sessions");
    assert_eq!(status, "HTTP/1.1 200 OK");
    assert_eq!(v["away"]["line"], "Home is away. Showing chats from Laptop.");
    assert_eq!(v["away"]["home"]["name"], "Studio");
    assert_eq!(v["away"]["showing"], json!([{"machine": "laptop", "label": "Laptop"}]));
    let rows = v["machines"].as_array().unwrap();
    let row = rows.iter().find(|m| m["name"] == "laptop").expect("the laptop's row");
    assert_eq!(row["sessions"][0]["sid"], "lap-1");
    assert_eq!(v["summary"]["running"], 1);
    // what needs the home computer says so, in a sentence
    let (status, v) = ask(ports.api, &ports.gate, "GET", "/api/history");
    assert_eq!(status, "HTTP/1.1 503 Service Unavailable");
    assert_eq!((v["kind"].as_str(), v["error"].as_str()), (Some("home_away"), Some(standin::HOME_AWAY)));
    // the gate is as shut as it was: another app on the phone gets nothing, away or not
    let refused = Some(("HTTP/1.1 403 Forbidden".to_string(), String::new()));
    assert_eq!(get(ports.api, None), refused);
    assert_eq!(get(ports.hub, Some("0".repeat(64).as_str())), refused);
    // and the phone's secret went to no other computer
    let asked = laptop.asked.lock().unwrap().clone();
    assert!(asked.iter().any(|h| h.starts_with("GET /own/v1/board ")), "{asked:?}");
    assert!(asked.iter().any(|h| h.starts_with("POST /own/v1/roster ")), "the two compared their lists: {asked:?}");
    assert!(asked.iter().all(|h| !h.to_ascii_lowercase().contains("x-switchboard-gate")), "{asked:?}");

    // The app is closed and opened with the home computer still away: the list is in the data folder.
    phone.stop();
    drop(phone);
    let phone = make();
    phone.set_listener(Box::new(heard.clone()));
    let again = phone.start(info.clone()).unwrap();
    until("away is said again", || phone.status().line == "Home is away. Showing chats from Laptop.");
    let (status, v) = ask(again.api, &again.gate, "GET", "/api/sessions");
    assert_eq!((status.as_str(), v["machines"].as_array().map(Vec::len)), ("HTTP/1.1 200 OK", Some(2)));

    // The home computer comes back where it was: the same ports lead to it again, and nothing is left of away.
    let back = home_at(home_key, port, paired, held);
    until("home is reached again", || {
        let s = phone.status();
        s.state == LinkState::Connected && !s.away
    });
    assert!(ok(again.api, &again.gate));
    assert_eq!(phone.start(info).unwrap(), again);
    assert!(back.served.load(Ordering::Relaxed) >= 1);
    until("the connections to the other computers are let go", || laptop.ep.peers().is_empty());

    let all = heard.0.lock().unwrap().clone();
    let aways: Vec<bool> = all.iter().map(|s| s.away).collect();
    assert!(aways.contains(&true) && aways.last() == Some(&false), "{aways:?}");
    assert!(all.iter().filter(|s| s.away).all(|s| s.line.starts_with("Home is away.")), "{all:?}");
    for pair in all.windows(2) {
        assert!(pair[0].differs(&pair[1]), "told the same thing twice: {pair:?}");
    }
    drop(laptop.rt);
}

#[test]
fn a_phone_with_no_list_says_home_is_away_and_shows_nothing() {
    // (a phone that paired with a home computer from before lists: it never got one)
    let home = home();
    let phone = phone_away_after(Duration::from_millis(300));
    let info = phone.pair_at(&home.link(), "Pixel", home.addrs()).expect("pairing");
    let ports = phone.start(info).unwrap();
    assert!(ok(ports.api, &ports.gate));
    assert!(!phone.dir.join("roster.json").exists(), "home had no list to hand over");
    home.stop();
    until("away is said", || phone.status().away);
    until("and that nobody can be reached", || {
        phone.status().line == "Home is away. No other computer can be reached."
    });
    let (status, v) = ask(ports.api, &ports.gate, "GET", "/api/sessions");
    assert_eq!((status.as_str(), v["machines"].as_array().map(Vec::len)), ("HTTP/1.1 200 OK", Some(0)));
    assert!(v["away"]["roster"].is_null());
}

#[test]
fn a_phone_that_home_removed_is_not_shown_the_other_computers() {
    let home = home();
    let phone = phone_away_after(Duration::from_millis(200));
    let info = phone.pair_at(&home.link(), "Pixel", home.addrs()).expect("pairing");
    let ports = phone.start(info).unwrap();
    assert!(ok(ports.api, &ports.gate));
    let id: DeviceId = phone.device_id().parse().unwrap();
    assert!(home.paired.remove(&id).unwrap());
    assert_eq!(home.ep.sweep(), 1);
    until("the removal shows", || phone.status().state == LinkState::Removed);
    std::thread::sleep(Duration::from_millis(900));
    let s = phone.status();
    assert!(!s.away && s.line == words::REMOVED, "{s:?}");
    assert_eq!(get(ports.api, Some(&ports.gate)), None, "its requests are closed with no answer");
}

fn phone_away_after(after: Duration) -> Arc<HomeLink> {
    let dir = scratch("phone").to_string_lossy().into_owned();
    HomeLink::with_timing(new_device_key(), dir, None, quick(), after).unwrap()
}

struct Heard(Mutex<Vec<LinkStatus>>);

impl LinkListener for Arc<Heard> {
    fn changed(&self, status: LinkStatus) {
        self.0.lock().unwrap().push(status);
    }
}

#[test]
fn the_listener_hears_each_change_once() {
    let home = home();
    let (phone, info) = paired_phone(&home);
    let heard = Arc::new(Heard(Mutex::new(vec![])));
    phone.set_listener(Box::new(heard.clone()));
    let states = || heard.0.lock().unwrap().iter().map(|s| s.state).collect::<Vec<_>>();

    let ports = phone.start(info.clone()).unwrap();
    assert!(ok(ports.api, &ports.gate));
    until("connected is heard", || states().contains(&LinkState::Connected));
    phone.stop();
    until("stopped is heard", || states().last() == Some(&LinkState::Stopped));
    let all = heard.0.lock().unwrap().clone();
    for pair in all.windows(2) {
        assert!(pair[0].differs(&pair[1]), "told the same thing twice: {pair:?}");
    }
    assert_eq!(all.iter().filter(|s| s.state == LinkState::Stopped).count(), 1);

    phone.clear_listener();
    let n = all.len();
    phone.start(info).unwrap();
    until("connected", || phone.status().state == LinkState::Connected);
    assert_eq!(heard.0.lock().unwrap().len(), n, "nobody is told after clear_listener");
}

#[test]
fn a_short_code_pairs_too() {
    let home = home();
    let (code, at) = home.code();
    let phone = phone();
    // typed the way people type: lower case, a space for the dash
    let typed = code.to_lowercase().replace('-', " ");
    let info = phone.pair_at(&typed, "Pixel", at).expect("pairing with the code");
    assert_eq!(info.id, home.key_id.to_string(), "the code led to home, and home said who it is");
    assert_eq!(home.paired.list()[0].kind, Kind::Phone);

    // the code is used up
    let (_, at) = home.code();
    let again = self::phone().pair_at(&typed, "Pixel", at);
    assert!(matches!(again, Err(LinkError::Code { .. }) | Err(LinkError::Unreachable { .. })), "{again:?}");
}

#[test]
fn a_wrong_or_used_link_says_so() {
    let home = home();
    let link = home.link();
    let (_, _) = paired_phone(&home);
    // a link that was never given out
    let other = PairLink::new(home.key_id, None, "Studio").to_string();
    let e = phone().pair_at(&other, "Phone", home.addrs()).unwrap_err();
    assert!(matches!(&e, LinkError::Code { text: message } if message.starts_with("That code has expired")), "{e:?}");
    // a link that works once works once
    let first = phone().pair_at(&link, "One", home.addrs());
    assert!(first.is_ok(), "{first:?}");
    let e = phone().pair_at(&link, "Two", home.addrs()).unwrap_err();
    assert!(matches!(e, LinkError::Code { .. }), "{e:?}");
    assert_eq!(home.paired.list().len(), 2);
}

#[test]
fn what_cannot_work_is_said_before_anything_is_sent() {
    let p = phone();
    let says = |r: Result<HomeInfo, LinkError>| r.unwrap_err().message().to_string();
    assert!(matches!(p.pair(String::new(), "x".into()), Err(LinkError::Input { .. })));
    assert_eq!(says(p.pair("ABCD".into(), "x".into())), "A code has 8 letters and digits. That was 4.");
    assert!(
        says(p.pair("https://example.com/x".into(), "x".into())).starts_with("That isn't a Switchboard pairing link.")
    );
    if BUILT_IN_RELAY.is_empty() {
        assert_eq!(says(p.pair("ABCD-EFGH".into(), "x".into())), words::NEEDS_LINK);
        let link = PairLink::new(DeviceKey::generate().id(), None, "Studio").to_string();
        assert_eq!(says(p.pair(link, "x".into())), words::NEEDS_ADDRESS);
    }
    // nobody at the address
    let link = PairLink::new(DeviceKey::generate().id(), None, "Studio").to_string();
    let e = p.pair_at(&link, "x", vec!["127.0.0.1:9".parse().unwrap()]).unwrap_err();
    assert!(
        matches!(&e, LinkError::Unreachable { text: message } if message.starts_with("Couldn't reach your home computer.")),
        "{e:?}"
    );

    let dir = scratch("bad").to_string_lossy().into_owned();
    assert!(HomeLink::new(vec![1, 2, 3], dir.clone(), None).is_err(), "a key is 32 bytes");
    assert!(HomeLink::new(new_device_key(), dir.clone(), Some("not a url".into())).is_err());
    assert!(HomeLink::new(new_device_key(), dir.clone(), Some("  ".into())).is_ok(), "an empty relay is no relay");
    let p = HomeLink::new(new_device_key(), dir, None).unwrap();
    assert!(p.start(HomeInfo { id: "nope".into(), name: String::new(), relay: None }).is_err());
}

#[test]
fn a_key_and_its_id() {
    let key = new_device_key();
    assert_eq!(key.len(), 32);
    assert_ne!(key, new_device_key());
    let id = device_id_of(key.clone()).unwrap();
    assert_eq!(id.len(), 64);
    assert_eq!(phone_with(key).device_id(), id);
    assert_eq!(device_id_of(vec![0; 31]), None);
    assert_eq!(gate_header(), "X-Switchboard-Gate");
}

fn phone_with(key: Vec<u8>) -> Arc<HomeLink> {
    HomeLink::with_clocks(key, scratch("phone").to_string_lossy().into_owned(), None, quick()).unwrap()
}
