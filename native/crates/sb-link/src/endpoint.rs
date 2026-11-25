//! The endpoint: what carries pairing and the tunnel between two computers. The only file that names iroh.
//!
//! One endpoint per computer, with the computer's key (`identity`), so its id is the device id everything else
//! uses. It speaks two protocols:
//!
//! - `switchboard/tunnel/1`: streams to home's server and hub (`tunnel`). Only a paired device gets in. It is
//!   refused once when its connection arrives (`after_handshake`), again when the connection is taken, and again
//!   at every stream, each time by asking the paired list as it is then. A device that was removed is cut off
//!   without a restart, and the connections it had open are closed (`cut`, `sweep`).
//! - `switchboard/pair/1`: open to anyone who can reach the endpoint, because a device that is not paired yet
//!   has to get in somehow. What it reaches is the pairing exchange and nothing else, a few at a time.
//! - `switchboard/list/1`: open to anyone too, on the home computer only. A device that home refuses shows it
//!   the list of devices it holds, which is how a home computer brought back from an old backup hears of the
//!   home computer that replaced it (`show`, `on_shown`). One bounded message each way, a few a minute.
//!
//! What this file never does: use anyone's servers but the relay it is given. `presets::Minimal` sets the
//! cipher suite provider and nothing more; the relay is `RelayMode::custom` or off; there is no DNS or pkarr
//! lookup, no mDNS, and no port mapping (UPnP and friends are what raise firewall and privacy prompts). It never
//! waits to be "online" either: a dial made straight after start works, and waiting costs three seconds.
//!
//! The dialling side (`Tunnel`) keeps one connection to home and makes it again, with growing waits, when it
//! ends. A path that dies without a word would otherwise hang every request for the 30 seconds QUIC takes to give
//! up: a stream that has had no answer for 2 seconds on a connection that has heard nothing at all for 10 closes
//! the connection, and the next request dials again.

use crate::identity::{DeviceId, DeviceKey};
use crate::paired::Paired;
use crate::tunnel::{self, PipeOptions, Target, Targets};
use iroh::endpoint::{
    AfterHandshakeOutcome, Connection, EndpointHooks, PortmapperConfig, QuicTransportConfig, RecvStream, SendStream,
    Side, presets,
};
use iroh::{Endpoint, EndpointAddr, EndpointId, RelayMode, RelayUrl, SecretKey, Watcher};
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{Semaphore, mpsc, watch};

pub const ALPN_TUNNEL: &[u8] = b"switchboard/tunnel/1";
pub const ALPN_PAIR: &[u8] = b"switchboard/pair/1";
/// A device the home computer refuses shows it the list of devices it holds, and nothing else (`show`).
pub const ALPN_LIST: &[u8] = b"switchboard/list/1";
/// Lists shown in a minute, over everyone.
pub const LIST_PER_MINUTE: u32 = 30;
/// The most a shown list, or the answer to one, may be.
pub const LIST_BYTES: usize = 256 * 1024;
/// Streams one device may have open at once.
pub const MAX_STREAMS: u32 = 256;
/// Pairing connections taken in a minute, over everyone; more are turned away until the minute is over.
pub const PAIR_PER_MINUTE: u32 = 30;
/// Pairing exchanges running at once.
pub const PAIR_AT_ONCE: usize = 8;

/// Why a connection was closed, as the other side reads it.
pub const CLOSE_NOT_PAIRED: u32 = 403;
pub const CLOSE_REMOVED: u32 = 410;
pub const CLOSE_SILENT: u32 = 408;
pub const CLOSE_BUSY: u32 = 429;
pub const CLOSE_DONE: u32 = 0;

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Android only: gives the endpoint's name lookups the app's Java side, once, before the first `bind`.
///
/// A relay is named by a host name, and the endpoint looks it up with the phone's own DNS servers. On Android
/// those are not in a file: they are read from the system's ConnectivityManager, which takes the Java VM and the
/// application's Context. Without them the lookup quietly goes to public resolvers instead, which is both a
/// promise broken (this file uses nobody's servers but the relay it is given) and a relay on a private name
/// (a tailnet, a home router's own names) that is never found.
///
/// # Safety
///
/// `java_vm` is the process's `JavaVM*` and `context` a JNI global reference to the application Context; both
/// stay valid for the life of the process. Called at most once per process.
#[cfg(target_os = "android")]
pub unsafe fn install_android_context(java_vm: *mut std::ffi::c_void, context: *mut std::ffi::c_void) {
    unsafe { iroh::dns::install_android_jni_context(java_vm, context) }
}

fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// Who may open a tunnel. Asked every time, never remembered.
pub trait Gate: Send + Sync + 'static {
    fn allows(&self, id: &[u8; 32]) -> bool;
}

impl Gate for Paired {
    fn allows(&self, id: &[u8; 32]) -> bool {
        Paired::allows(self, id)
    }
}

