use super::*;
use crate::paired::{Device, Kind, Via};
use crate::pairing::{self, About, Binding, Grant, Home, Offers, Secret, Select};
use crate::testing::scratch;
use crate::tunnel::{Forwarder, Target};
use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

const LONG: Duration = Duration::from_secs(20);

/// Clocks short enough to test and long enough for a busy build machine.
fn quick() -> Clocks {
    Clocks {
        backoff_first: Duration::from_millis(100),
        backoff_max: Duration::from_millis(400),
        dial: Duration::from_secs(5),
        ..Clocks::default()
    }
}

fn loopback(port: u16) -> Option<SocketAddr> {
    Some(SocketAddr::from(([127, 0, 0, 1], port)))
}

fn device(id: DeviceId, name: &str) -> Device {
    Device { id, name: name.into(), kind: Kind::Computer, machine: Some(name.into()), paired: 1, via: Via::Code }
}

/// Home: an endpoint with no relay on a loopback port, its paired list in a folder of its own, and its accept
/// loop running towards `targets`.
struct HomeSide {
    key: DeviceKey,
    ep: Arc<LinkEndpoint>,
    paired: Arc<Paired>,
    pairs: mpsc::Receiver<PairIncoming>,
}

async fn home_on(key: DeviceKey, paired: Arc<Paired>, targets: Targets, port: u16) -> HomeSide {
    let opts = Options { bind: loopback(port), tunnel: true, pair: true, clocks: quick(), ..Options::default() };
    let end = Instant::now() + LONG;
    let ep = loop {
        // (a port that was just closed can take a moment to be free again)
        match LinkEndpoint::bind(&key, opts.clone(), paired.clone()).await {
            Ok(ep) => break ep,
            Err(e) => assert!(Instant::now() < end, "{e}"),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let (tx, pairs) = mpsc::channel(4);
    let serving = ep.clone();
    tokio::spawn(async move { serving.serve(targets, tx).await });
    HomeSide { key, ep, paired, pairs }
}

async fn home(targets: Targets) -> HomeSide {
    let paired = Arc::new(Paired::load(&crate::paired::paired_path(&scratch("endpoint"))));
    home_on(DeviceKey::generate(), paired, targets, 0).await
}

impl HomeSide {
    fn peer(&self) -> Peer {
        Peer { id: self.ep.id(), relay: None, addrs: self.ep.addrs() }
    }
}

/// Another computer: an endpoint that takes nothing, a tunnel to `home` kept up, and a forwarder for `target`
/// on a free loopback port.
struct Member {
    ep: Arc<LinkEndpoint>,
    tunnel: Arc<Tunnel>,
    port: SocketAddr,
}

async fn member_with(key: &DeviceKey, home: Peer, target: Target) -> Member {
    let opts = Options { bind: loopback(0), clocks: quick(), ..Options::default() };
    let ep = LinkEndpoint::bind(key, opts, Arc::new(Nobody)).await.unwrap();
    let tunnel = Tunnel::new(ep.clone(), home);
    tokio::spawn(tunnel.clone().run());
    let f = Forwarder::bind("127.0.0.1:0".parse().unwrap(), target).await.unwrap();
    let port = f.local_addr().unwrap();
    let t = tunnel.clone();
    tokio::spawn(f.run(move || {
        let t = t.clone();
        async move { t.open().await }
    }));
    Member { ep, tunnel, port }
}

async fn until(what: &str, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + LONG;
    while !f() {
        assert!(Instant::now() < end, "never happened: {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A server that answers every connection with `body`, and counts them.
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
                let answer =
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = s.write_all(answer.as_bytes()).await;
            });
        }
    });
    (addr, count)
}