/// The gate of a computer that accepts no tunnels (every computer but home).
pub struct Nobody;

impl Gate for Nobody {
    fn allows(&self, _: &[u8; 32]) -> bool {
        false
    }
}

/// The clocks an endpoint runs by. The defaults are the measured ones; tests shorten them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clocks {
    /// How often an idle connection says it is still there. iroh's own 5 seconds: do not raise it.
    pub keep_alive: Duration,
    /// A stream that has had no byte back for this long ...
    pub first_byte: Duration,
    /// ... on a connection that has heard nothing at all for this long, means the path is dead.
    pub silent: Duration,
    /// The wait before the first redial, doubled after each failure up to `backoff_max`. The longest wait is how
    /// long a computer that came back can go unnoticed, on top of one dial, so it is kept short: a dial is a few
    /// packets.
    pub backoff_first: Duration,
    pub backoff_max: Duration,
    /// How long one dial may take.
    pub dial: Duration,
}

impl Default for Clocks {
    fn default() -> Clocks {
        Clocks {
            keep_alive: Duration::from_secs(5),
            first_byte: Duration::from_secs(2),
            silent: Duration::from_secs(10),
            backoff_first: Duration::from_secs(1),
            backoff_max: Duration::from_secs(10),
            dial: Duration::from_secs(15),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The relay this endpoint is reached through. None: no relay at all, direct addresses only.
    pub relay: Option<String>,
    /// One UDP socket on this address instead of every interface (tests use loopback).
    pub bind: Option<SocketAddr>,
    /// No UDP at all: every path goes through the relay.
    pub relay_only: bool,
    /// Take tunnel connections (home).
    pub tunnel: bool,
    /// Take pairing connections (home, and the short code's temporary endpoint).
    pub pair: bool,
    pub clocks: Clocks,
}

/// What the home computer does with a list of devices it is shown by a device it refuses: the bytes are the
/// list as that device holds it. Some: what is sent back. It is called for anyone who can reach the endpoint,
/// so it must believe nothing it cannot check, and give nothing away to a caller it cannot place.
pub type Shown = Arc<dyn Fn(Vec<u8>) -> Option<Vec<u8>> + Send + Sync>;

/// Refuses a tunnel from a device that is not paired, as soon as its connection is up.
struct GateHook(Arc<dyn Gate>);

impl std::fmt::Debug for GateHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GateHook")
    }
}

impl EndpointHooks for GateHook {
    async fn after_handshake(&self, conn: &Connection) -> AfterHandshakeOutcome {
        // (a connection this side dialled is to the id it dialled: only the ones coming in are asked about)
        if conn.side() == Side::Server && conn.alpn() == ALPN_TUNNEL && !self.0.allows(conn.remote_id().as_bytes()) {
            return AfterHandshakeOutcome::Reject {
                error_code: CLOSE_NOT_PAIRED.into(),
                reason: b"not paired".to_vec(),
            };
        }
        AfterHandshakeOutcome::Accept
    }
}

/// At most `max` in each window of `per`.
struct Rate {
    max: u32,
    per: Duration,
    state: Mutex<(Instant, u32)>,
}

impl Rate {
    fn new(max: u32, per: Duration) -> Rate {
        Rate { max, per, state: Mutex::new((Instant::now(), 0)) }
    }

    fn take(&self, now: Instant) -> bool {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if now.duration_since(s.0) >= self.per {
            *s = (now, 0);
        }
        if s.1 >= self.max {
            return false;
        }
        s.1 += 1;
        true
    }
}

/// The streams that are waiting for their first byte back, by when they were opened.
#[derive(Default)]
struct Waiting {
    next: AtomicU64,
    since: Mutex<HashMap<u64, Instant>>,
}

impl Waiting {
    fn add(self: &Arc<Self>) -> Waits {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.since.lock().unwrap_or_else(|e| e.into_inner()).insert(id, Instant::now());
        Waits { id, list: self.clone() }
    }

    fn oldest(&self) -> Option<Instant> {
        self.since.lock().unwrap_or_else(|e| e.into_inner()).values().min().copied()
    }
}

/// One stream's place in `Waiting`; dropping it (the first byte came, or the stream ended) takes it out.
struct Waits {
    id: u64,
    list: Arc<Waiting>,
}

impl Drop for Waits {
    fn drop(&mut self) {
        self.list.since.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}

/// Whether a path is dead: a stream has waited longer than `first_byte` and nothing has arrived for `silent`.
fn dead(oldest_wait: Option<Duration>, since_heard: Duration, clocks: &Clocks) -> bool {
    oldest_wait.is_some_and(|w| w > clocks.first_byte) && since_heard > clocks.silent
}

/// One stream between two endpoints, as one thing that reads and writes. Shutting it down says "no more from
/// this side" and leaves the other direction open, as `tunnel::pipe` needs.
pub struct LinkStream {
    send: SendStream,
    recv: RecvStream,
    waits: Option<Waits>,
}

impl LinkStream {
    fn new(send: SendStream, recv: RecvStream) -> LinkStream {
        LinkStream { send, recv, waits: None }
    }
}

impl AsyncRead for LinkStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let r = AsyncRead::poll_read(Pin::new(&mut self.recv), cx, buf);
        if r.is_ready() {
            self.waits = None; // an answer, the end, or an error: no longer waiting on a silent path
        }
        r
    }
}

impl AsyncWrite for LinkStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        AsyncWrite::poll_write(Pin::new(&mut self.send), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_flush(Pin::new(&mut self.send), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        AsyncWrite::poll_shutdown(Pin::new(&mut self.send), cx)
    }
}

/// How a connection reaches the other side right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKind {
    /// Any path that is not the relay: the same network, a tailnet, or through the routers.
    Direct,
    Relay,
}

impl PathKind {
    pub fn name(self) -> &'static str {
        match self {
            PathKind::Direct => "direct",
            PathKind::Relay => "relay",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathStatus {
    pub connected: bool,
    pub path: Option<PathKind>,
    pub rtt_ms: Option<u64>,
    /// The other side's address, when the path is direct.
    pub remote: Option<SocketAddr>,
    /// When this connection was made, in seconds since 1970.
    pub since: Option<u64>,
    /// Why there is no connection, in the endpoint's words.
    pub error: Option<String>,
    /// The other side said this device is not paired with it (any more).
    pub refused: bool,
}

fn path_of(conn: &Connection) -> (Option<PathKind>, Option<u64>, Option<SocketAddr>) {
    let paths = conn.paths();
    let Some(p) = paths.iter().find(|p| p.is_selected()) else { return (None, None, None) };
    let kind = if p.is_relay() { PathKind::Relay } else { PathKind::Direct };
    let remote = match p.remote_addr() {
        iroh::TransportAddr::Ip(a) => Some(*a),
        _ => None,
    };
    (Some(kind), Some(p.rtt().as_millis() as u64), remote)
}

/// Everything that has arrived on a connection so far, as a count that only grows.
fn heard(conn: &Connection) -> u64 {
    conn.stats().udp_rx.datagrams
}

/// A paired device that is connected to this endpoint now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerStatus {
    pub id: DeviceId,
    pub path: Option<PathKind>,
    pub rtt_ms: Option<u64>,
    pub since: u64,
    /// Where it is connecting from, when its path is a direct one.
    pub remote: Option<SocketAddr>,
}

/// What this endpoint knows of its relay.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayState {
    pub url: Option<String>,
    pub connected: bool,
    /// The last failure to connect, in the endpoint's words.
    pub error: Option<String>,
    /// The relay said another endpoint is connected with this same key: a copied disk, or two nodes at once.
    pub same_id: bool,
}

/// A device that connected to pair, with its one stream.
pub struct PairIncoming {
    pub stream: LinkStream,
    /// The id this endpoint answered as (home's own, or the short code's).
    pub host: DeviceId,
    pub joiner: DeviceId,
    /// A value only the two ends of this connection can make (the TLS exporter): `pairing::Binding::channel`.
    pub channel: [u8; 32],
    conn: Connection,
    _place: tokio::sync::OwnedSemaphorePermit,
}

impl PairIncoming {
    /// After the exchange: says nothing more is coming, and waits a moment for the device to have read the
    /// answer before the connection goes.
    pub async fn finish(mut self) {
        use tokio::io::AsyncWriteExt;
        let _ = self.stream.shutdown().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.conn.closed()).await;
        self.conn.close(CLOSE_DONE.into(), b"done");
    }
}

/// This side's connection to an endpoint it wants to pair with.
pub struct PairOutgoing {
    pub stream: LinkStream,
    pub host: DeviceId,
    pub joiner: DeviceId,
    pub channel: [u8; 32],
    conn: Connection,
}

impl PairOutgoing {
    pub fn finish(self) {
        self.conn.close(CLOSE_DONE.into(), b"done");
    }
}

/// A stream a device opened to this computer's own surface (`{"to":"own"}`), with its first line already read.
/// The device passed the gate when it connected and again for this stream.
pub struct OwnIncoming {
    pub stream: LinkStream,
    pub caller: DeviceId,
}

const EXPORT_LABEL: &[u8] = b"switchboard pair 1 channel";

fn channel_of(conn: &Connection) -> io::Result<[u8; 32]> {
    let mut out = [0u8; 32];
    conn.export_keying_material(&mut out, EXPORT_LABEL, b"").map_err(|_| other("no channel value"))?;
    Ok(out)
}

struct Live {
    conn: Connection,
    since: u64,
}