/// One GET through `port`. The whole answer, or None when the connection was closed without one.
async fn get(port: SocketAddr) -> Option<String> {
    let mut c = TcpStream::connect(port).await.unwrap();
    c.write_all(b"GET /api/state HTTP/1.1\r\nHost: 127.0.0.1:8130\r\n\r\n").await.ok()?;
    let mut answer = vec![];
    match timeout(LONG, c.read_to_end(&mut answer)).await {
        Ok(Ok(_)) if !answer.is_empty() => Some(String::from_utf8_lossy(&answer).into_owned()),
        Ok(_) => None,
        Err(_) => panic!("the connection was left hanging"),
    }
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

#[test]
fn the_names_on_the_wire() {
    assert_eq!(ALPN_TUNNEL, b"switchboard/tunnel/1");
    assert_eq!(ALPN_PAIR, b"switchboard/pair/1");
    assert_eq!(MAX_STREAMS, 256);
    let c = Clocks::default();
    assert_eq!(
        (c.keep_alive, c.first_byte, c.silent),
        (Duration::from_secs(5), Duration::from_secs(2), Duration::from_secs(10))
    );
}

#[test]
fn a_path_is_dead_only_when_a_stream_waits_and_nothing_at_all_arrives() {
    let c = Clocks::default();
    let s = Duration::from_secs;
    assert!(dead(Some(s(3)), s(11), &c));
    assert!(!dead(None, s(60), &c), "nobody is waiting: an idle connection is left alone");
    assert!(!dead(Some(s(3)), s(9), &c), "packets still arrive: the server is only slow");
    assert!(!dead(Some(s(1)), s(11), &c), "the stream has only just been opened");
    assert!(!dead(Some(s(2)), s(11), &c));
    assert!(!dead(Some(s(3)), s(10), &c));
}

#[test]
fn streams_wait_until_their_first_byte() {
    let w: Arc<Waiting> = Arc::default();
    assert!(w.oldest().is_none());
    let a = w.add();
    std::thread::sleep(Duration::from_millis(5));
    let b = w.add();
    let first = w.oldest().unwrap();
    drop(b);
    assert_eq!(w.oldest(), Some(first), "the oldest is the one still waiting");
    drop(a);
    assert!(w.oldest().is_none());
}

#[test]
fn the_pairing_door_lets_only_so_many_in_a_minute() {
    let r = Rate::new(3, Duration::from_secs(60));
    let t = Instant::now();
    assert!(r.take(t) && r.take(t) && r.take(t));
    assert!(!r.take(t));
    assert!(!r.take(t + Duration::from_secs(59)));
    assert!(r.take(t + Duration::from_secs(60)), "a new minute");
    assert!(r.take(t + Duration::from_secs(61)) && r.take(t + Duration::from_secs(61)));
    assert!(!r.take(t + Duration::from_secs(62)));
}

#[test]
fn the_relays_warning_about_one_key_in_two_places() {
    let relay = "iroh::socket::transports::relay::actor";
    let said = "Relay server reports problem: Another endpoint connected with the same endpoint id. No more messages will be received.";
    assert_eq!(same_id_from(relay, said), Some(true));
    assert_eq!(same_id_from(relay, "Relay server reports: The connection is healthy and has recovered"), Some(false));
    assert_eq!(same_id_from(relay, "Relay server reports problem: something else"), None);
    assert_eq!(same_id_from("iroh::endpoint", said), None, "only the relay's own lines count");
}

#[test]
fn an_address_needs_a_relay_or_somewhere_to_go() {
    let id = DeviceKey::generate().id();
    assert!(addr_of(&Peer { id, relay: None, addrs: vec![] }).is_err());
    assert!(addr_of(&Peer { id, relay: Some("not a url".into()), addrs: vec![] }).is_err());
    let a = addr_of(&Peer { id, relay: Some("https://relay.example".into()), addrs: vec![] }).unwrap();
    assert_eq!(a.relay_urls().count(), 1);
    let a = addr_of(&Peer { id, relay: None, addrs: vec!["127.0.0.1:9".parse().unwrap()] }).unwrap();
    assert_eq!(a.ip_addrs().count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_that_is_not_paired_is_refused() {
    let (api, count) = http_server("secret").await;
    let home = home(Targets { api: Some(api), hub: None }).await;
    let m = member_with(&DeviceKey::generate(), home.peer(), Target::Api).await;

    assert_eq!(get(m.port).await, None, "its request is closed with no answer");
    until("the refusal shows", || m.tunnel.status().refused).await;
    let s = m.tunnel.status();
    assert!(!s.connected && s.error.is_some(), "{s:?}");
    assert_eq!(count.load(Ordering::Relaxed), 0, "nothing of it reached the server");
    assert!(home.ep.peers().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_paired_device_gets_an_http_answer_through_the_tunnel() {
    let (api, count) = http_server("hello from home").await;
    let home = home(Targets { api: Some(api), hub: None }).await;
    let key = DeviceKey::generate();
    home.paired.add(device(key.id(), "laptop")).unwrap();
    let m = member_with(&key, home.peer(), Target::Api).await;

    let answer = get(m.port).await.expect("an answer");
    assert!(answer.starts_with("HTTP/1.1 200 OK\r\n") && answer.ends_with("\r\n\r\nhello from home"), "{answer}");
    assert_eq!(count.load(Ordering::Relaxed), 1);

    let s = m.tunnel.status();
    assert!(s.connected && !s.refused && s.error.is_none(), "{s:?}");
    assert_eq!(s.path, Some(PathKind::Direct), "no relay here, so the path is direct");
    assert!(s.rtt_ms.is_some() && s.remote.is_some() && s.since.is_some(), "{s:?}");
    let peers = home.ep.peers();
    assert_eq!(peers.len(), 1);
    assert_eq!((peers[0].id, peers[0].path), (key.id(), Some(PathKind::Direct)));
    assert!(home.ep.last_seen(&key.id()).is_some());
    assert!(home.ep.last_seen(&home.ep.id()).is_none());

    // a target home does not serve is closed, and the tunnel stays
    let other = Forwarder::bind("127.0.0.1:0".parse().unwrap(), Target::Hub).await.unwrap();
    let other_port = other.local_addr().unwrap();
    let t = m.tunnel.clone();
    tokio::spawn(other.run(move || {
        let t = t.clone();
        async move { t.open().await }
    }));
    assert_eq!(get(other_port).await, None);
    assert!(get(m.port).await.is_some());
    assert_eq!(m.ep.relay(), RelayState::default(), "no relay was set");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_websocket_echoes_through_the_endpoints() {
    let hub = ws_echo_server().await;
    let home = home(Targets { api: None, hub: Some(hub) }).await;
    let key = DeviceKey::generate();
    home.paired.add(device(key.id(), "laptop")).unwrap();
    let m = member_with(&key, home.peer(), Target::Hub).await;

    let tcp = TcpStream::connect(m.port).await.unwrap();
    let (mut ws, response) = tokio_tungstenite::client_async("ws://127.0.0.1:8132/node", tcp).await.unwrap();
    assert_eq!(response.status().as_u16(), 101);
    ws.send(Message::Text("typed".into())).await.unwrap();
    let got = timeout(LONG, ws.next()).await.unwrap().unwrap().unwrap();
    assert!(matches!(&got, Message::Text(t) if t.as_str() == "typed"), "{got:?}");
    let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    ws.send(Message::Binary(big.clone().into())).await.unwrap();
    let got = timeout(LONG, ws.next()).await.unwrap().unwrap().unwrap();
    assert!(matches!(&got, Message::Binary(b) if b[..] == big[..]));
    ws.close(None).await.unwrap();
    let end = timeout(LONG, ws.next()).await.unwrap();
    assert!(matches!(end, Some(Ok(Message::Close(_))) | None), "{end:?}");

    // the same thing with the stream in hand instead of a port: what the node's own link does
    let mut stream = m.tunnel.open().await.unwrap();
    crate::tunnel::write_header(&mut stream, Target::Hub).await.unwrap();
    let (mut ws, _) = tokio_tungstenite::client_async("ws://127.0.0.1:8136/node", stream).await.unwrap();
    ws.send(Message::Text("again".into())).await.unwrap();
    let got = timeout(LONG, ws.next()).await.unwrap().unwrap().unwrap();
    assert!(matches!(&got, Message::Text(t) if t.as_str() == "again"), "{got:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_tunnel_comes_back_after_home_restarts() {
    let (api, _) = http_server("first").await;
    let first = home(Targets { api: Some(api), hub: None }).await;
    let key = DeviceKey::generate();
    first.paired.add(device(key.id(), "laptop")).unwrap();
    let m = member_with(&key, first.peer(), Target::Api).await;
    assert!(get(m.port).await.unwrap().ends_with("first"));

    // home goes away: requests are closed at once, not left hanging
    let port = first.ep.addrs()[0].port();
    let HomeSide { key: home_key, paired, ep: first_ep, pairs: _ } = first;
    first_ep.close().await;
    drop(first_ep); // (the port is free again when the last handle to the endpoint is gone)
    until("the member notices", || !m.tunnel.status().connected).await;
    let began = Instant::now();
    assert_eq!(get(m.port).await, None);
    assert!(began.elapsed() < Duration::from_secs(8), "{:?}", began.elapsed());
    assert!(m.tunnel.status().error.is_some());

    // and comes back with the same key on the same port: the member finds it again by itself
    let (api, _) = http_server("second").await;
    let second = home_on(home_key, paired, Targets { api: Some(api), hub: None }, port).await;
    until("the member is back", || m.tunnel.status().connected).await;
    assert!(get(m.port).await.unwrap().ends_with("second"));
    assert_eq!(second.ep.peers().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn removing_a_device_cuts_it_off() {
    let (api, count) = http_server("hello").await;
    let hub = ws_echo_server().await;
    let home = home(Targets { api: Some(api), hub: Some(hub) }).await;
    let (key, stays) = (DeviceKey::generate(), DeviceKey::generate());
    home.paired.add(device(key.id(), "laptop")).unwrap();
    home.paired.add(device(stays.id(), "desktop")).unwrap();
    let m = member_with(&key, home.peer(), Target::Api).await;
    let other = member_with(&stays, home.peer(), Target::Api).await;
    assert!(get(m.port).await.is_some() && get(other.port).await.is_some());

    // a connection it holds open
    let mut stream = m.tunnel.open().await.unwrap();
    crate::tunnel::write_header(&mut stream, Target::Hub).await.unwrap();
    let (mut ws, _) = tokio_tungstenite::client_async("ws://127.0.0.1:8136/node", stream).await.unwrap();
    ws.send(Message::Text("still here".into())).await.unwrap();
    assert!(timeout(LONG, ws.next()).await.unwrap().unwrap().is_ok());

    assert!(home.paired.remove(&key.id()).unwrap());
    assert_eq!(home.ep.sweep(), 1, "only the removed device is swept");
    let end = timeout(LONG, ws.next()).await.expect("the open connection ends");
    assert!(!matches!(end, Some(Ok(Message::Text(_)))), "{end:?}");

    until("the removal shows", || m.tunnel.status().refused).await;
    let before = count.load(Ordering::Relaxed);
    assert_eq!(get(m.port).await, None, "and nothing new gets through");
    assert_eq!(count.load(Ordering::Relaxed), before);
    assert_eq!(home.ep.peers().iter().map(|p| p.id).collect::<Vec<_>>(), vec![stays.id()]);
    assert!(get(other.port).await.is_some(), "the other device is untouched");
    assert_eq!(home.ep.sweep(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_removed_device_is_stopped_at_its_next_stream_even_before_a_sweep() {
    let (api, count) = http_server("hello").await;
    let home = home(Targets { api: Some(api), hub: None }).await;
    let key = DeviceKey::generate();
    home.paired.add(device(key.id(), "laptop")).unwrap();
    let m = member_with(&key, home.peer(), Target::Api).await;
    assert!(get(m.port).await.is_some());
    assert!(home.paired.remove(&key.id()).unwrap());
    assert_eq!(get(m.port).await, None);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    until("the removal shows", || m.tunnel.status().refused).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_that_is_not_paired_can_reach_pairing_and_nothing_else() {
    let (api, count) = http_server("secret").await;
    let mut home = home(Targets { api: Some(api), hub: None }).await;
    let key = DeviceKey::generate();
    let opts = Options { bind: loopback(0), clocks: quick(), ..Options::default() };
    let joiner = LinkEndpoint::bind(&key, opts, Arc::new(Nobody)).await.unwrap();

    let offers = Offers::new();
    let (secret, _) = offers.mint_link(100);
    let out = joiner.pair_dial(&home.peer()).await.unwrap();
    assert_eq!((out.host, out.joiner), (home.ep.id(), key.id()));
    let out_channel = out.channel;

    let home_about = Home { id: home.key.id(), name: "studio".into(), relay: None };
    let hosting = async {
        // (a stream is only seen by the other side once something is written on it)
        let mut inc = timeout(LONG, home.pairs.recv()).await.unwrap().unwrap();
        assert_eq!((inc.host, inc.joiner), (home.ep.id(), key.id()));
        let channel = inc.channel;
        let binding = Binding { host: inc.host.0, joiner: inc.joiner.0, channel: &channel };
        let r = pairing::host(&mut inc.stream, &offers, Select::Link, &binding, &home_about, 101, |req| async move {
            assert_eq!(req.about.name, "laptop");
            Ok(Grant { machine: Some("laptop".into()), token: Some("tok".into()) })
        })
        .await;
        inc.finish().await;
        (r, channel)
    };
    let joining = async {
        let mut out = out;
        let binding = Binding { host: out.host.0, joiner: out.joiner.0, channel: &out_channel };
        let about = About { name: "laptop".into(), kind: Kind::Computer, os: "linux".into(), stays_with: None };
        let r = pairing::join(&mut out.stream, &Secret::Link(secret), &binding, &about).await;
        out.finish();
        r
    };
    let ((joined, in_channel), welcome) = tokio::join!(hosting, joining);
    assert_eq!(in_channel, out_channel, "both ends make the same channel value");
    assert_ne!(in_channel, [0u8; 32]);
    let (joined, welcome) = (joined.unwrap(), welcome.unwrap());
    assert_eq!(joined.id, key.id());
    assert_eq!(
        (welcome.home_id, welcome.machine.as_deref(), welcome.token.as_deref()),
        (home.key.id(), Some("laptop"), Some("tok"))
    );

    // pairing was all it could reach: a tunnel is still refused until home adds it
    let m = member_with(&key, home.peer(), Target::Api).await;
    assert_eq!(get(m.port).await, None);
    assert_eq!(count.load(Ordering::Relaxed), 0);
    home.paired.add(device(key.id(), "laptop")).unwrap();
    until("the tunnel is up", || m.tunnel.status().connected).await;
    assert!(get(m.port).await.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_that_is_refused_can_show_its_list_and_nothing_else() {
    let h = home(Targets::default()).await;
    let peer = h.peer();
    let opts = Options { bind: loopback(0), clocks: quick(), ..Options::default() };
    let stranger = LinkEndpoint::bind(&DeviceKey::generate(), opts, Arc::new(Nobody)).await.unwrap();

    // until the home computer says what it does with a shown list, nothing is read
    assert!(stranger.show(&peer, b"a list").await.is_err());

    let seen = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let keep = seen.clone();
    h.ep.on_shown(Arc::new(move |list: Vec<u8>| {
        let known = list == b"a list";
        keep.lock().unwrap().push(list);
        known.then(|| b"an answer".to_vec())
    }));
    assert_eq!(stranger.show(&peer, b"a list").await.unwrap(), b"an answer");
    assert_eq!(stranger.show(&peer, b"something else").await.unwrap(), b"", "no answer is an empty one");
    assert_eq!(*seen.lock().unwrap(), [b"a list".to_vec(), b"something else".to_vec()]);
    // more than a list can be is not handed over
    let _ = stranger.show(&peer, &vec![b'x'; LIST_BYTES + 1]).await;
    assert_eq!(seen.lock().unwrap().len(), 2);
    // and the tunnel stays shut to it
    let tunnel = Tunnel::new(stranger.clone(), peer);
    tokio::spawn(tunnel.clone().run());
    until("it is refused", || tunnel.status().refused).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_endpoint_that_takes_nothing_takes_nothing() {
    let key = DeviceKey::generate();
    let opts = Options { bind: loopback(0), clocks: quick(), ..Options::default() };
    let quiet = LinkEndpoint::bind(&key, opts.clone(), Arc::new(Nobody)).await.unwrap();
    let (tx, _rx) = mpsc::channel(1);
    let serving = quiet.clone();
    tokio::spawn(async move { serving.serve(Targets { api: None, hub: None }, tx).await });
    let caller = LinkEndpoint::bind(&DeviceKey::generate(), opts, Arc::new(Nobody)).await.unwrap();
    let peer = Peer { id: quiet.id(), relay: None, addrs: quiet.addrs() };
    assert!(caller.pair_dial(&peer).await.is_err(), "no protocol is offered, so the connection fails");
}

// ---- a computer that is not home, answering for itself to the devices on home's roster (phase 4) ---------------

use crate::roster::{self, Blocks, RosterCache, RosterDevice, RosterGate};

const ROSTER_T0: u64 = 1_795_000_000;

/// A member that takes tunnel connections from the devices on the roster it holds, names no address, and hands
/// its own streams over.
struct OwnSide {
    ep: Arc<LinkEndpoint>,
    gate: Arc<RosterGate>,
    owns: mpsc::Receiver<OwnIncoming>,
    clock: Arc<AtomicU64>,
}

impl OwnSide {
    fn peer(&self) -> Peer {
        Peer { id: self.ep.id(), relay: None, addrs: self.ep.addrs() }
    }
}

async fn own_side(home: &DeviceKey) -> OwnSide {
    let d = scratch("endpoint-own");
    let cache = Arc::new(RosterCache::load(&d.join("roster.json"), home.id()));
    let blocks = Arc::new(Blocks::load(&d.join("blocked.json")));
    let clock = Arc::new(AtomicU64::new(ROSTER_T0));
    let c = clock.clone();
    let gate = Arc::new(RosterGate::new(cache, blocks).with_clock(move || c.load(Ordering::Relaxed)));
    let opts = Options { bind: loopback(0), tunnel: true, clocks: quick(), ..Options::default() };
    let ep = LinkEndpoint::bind(&DeviceKey::generate(), opts, gate.clone()).await.unwrap();
    let (pairs, _) = mpsc::channel(1);
    let (tx, owns) = mpsc::channel(8);
    let serving = ep.clone();
    tokio::spawn(async move { serving.serve_with(Targets { api: None, hub: None }, pairs, Some(tx)).await });
    OwnSide { ep, gate, owns, clock }
}

fn listed(home: &DeviceKey, seq: u64, at: u64, ids: &[DeviceId]) -> roster::Envelope {
    let devices = ids
        .iter()
        .map(|id| RosterDevice {
            id: *id,
            name: id.short(),
            kind: Kind::Computer,
            machine: None,
            ver: None,
            own: None,
            addrs: vec![],
        })
        .collect();
    let body = roster::Body { home: home.id(), home_name: "Home".into(), epoch: 1, seq, at, relay: None, devices };
    roster::sign(home, body).unwrap().envelope
}

/// A device that dials `peer` and nothing else.
async fn caller(key: &DeviceKey, peer: Peer) -> Arc<Tunnel> {
    let opts = Options { bind: loopback(0), clocks: quick(), ..Options::default() };
    let ep = LinkEndpoint::bind(key, opts, Arc::new(Nobody)).await.unwrap();
    let tunnel = Tunnel::new(ep, peer);
    tokio::spawn(tunnel.clone().run());
    tunnel
}

/// Opens a stream for `target` and says what came back for one line sent: None when the stream was closed with
/// no answer.
async fn ask(tunnel: &Tunnel, target: Target) -> Option<Vec<u8>> {
    let mut s = tunnel.open().await.ok()?;
    crate::tunnel::write_header(&mut s, target).await.ok()?;
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n").await.ok()?;
    let mut out = Vec::new();
    match timeout(LONG, s.read_to_end(&mut out)).await {
        Ok(Ok(n)) if n > 0 => Some(out),
        _ => None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_off_the_roster_is_refused_by_a_member() {
    let home = DeviceKey::generate();
    let mut m = own_side(&home).await;
    let (on, off) = (DeviceKey::generate(), DeviceKey::generate());
    // with no roster at all, nobody
    let early = caller(&on, m.peer()).await;
    until("the refusal shows with no roster", || early.status().refused).await;
    assert!(m.gate.cache.offer(&listed(&home, 1, ROSTER_T0, &[on.id()]), ROSTER_T0).unwrap());
    let stranger = caller(&off, m.peer()).await;
    until("the refusal shows", || stranger.status().refused).await;
    assert!(stranger.status().error.unwrap().contains("not paired"), "closed with 403");
    assert!(ask(&stranger, Target::Own).await.is_none());
    // (the listed device's tunnel may have come back by now; the stranger never shows)
    assert!(m.ep.peers().iter().all(|p| p.id != off.id()));
    assert!(m.owns.try_recv().is_err(), "nothing of it was handed over");
    // even home's own key is a stranger when the roster does not list it
    let h = caller(&home, m.peer()).await;
    until("home is refused too", || h.status().refused).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_on_the_roster_gets_an_own_stream_and_no_address() {
    let home = DeviceKey::generate();
    let mut m = own_side(&home).await;
    let on = DeviceKey::generate();
    m.gate.cache.offer(&listed(&home, 1, ROSTER_T0, &[on.id()]), ROSTER_T0).unwrap();
    let t = caller(&on, m.peer()).await;
    until("connected", || t.status().connected).await;

    let mut s = t.open().await.unwrap();
    crate::tunnel::write_header(&mut s, Target::Own).await.unwrap();
    s.write_all(b"ping").await.unwrap();
    let mut inc = timeout(LONG, m.owns.recv()).await.unwrap().unwrap();
    assert_eq!(inc.caller, on.id(), "the stream comes with who opened it");
    let mut buf = [0u8; 4];
    inc.stream.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping", "and with nothing of the first line left in it");
    inc.stream.write_all(b"pong").await.unwrap();
    s.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"pong");

    // the targets that lead to an address are refused: a member names none, whatever listens on this computer
    let (api, count) = http_server("secret").await;
    let _ = api;
    assert!(ask(&t, Target::Api).await.is_none());
    assert!(ask(&t, Target::Hub).await.is_none());
    assert_eq!(count.load(Ordering::Relaxed), 0);
    assert!(m.owns.try_recv().is_err(), "and neither was handed over as an own stream");
    assert!(t.status().connected, "refusing a target does not end the connection");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_device_dropped_by_a_newer_roster_is_cut_at_once() {
    let home = DeviceKey::generate();
    let mut m = own_side(&home).await;
    let (goes, stays) = (DeviceKey::generate(), DeviceKey::generate());
    m.gate.cache.offer(&listed(&home, 1, ROSTER_T0, &[goes.id(), stays.id()]), ROSTER_T0).unwrap();
    let (a, b) = (caller(&goes, m.peer()).await, caller(&stays, m.peer()).await);
    until("both connected", || a.status().connected && b.status().connected).await;

    // a stream it holds open (a terminal, say)
    let mut held = a.open().await.unwrap();
    crate::tunnel::write_header(&mut held, Target::Own).await.unwrap();
    held.write_all(b"x").await.unwrap();
    let _inc = timeout(LONG, m.owns.recv()).await.unwrap().unwrap();

    assert!(m.gate.cache.offer(&listed(&home, 2, ROSTER_T0 + 5, &[stays.id()]), ROSTER_T0 + 5).unwrap());
    assert_eq!(m.ep.sweep(), 1, "only the dropped device");
    let mut buf = [0u8; 1];
    let end = timeout(LONG, held.read(&mut buf)).await.expect("the open stream ends");
    assert!(!matches!(end, Ok(n) if n > 0), "{end:?}");
    until("the removal shows", || a.status().refused).await;
    assert!(ask(&a, Target::Own).await.is_none());
    assert_eq!(m.ep.peers().iter().map(|p| p.id).collect::<Vec<_>>(), vec![stays.id()]);
    assert!(b.status().connected, "the other device is untouched");

    // the dropped device replays the roster that listed it: nothing changes
    assert_eq!(m.gate.cache.offer(&listed(&home, 1, ROSTER_T0, &[goes.id(), stays.id()]), ROSTER_T0 + 9), Ok(false));
    assert!(!m.gate.allows(&goes.id().0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dropped_or_blocked_device_is_stopped_at_its_next_stream_even_before_a_sweep() {
    let home = DeviceKey::generate();
    let mut m = own_side(&home).await;
    let (goes, blocked) = (DeviceKey::generate(), DeviceKey::generate());
    m.gate.cache.offer(&listed(&home, 1, ROSTER_T0, &[goes.id(), blocked.id()]), ROSTER_T0).unwrap();
    let (a, b) = (caller(&goes, m.peer()).await, caller(&blocked, m.peer()).await);
    until("both connected", || a.status().connected && b.status().connected).await;

    m.gate.blocks.block(blocked.id(), "x", m.gate.cache.stamp().unwrap(), ROSTER_T0).unwrap();
    assert!(ask(&b, Target::Own).await.is_none());
    until("the block shows", || b.status().refused).await;

    m.gate.cache.offer(&listed(&home, 1, ROSTER_T0 + 5, &[goes.id(), blocked.id()]), ROSTER_T0 + 5).unwrap();
    until("still connected", || a.status().connected).await;
    m.gate.cache.offer(&listed(&home, 2, ROSTER_T0 + 6, &[]), ROSTER_T0 + 6).unwrap();
    assert!(ask(&a, Target::Own).await.is_none());
    until("the removal shows", || a.status().refused).await;
    assert!(m.owns.try_recv().is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_roster_admits_nobody_at_the_handshake_and_cuts_who_is_in() {
    let home = DeviceKey::generate();
    let m = own_side(&home).await;
    let (first, late) = (DeviceKey::generate(), DeviceKey::generate());
    m.gate.cache.offer(&listed(&home, 1, ROSTER_T0, &[first.id(), late.id()]), ROSTER_T0).unwrap();
    let a = caller(&first, m.peer()).await;
    until("connected", || a.status().connected).await;
    // seven days and a second later
    m.clock.store(ROSTER_T0 + roster::HONOURED + 1, Ordering::Relaxed);
    let b = caller(&late, m.peer()).await;
    until("refused at the handshake", || b.status().refused).await;
    assert!(ask(&a, Target::Own).await.is_none(), "and the one already in gets no new stream");
    until("the first is out", || a.status().refused).await;
    assert_eq!(m.ep.sweep(), 0, "nothing is left to sweep");
}

#[test]
fn container_and_vm_networks_are_not_ways_in() {
    for name in
        ["docker0", "br-3f9a1c2b7d10", "veth1a2b3c", "virbr0", "vmnet8", "vboxnet0", "cni0", "lxcbr0", "podman1"]
    {
        assert!(inside_only(name), "{name}");
    }
    assert!(inside_only("vEthernet (WSL)") && inside_only("bridge100"));
    // The network a computer is really on, a VPN another computer shares, and a Mac's Thunderbolt bridge.
    for name in ["eth0", "enp3s0", "wlan0", "en0", "br0", "tailscale0", "utun4", "wg0", "bridge0", "Ethernet", "Wi-Fi"]
    {
        assert!(!inside_only(name), "{name}");
    }
}

#[test]
fn an_address_on_a_container_network_is_left_off_the_list() {
    let a = |s: &str| s.parse::<SocketAddr>().unwrap();
    let docker: Vec<IpAddr> = vec!["198.51.100.1".parse().unwrap(), "203.0.113.1".parse().unwrap()];
    let all = vec![a("198.51.100.1:41641"), a("203.0.113.1:41641"), a("192.0.2.20:41641"), a("[2001:db8::20]:41641")];
    assert_eq!(reachable(all, &docker), vec![a("192.0.2.20:41641"), a("[2001:db8::20]:41641")]);
    // An IPv4 address written the IPv6 way is the same address.
    assert_eq!(
        reachable(vec![a("[::ffff:198.51.100.1]:41641"), a("192.0.2.20:41641")], &docker),
        vec![a("192.0.2.20:41641")]
    );
    // Nothing else to offer: what there is stays.
    assert_eq!(reachable(vec![a("198.51.100.1:41641")], &docker), vec![a("198.51.100.1:41641")]);
    assert_eq!(reachable(vec![a("192.0.2.20:41641")], &[]), vec![a("192.0.2.20:41641")]);
}