/// Where another endpoint is: its id, the relay it is reached through, and addresses it was last seen at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Peer {
    pub id: DeviceId,
    pub relay: Option<String>,
    pub addrs: Vec<SocketAddr>,
}

fn addr_of(peer: &Peer) -> io::Result<EndpointAddr> {
    let id = EndpointId::from_bytes(peer.id.as_bytes()).map_err(|_| other("that id is not a key"))?;
    let mut addr = EndpointAddr::new(id);
    if let Some(r) = &peer.relay {
        addr = addr.with_relay_url(r.parse::<RelayUrl>().map_err(|e| other(format!("relay {r}: {e}")))?);
    }
    // A Mac's background programs may not start traffic to an address on the local network, and the attempt
    // fails without a word (plan section 7, point 1). With a relay to start on, the stored addresses are left
    // out there, and the direct path forms when the other side sends first.
    let skip = cfg!(target_os = "macos") && peer.relay.is_some();
    if !skip {
        for a in &peer.addrs {
            addr = addr.with_ip_addr(*a);
        }
    }
    if addr.is_empty() {
        return Err(other("no relay is set and no address is known for the other computer"));
    }
    Ok(addr)
}

pub struct LinkEndpoint {
    ep: Endpoint,
    id: DeviceId,
    relay: Option<String>,
    gate: Arc<dyn Gate>,
    clocks: Clocks,
    live: Mutex<HashMap<DeviceId, Vec<Live>>>,
    seen: Mutex<HashMap<DeviceId, u64>>,
    pair_rate: Rate,
    pair_room: Arc<Semaphore>,
    list_rate: Rate,
    shown: Mutex<Option<Shown>>,
}

impl LinkEndpoint {
    /// An endpoint with `key`, listening. It does not wait for the relay.
    pub async fn bind(key: &DeviceKey, opts: Options, gate: Arc<dyn Gate>) -> io::Result<Arc<LinkEndpoint>> {
        watch_log();
        let transport = QuicTransportConfig::builder()
            .max_concurrent_bidi_streams(MAX_STREAMS.into())
            .keep_alive_interval(opts.clocks.keep_alive)
            .build();
        let relay_mode = match &opts.relay {
            Some(r) => RelayMode::custom([r.parse::<RelayUrl>().map_err(|e| other(format!("relay {r}: {e}")))?]),
            None => RelayMode::Disabled,
        };
        let mut alpns = vec![];
        if opts.tunnel {
            alpns.push(ALPN_TUNNEL.to_vec());
            alpns.push(ALPN_LIST.to_vec());
        }
        if opts.pair {
            alpns.push(ALPN_PAIR.to_vec());
        }
        let mut b = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(key.secret_bytes()))
            .alpns(alpns)
            .relay_mode(relay_mode)
            .transport_config(transport)
            .portmapper_config(PortmapperConfig::Disabled)
            .hooks(GateHook(gate.clone()));
        if opts.relay_only {
            b = b.clear_ip_transports();
        } else if let Some(addr) = opts.bind {
            b = b.clear_ip_transports().bind_addr(addr).map_err(other)?;
        }
        let ep = b.bind().await.map_err(other)?;
        Ok(Arc::new(LinkEndpoint {
            ep,
            id: key.id(),
            relay: opts.relay,
            gate,
            clocks: opts.clocks,
            live: Mutex::default(),
            seen: Mutex::default(),
            pair_rate: Rate::new(PAIR_PER_MINUTE, Duration::from_secs(60)),
            pair_room: Arc::new(Semaphore::new(PAIR_AT_ONCE)),
            list_rate: Rate::new(LIST_PER_MINUTE, Duration::from_secs(60)),
            shown: Mutex::default(),
        }))
    }

    pub fn id(&self) -> DeviceId {
        self.id
    }

    /// The addresses this endpoint can be reached at without the relay, as far as it knows.
    ///
    /// An address on a container, bridge or virtual machine network (Docker's `docker0` and `br-…`, libvirt's
    /// `virbr0`, a Hyper-V switch) is left out: no other computer can reach this one there, and only a few addresses
    /// fit on the list of devices and in a pairing link, so one of those took the place of the address that works.
    pub fn addrs(&self) -> Vec<SocketAddr> {
        let mut v: Vec<SocketAddr> = self.ep.addr().ip_addrs().copied().collect();
        for a in self.ep.bound_sockets() {
            if !a.ip().is_unspecified() && !v.contains(&a) {
                v.push(a);
            }
        }
        let inside: Vec<IpAddr> = netdev::get_interfaces()
            .iter()
            .filter(|i| inside_only(&i.name) || i.friendly_name.as_deref().is_some_and(inside_only))
            .flat_map(|i| {
                i.ipv4.iter().map(|n| IpAddr::V4(n.addr())).chain(i.ipv6.iter().map(|n| IpAddr::V6(n.addr())))
            })
            .collect();
        reachable(v, &inside)
    }

    pub fn relay(&self) -> RelayState {
        let Some(url) = &self.relay else { return RelayState::default() };
        let mut w = self.ep.home_relay_status();
        let all = w.get();
        let connected = all.iter().any(|s| s.is_connected());
        let error = if connected { None } else { all.iter().find_map(|s| s.last_error().map(|e| format!("{e:#}"))) };
        RelayState { url: Some(url.clone()), connected, error, same_id: connected && SAME_ID.load(Ordering::Relaxed) }
    }

    /// Waits until the relay has this endpoint, or `most` has passed. For a short code only: the code is no use
    /// before the relay knows the id it leads to.
    pub async fn relay_ready(&self, most: Duration) -> bool {
        let wait = async {
            while !self.relay().connected {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        tokio::time::timeout(most, wait).await.is_ok()
    }

    pub async fn close(&self) {
        self.ep.close().await;
    }

    // ---- home's side ----------------------------------------------------------------------------------------

    /// Takes connections until the endpoint closes. Tunnel streams go to `targets`; pairing connections are
    /// handed to `pairs`, and turned away when nobody is there to take them.
    pub async fn serve(self: &Arc<Self>, targets: Targets, pairs: mpsc::Sender<PairIncoming>) {
        self.serve_with(targets, pairs, None).await
    }

    /// `serve`, and streams for this computer's own surface are handed to `owns`. Without it (or when nobody
    /// takes them) such a stream is dropped, which the other side sees as its connection closing.
    pub async fn serve_with(
        self: &Arc<Self>,
        targets: Targets,
        pairs: mpsc::Sender<PairIncoming>,
        owns: Option<mpsc::Sender<OwnIncoming>>,
    ) {
        let targets = Arc::new(targets);
        while let Some(incoming) = self.ep.accept().await {
            let (me, targets, pairs, owns) = (self.clone(), targets.clone(), pairs.clone(), owns.clone());
            tokio::spawn(async move {
                let Ok(Ok(conn)) = tokio::time::timeout(Duration::from_secs(10), incoming).await else { return };
                if conn.alpn() == ALPN_TUNNEL {
                    me.serve_tunnel(conn, &targets, owns.as_ref()).await;
                } else if conn.alpn() == ALPN_PAIR {
                    me.take_pair(conn, &pairs).await;
                } else if conn.alpn() == ALPN_LIST {
                    me.take_shown(conn).await;
                } else {
                    conn.close(CLOSE_NOT_PAIRED.into(), b"unknown protocol");
                }
            });
        }
    }

    async fn serve_tunnel(
        self: &Arc<Self>,
        conn: Connection,
        targets: &Arc<Targets>,
        owns: Option<&mpsc::Sender<OwnIncoming>>,
    ) {
        let id = DeviceId(*conn.remote_id().as_bytes());
        if !self.gate.allows(&id.0) {
            return conn.close(CLOSE_NOT_PAIRED.into(), b"not paired");
        }
        let stable = conn.stable_id();
        self.live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(id)
            .or_default()
            .push(Live { conn: conn.clone(), since: unix_now() });
        self.touch(id);
        while let Ok((send, recv)) = conn.accept_bi().await {
            // asked again at every stream: a device removed a moment ago gets nothing more
            if !self.gate.allows(&id.0) {
                // (every connection it has, and off the list of who is connected before any of them closes)
                self.cut(&id);
                conn.close(CLOSE_REMOVED.into(), b"removed");
                break;
            }
            self.touch(id);
            let (targets, owns) = (targets.clone(), owns.cloned());
            tokio::spawn(async move {
                let mut stream = LinkStream::new(send, recv);
                let Ok(target) = tunnel::read_header(&mut stream).await else { return };
                if target == Target::Own {
                    // (a full queue drops the stream: the caller's request fails and it asks again)
                    if let Some(owns) = owns {
                        let _ = owns.try_send(OwnIncoming { stream, caller: id });
                    }
                    return;
                }
                let _ = tunnel::serve_to(stream, target, &targets, &PipeOptions::default()).await;
            });
        }
        self.touch(id);
        let mut live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(list) = live.get_mut(&id) {
            list.retain(|l| l.conn.stable_id() != stable);
            if list.is_empty() {
                live.remove(&id);
            }
        }
    }

    fn touch(&self, id: DeviceId) {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).insert(id, unix_now());
    }

    async fn take_pair(self: &Arc<Self>, conn: Connection, pairs: &mpsc::Sender<PairIncoming>) {
        let busy = |conn: &Connection| conn.close(CLOSE_BUSY.into(), b"busy");
        if !self.pair_rate.take(Instant::now()) {
            return busy(&conn);
        }
        let Ok(place) = self.pair_room.clone().try_acquire_owned() else { return busy(&conn) };
        let Ok(Ok((send, recv))) = tokio::time::timeout(Duration::from_secs(10), conn.accept_bi()).await else {
            return busy(&conn);
        };
        let Ok(channel) = channel_of(&conn) else { return busy(&conn) };
        let incoming = PairIncoming {
            stream: LinkStream::new(send, recv),
            host: self.id,
            joiner: DeviceId(*conn.remote_id().as_bytes()),
            channel,
            conn: conn.clone(),
            _place: place,
        };
        if pairs.try_send(incoming).is_err() {
            busy(&conn);
        }
    }

    /// What is done with a list of devices shown by a device this endpoint refuses. Until it is set, such a
    /// connection is closed with nothing read.
    pub fn on_shown(&self, shown: Shown) {
        *self.shown.lock().unwrap_or_else(|e| e.into_inner()) = Some(shown);
    }

    /// One shown list: read whole, handed over, answered, closed. Anyone can get this far, so it is a few a
    /// minute, of a bounded size, and each has ten seconds.
    async fn take_shown(self: &Arc<Self>, conn: Connection) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let shown = self.shown.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some(shown) = shown.filter(|_| self.list_rate.take(Instant::now())) else {
            return conn.close(CLOSE_BUSY.into(), b"busy");
        };
        let exchange = async {
            let (send, recv) = conn.accept_bi().await.ok()?;
            let mut stream = LinkStream::new(send, recv);
            let mut body = Vec::new();
            (&mut stream).take(LIST_BYTES as u64 + 1).read_to_end(&mut body).await.ok()?;
            if body.len() > LIST_BYTES {
                return None;
            }
            let answer = shown(body).unwrap_or_default();
            stream.write_all(&answer).await.ok()?;
            stream.shutdown().await.ok()?;
            // (the other side closes once it has read the answer)
            conn.closed().await;
            Some(())
        };
        let _ = tokio::time::timeout(Duration::from_secs(10), exchange).await;
        conn.close(CLOSE_DONE.into(), b"done");
    }

    /// Closes every connection `id` has open. How many there were.
    pub fn cut(&self, id: &DeviceId) -> usize {
        let gone = self.live.lock().unwrap_or_else(|e| e.into_inner()).remove(id).unwrap_or_default();
        for l in &gone {
            l.conn.close(CLOSE_REMOVED.into(), b"removed");
        }
        gone.len()
    }

    /// Closes the connections of every device the gate no longer allows. How many devices that was.
    pub fn sweep(&self) -> usize {
        let ids: Vec<DeviceId> = self.live.lock().unwrap_or_else(|e| e.into_inner()).keys().copied().collect();
        ids.iter().filter(|id| !self.gate.allows(&id.0) && self.cut(id) > 0).count()
    }

    /// The paired devices connected now.
    pub fn peers(&self) -> Vec<PeerStatus> {
        let live = self.live.lock().unwrap_or_else(|e| e.into_inner());
        live.iter()
            .filter_map(|(id, list)| {
                let l = list.iter().rfind(|l| l.conn.close_reason().is_none())?;
                let (path, rtt_ms, remote) = path_of(&l.conn);
                let remote = remote.filter(|_| path == Some(PathKind::Direct));
                Some(PeerStatus { id: *id, path, rtt_ms, since: l.since, remote })
            })
            .collect()
    }

    /// When `id` last had a connection or opened a stream, in seconds since 1970. Kept in memory only.
    pub fn last_seen(&self, id: &DeviceId) -> Option<u64> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).get(id).copied()
    }

    // ---- the joining side -----------------------------------------------------------------------------------

    /// Connects to `peer` to pair with it and opens the one stream the exchange runs on.
    pub async fn pair_dial(&self, peer: &Peer) -> io::Result<PairOutgoing> {
        let addr = addr_of(peer)?;
        let conn = tokio::time::timeout(self.clocks.dial, self.ep.connect(addr, ALPN_PAIR))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the other computer did not answer"))?
            .map_err(|e| other(format!("{e:#}")))?;
        let (send, recv) = conn.open_bi().await.map_err(other)?;
        let channel = channel_of(&conn)?;
        Ok(PairOutgoing { stream: LinkStream::new(send, recv), host: peer.id, joiner: self.id, channel, conn })
    }

    /// Shows `peer` (the home computer, which refuses this device) the list of devices this device holds, and
    /// returns what it sent back: nothing, or the list it holds itself.
    pub async fn show(&self, peer: &Peer, list: &[u8]) -> io::Result<Vec<u8>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let addr = addr_of(peer)?;
        let exchange = async {
            let conn = self.ep.connect(addr, ALPN_LIST).await.map_err(|e| other(format!("{e:#}")))?;
            let (send, recv) = conn.open_bi().await.map_err(other)?;
            let mut stream = LinkStream::new(send, recv);
            stream.write_all(list).await?;
            stream.shutdown().await?;
            let mut answer = Vec::new();
            (&mut stream).take(LIST_BYTES as u64 + 1).read_to_end(&mut answer).await?;
            conn.close(CLOSE_DONE.into(), b"done");
            if answer.len() > LIST_BYTES {
                return Err(other("the answer was too long"));
            }
            Ok(answer)
        };
        tokio::time::timeout(self.clocks.dial, exchange)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the other computer did not answer"))?
    }
}

#[derive(Default)]
struct DialState {
    error: Option<String>,
    refused: bool,
    since: Option<u64>,
}

/// The dialling side's one connection to home, kept up.
pub struct Tunnel {
    ep: Arc<LinkEndpoint>,
    peer: Mutex<Peer>,
    conn: watch::Sender<Option<Connection>>,
    state: Mutex<DialState>,
    waiting: Arc<Waiting>,
}

impl Tunnel {
    pub fn new(ep: Arc<LinkEndpoint>, peer: Peer) -> Arc<Tunnel> {
        Arc::new(Tunnel {
            ep,
            peer: Mutex::new(peer),
            conn: watch::channel(None).0,
            state: Mutex::default(),
            waiting: Arc::default(),
        })
    }

    pub fn peer(&self) -> Peer {
        self.peer.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Addresses to try next time, from where the other side was last seen.
    pub fn set_addrs(&self, addrs: Vec<SocketAddr>) {
        self.peer.lock().unwrap_or_else(|e| e.into_inner()).addrs = addrs;
    }

    fn current(&self) -> Option<Connection> {
        self.conn.borrow().as_ref().filter(|c| c.close_reason().is_none()).cloned()
    }

    /// Keeps the connection up for as long as this runs: dials, watches the connection, and dials again when it
    /// ends, waiting longer after each failure.
    pub async fn run(self: Arc<Self>) {
        let clocks = self.ep.clocks.clone();
        let mut wait = clocks.backoff_first;
        loop {
            let began = Instant::now();
            let dialled = match addr_of(&self.peer()) {
                Ok(addr) => match tokio::time::timeout(clocks.dial, self.ep.ep.connect(addr, ALPN_TUNNEL)).await {
                    Ok(Ok(conn)) => Ok(conn),
                    Ok(Err(e)) => Err(format!("{e:#}")),
                    Err(_) => Err("the other computer did not answer".to_string()),
                },
                Err(e) => Err(e.to_string()),
            };
            match dialled {
                Ok(conn) => {
                    {
                        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
                        *s = DialState { error: None, refused: false, since: Some(unix_now()) };
                    }
                    self.conn.send_replace(Some(conn.clone()));
                    let why = self.watch(&conn, &clocks).await;
                    self.conn.send_replace(None);
                    self.failed(why);
                }
                Err(why) => self.failed(why),
            }
            // a connection that lasted starts the waits over
            wait = if began.elapsed() > clocks.backoff_max * 2 { clocks.backoff_first } else { wait };
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(clocks.backoff_max);
        }
    }

    fn failed(&self, why: String) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.refused = why.contains("not paired") || why.contains("removed");
        s.error = Some(why);
        s.since = None;
    }

    /// Returns why the connection ended. Ends it itself when its path has gone silent.
    async fn watch(&self, conn: &Connection, clocks: &Clocks) -> String {
        let mut tick = tokio::time::interval((clocks.silent / 8).max(Duration::from_millis(50)));
        let (mut count, mut last) = (heard(conn), Instant::now());
        loop {
            tokio::select! {
                e = conn.closed() => return e.to_string(),
                _ = tick.tick() => {}
            }
            let now = heard(conn);
            if now != count {
                (count, last) = (now, Instant::now());
            }
            if dead(self.waiting.oldest().map(|t| t.elapsed()), last.elapsed(), clocks) {
                conn.close(CLOSE_SILENT.into(), b"silent");
                return "the connection went silent".into();
            }
        }
    }

    /// A new stream to the other side, with nothing written on it yet. Fails at once when there is no
    /// connection and none is made within a few seconds: the caller's own connection is then closed, as it
    /// would be by any server that is away.
    pub async fn open(&self) -> io::Result<LinkStream> {
        let conn = match self.current() {
            Some(c) => c,
            None => {
                let mut rx = self.conn.subscribe();
                let found = async {
                    loop {
                        if let Some(c) = rx.borrow_and_update().as_ref().filter(|c| c.close_reason().is_none()) {
                            return Some(c.clone());
                        }
                        if rx.changed().await.is_err() {
                            return None;
                        }
                    }
                };
                match tokio::time::timeout(Duration::from_secs(3), found).await {
                    Ok(Some(c)) => c,
                    _ => {
                        let why = self.state.lock().unwrap_or_else(|e| e.into_inner()).error.clone();
                        return Err(io::Error::new(
                            io::ErrorKind::NotConnected,
                            why.unwrap_or_else(|| "not connected to the home computer yet".into()),
                        ));
                    }
                }
            }
        };
        let (send, recv) = conn.open_bi().await.map_err(other)?;
        Ok(LinkStream { send, recv, waits: Some(self.waiting.add()) })
    }

    pub fn status(&self) -> PathStatus {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match self.current() {
            Some(conn) => {
                let (path, rtt_ms, remote) = path_of(&conn);
                PathStatus { connected: true, path, rtt_ms, remote, since: s.since, error: None, refused: false }
            }
            None => PathStatus { error: s.error.clone(), refused: s.refused, ..PathStatus::default() },
        }
    }

    /// Ends the connection now (the tunnel is being taken down).
    pub fn close(&self) {
        if let Some(c) = self.conn.send_replace(None) {
            c.close(CLOSE_DONE.into(), b"closing");
        }
    }
}

// ---- the relay's warnings -------------------------------------------------------------------------------------

/// Set while the relay says another endpoint holds this key's place.
static SAME_ID: AtomicBool = AtomicBool::new(false);
const SAME_ID_TEXT: &str = "Another endpoint connected with the same endpoint id";

/// iroh only says in its log that the relay found this key in use twice. This listens for that line (and for
/// the one that says it is over), and with SWITCHBOARD_NODE_DEBUG set prints iroh's warnings to stderr.
fn watch_log() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let loud = std::env::var_os("SWITCHBOARD_NODE_DEBUG").is_some_and(|v| !v.is_empty());
        let _ = tracing::subscriber::set_global_default(LogWatch { loud, spans: AtomicU64::new(1) });
    });
}

struct LogWatch {
    loud: bool,
    spans: AtomicU64,
}

fn relay_line(target: &str) -> bool {
    target.contains("relay")
}

/// What a log line means for `SAME_ID`: Some(true) the key is in use twice, Some(false) the relay is fine again.
fn same_id_from(target: &str, message: &str) -> Option<bool> {
    if !relay_line(target) {
        return None;
    }
    if message.contains(SAME_ID_TEXT) {
        Some(true)
    } else if message.contains("Relay server reports: ") || message.contains("recovered") {
        Some(false)
    } else {
        None
    }
}

struct Message(String);

impl tracing::field::Visit for Message {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

impl tracing::Subscriber for LogWatch {
    fn enabled(&self, meta: &tracing::Metadata<'_>) -> bool {
        let level = *meta.level();
        meta.is_event()
            && meta.target().starts_with("iroh")
            && (level <= tracing::Level::WARN || (level <= tracing::Level::INFO && relay_line(meta.target())))
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::INFO)
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(self.spans.fetch_add(1, Ordering::Relaxed).max(1))
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut m = Message(String::new());
        event.record(&mut m);
        let meta = event.metadata();
        if let Some(on) = same_id_from(meta.target(), &m.0) {
            SAME_ID.store(on, Ordering::Relaxed);
        }
        if self.loud && *meta.level() <= tracing::Level::WARN {
            eprintln!("switchboard-node: [{}] {}", meta.target(), m.0);
        }
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

/// Interface names that lead only to containers and virtual machines on this computer, never to another computer.
/// A VPN's interface (Tailscale, WireGuard) is not one of them: another computer does reach this one there. Nor is
/// a plain `br0`, which is often the bridge a Linux computer's own network address sits on.
const INSIDE_ONLY: [&str; 16] = [
    "docker",
    "br-",
    "veth",
    "virbr",
    "vnet",
    "vmnet",
    "vboxnet",
    "cni",
    "flannel",
    "cali",
    "weave",
    "kube",
    "podman",
    "lxc",
    "lxd",
    "vethernet",
];

/// Is this the name of an interface no other computer can come in by.
pub(crate) fn inside_only(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    // A Mac's `bridge100` and up are the networks it makes for virtual machines and Internet Sharing; `bridge0` is
    // its Thunderbolt bridge, which another Mac does plug into.
    let mac_vm = n.strip_prefix("bridge").and_then(|d| d.parse::<u32>().ok()).is_some_and(|d| d >= 100);
    mac_vm || INSIDE_ONLY.iter().any(|p| n.starts_with(p))
}

/// `addrs` without the ones on `inside`. When that would leave nothing, they are all kept: an address that may
/// not work is more use than none.
pub(crate) fn reachable(addrs: Vec<SocketAddr>, inside: &[IpAddr]) -> Vec<SocketAddr> {
    let kept: Vec<SocketAddr> = addrs.iter().filter(|a| !inside.contains(&a.ip().to_canonical())).copied().collect();
    if kept.is_empty() { addrs } else { kept }
}

#[cfg(test)]
mod tests;
