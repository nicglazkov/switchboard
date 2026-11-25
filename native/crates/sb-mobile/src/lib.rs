//! Switchboard keys for the phone apps.
//!
//! A phone reaches the home computer the way another computer does: it has a key of its own, pairs once with a
//! code or a link, and from then on dials home by its id, directly when it can and through the relay when it
//! cannot. On a computer that is the node's job. A phone has no node, so the same pieces of `sb-link` run inside
//! the app, behind the few calls below, which Swift and Kotlin reach through generated code (UniFFI).
//!
//! What the app gets is two loopback ports, one for the server's API and one for the hub. Its existing HTTP and
//! WebSocket code keeps working unchanged by pointing at `http://127.0.0.1:<port>`; each connection it makes
//! there becomes one stream to home. Other apps on the phone can connect to 127.0.0.1 too, so the ports ask for
//! a secret first (`gate`).
//!
//! When the home computer cannot be reached for a few seconds, the same two ports answer from what the other
//! computers say about their own chats (`away`): the board shows them, a terminal on one of them opens, and
//! everything that needs the home computer says so. The app's address does not change, and the moment home
//! answers again the ports lead to it as before. `LinkStatus::away` and its line say which of the two it is.
//!
//! What this crate does not do is keep secrets. The phone's key is the app's to store (the iOS Keychain, an
//! Android Keystore-wrapped file) and comes in as 32 bytes; which home the phone is paired with is the app's
//! to remember as well (`HomeInfo`). The data folder holds two small files, neither secret: where home was last
//! reached, so the next start can try that address even with the relay away, and the list of devices the home
//! computer signed (`roster.json`), which is how the phone finds the other computers while home is away.
//!
//! Apart from the link there is one more thing both apps need done the same way: opening a sealed notification
//! (`push`). It uses no network and no runtime, so the iPhone app's notification extension calls it too.
//!
//! Every call blocks until it is done and may be made from any thread but the UI's. `pair` takes as long as the
//! network does (a short code also costs a fraction of a second of deliberate hashing); `start`, `stop`,
//! `status` and `network_changed` return at once or nearly.

#[cfg(target_os = "android")]
mod android;
mod away;
mod gate;
pub mod push;
mod store;
mod words;

use gate::{Carrier, Port};
use sb_link::code::{PairInput, valid_relay};
use sb_link::control::resolve_relay;
use sb_link::endpoint::{Clocks, LinkEndpoint, Nobody, Options, PathKind, Peer, Tunnel};
use sb_link::identity::{DeviceId, DeviceKey, KEY_LEN};
use sb_link::paired::Kind;
use sb_link::pairing::{self, About, Binding, Secret};
use sb_link::reach::Others;
use sb_link::roster::RosterCache;
use sb_link::standin::{self, StandIn};
use sb_link::tunnel::Target;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::runtime::Runtime;
use tokio::task::JoinHandle;

uniffi::setup_scaffolding!();

/// The relay this build uses when nothing else names one. Empty: none. The same build setting as the node's.
pub const BUILT_IN_RELAY: &str = match option_env!("SB_RELAY_DEFAULT") {
    Some(r) => r,
    None => "",
};

/// The home computer this phone is paired with. The app keeps it (it is not secret) and hands it to `start`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HomeInfo {
    /// The home computer's id: 64 hex digits. What the app keys its saved board and waiting replies on.
    pub id: String,
    /// What the home computer calls itself, for showing.
    pub name: String,
    /// The relay home is reached through, when it has one.
    pub relay: Option<String>,
}

/// The two loopback ports, and the secret a connection to them must show.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LinkPorts {
    /// `http://127.0.0.1:<api>` is the server.
    pub api: u16,
    /// `ws://127.0.0.1:<hub>` is the hub (terminals).
    pub hub: u16,
    /// Send as the header named by `gate_header()` on every request and WebSocket handshake to those ports.
    pub gate: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LinkState {
    /// `start` has not been called, or `stop` has.
    Stopped,
    /// Dialling home, or dialling again after the connection ended. With `LinkStatus::away` set, the ports
    /// answer from the other computers meanwhile.
    Connecting,
    Connected,
    /// Home answered that this phone is not on its list (any more). Pairing again is the only way back.
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LinkPath {
    /// Any path that is not the relay: the same network, a tailnet, or through the routers.
    Direct,
    Relay,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LinkStatus {
    pub state: LinkState,
    /// How the connection reaches home, while there is one.
    pub path: Option<LinkPath>,
    /// The round trip to home in milliseconds, while there is a connection.
    pub rtt_ms: Option<u32>,
    /// One line for the app to show as it is: "Direct", "Through the relay", "Connecting to home", "Can't reach
    /// home: ...", or while away what the board is showing in its place ("Home is away. Showing chats from Laptop
    /// and Tower."). Empty when stopped.
    pub line: String,
    /// What the person can do about it, when there is something to do.
    pub fix: Option<String>,
    /// Why there is no connection, in the endpoint's own words, for a details row. Not for the main line.
    pub detail: Option<String>,
    /// The home computer has not been reached for a few seconds, and the two ports answer without it: the
    /// board is what the other computers say about their own chats, and what needs home is refused with a
    /// sentence. Ends by itself the moment home answers. The ports and the secret do not change either way.
    pub away: bool,
}

impl LinkStatus {
    fn stopped() -> LinkStatus {
        LinkStatus {
            state: LinkState::Stopped,
            path: None,
            rtt_ms: None,
            line: String::new(),
            fix: None,
            detail: None,
            away: false,
        }
    }

    /// Whether the app should be told: the round trip changing by itself is not news.
    fn differs(&self, other: &LinkStatus) -> bool {
        (self.state, self.path, &self.line, self.away) != (other.state, other.path, &other.line, other.away)
    }
}

/// Why a call did not work. `text` is a sentence to show as it is. (Not named `message`: a Kotlin exception has a
/// `message` of its own, and the generated class would not compile with a second one.)
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Error)]
pub enum LinkError {
    /// What was typed or scanned cannot be a code or a link. Nothing was sent.
    Input {
        text: String,
    },
    /// The code or link is wrong, used or expired: the field is the place to look.
    Code {
        text: String,
    },
    /// The home computer could not be reached.
    Unreachable {
        text: String,
    },
    Other {
        text: String,
    },
}

impl LinkError {
    pub fn message(&self) -> &str {
        match self {
            LinkError::Input { text: message }
            | LinkError::Code { text: message }
            | LinkError::Unreachable { text: message }
            | LinkError::Other { text: message } => message,
        }
    }

    fn other(message: impl Into<String>) -> LinkError {
        LinkError::Other { text: message.into() }
    }
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for LinkError {}

/// Told when the connection's state, path or line changes. Called on one of this crate's own threads.
#[uniffi::export(callback_interface)]
pub trait LinkListener: Send + Sync {
    fn changed(&self, status: LinkStatus);
}

/// A new key for this phone: 32 bytes from the system's random source. The app stores it and passes it to
/// `HomeLink::new` on every launch.
#[uniffi::export]
pub fn new_device_key() -> Vec<u8> {
    DeviceKey::generate().secret_bytes().to_vec()
}

/// The id that goes with a key (64 hex digits), or None when the bytes are not a key.
#[uniffi::export]
pub fn device_id_of(key: Vec<u8>) -> Option<String> {
    let bytes: [u8; KEY_LEN] = key.try_into().ok()?;
    Some(DeviceKey::from_bytes(bytes).id().to_string())
}

/// What was typed or scanned on the join screen, tidied: a code as `ABCD-EFGH`, or the link. An error says what
/// is wrong with it in a sentence, before anything is sent.
#[uniffi::export]
pub fn pairing_input(text: String) -> Result<String, LinkError> {
    words::input(&text)
}

/// The name of the header every request to the loopback ports must carry (`LinkPorts::gate` is its value).
#[uniffi::export]
pub fn gate_header() -> String {
    gate::GATE_HEADER.to_string()
}

/// What is running between `start` and `stop`.
struct Running {
    home: HomeInfo,
    ports: LinkPorts,
    carrier: Arc<Carrier>,
    /// The list of devices the home computer signed, as this phone holds it (roster.json in the data folder).
    cache: Arc<RosterCache>,
    /// What the two ports answer with: the home computer, or the other computers while it is away.
    standin: Arc<StandIn>,
    /// The endpoint and its tunnel, made again when the network changes.
    link: Option<(Arc<LinkEndpoint>, Arc<Tunnel>, JoinHandle<()>)>,
    /// The two ports, the stand-in's watch and the watcher.
    tasks: Vec<JoinHandle<()>>,
}

struct Inner {
    running: Option<Running>,
    /// The ports this launch used last, asked for again so the app's address stays the same.
    last_ports: Option<(u16, u16)>,
    last_told: Option<LinkStatus>,
}

/// This phone's link to its home computer. One per app, made at launch.
#[derive(uniffi::Object)]
pub struct HomeLink {
    rt: Option<Runtime>,
    key: DeviceKey,
    dir: PathBuf,
    /// The relay from the app's own settings, which wins over the one home named and the one built in.
    relay: Option<String>,
    clocks: Clocks,
    /// How long home may be out of reach before the ports answer without it.
    away_after: Duration,
    gate: Arc<str>,
    inner: Arc<Mutex<Inner>>,
    listener: Arc<Mutex<Option<Arc<dyn LinkListener>>>>,
    /// One endpoint with this key at a time: pairing and starting take turns.
    turn: tokio::sync::Mutex<()>,
}

/// The phone's clocks: the app is in front of someone while this runs, so it dials again sooner than a
/// computer's node does.
fn phone_clocks() -> Clocks {
    Clocks { backoff_max: Duration::from_secs(8), dial: Duration::from_secs(10), ..Clocks::default() }
}

fn os_name() -> &'static str {
    if cfg!(target_os = "ios") {
        "ios"
    } else if cfg!(target_os = "android") {
        "android"
    } else {
        std::env::consts::OS
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[uniffi::export]
impl HomeLink {
    /// `key`: the phone's 32 key bytes, from the app's secure storage. `data_dir`: a folder of the app's own, for
    /// the one small file this crate keeps. `relay`: a relay address from the app's settings, when the person
    /// set one; otherwise the one the home computer names is used, then the one built in.
    #[uniffi::constructor]
    pub fn new(key: Vec<u8>, data_dir: String, relay: Option<String>) -> Result<Arc<HomeLink>, LinkError> {
        HomeLink::with_timing(key, data_dir, relay, phone_clocks(), standin::AWAY_AFTER_PHONE)
    }

    /// This phone's id: 64 hex digits. What the home computer lists it under.
    pub fn device_id(&self) -> String {
        self.key.id().to_string()
    }

    /// Pairs this phone with a home computer. `input` is the short code or the link (from the QR code, or
    /// pasted); `name` is what the home computer will list this phone as. Blocks until the exchange is over.
    /// The link is stopped while this runs and stays stopped: call `start` with what comes back.
    pub fn pair(&self, input: String, name: String) -> Result<HomeInfo, LinkError> {
        self.pair_at(&input, &name, vec![])
    }

    /// Dials `home` and opens the two loopback ports. Returns as soon as the ports are open: the connection is
    /// made behind them, and a request that arrives first waits a few seconds for it. Calling it again while
    /// running with the same home changes nothing and returns the same ports.
    pub fn start(&self, home: HomeInfo) -> Result<LinkPorts, LinkError> {
        let id: DeviceId = home.id.parse().map_err(|_| LinkError::other("That is not a home computer's id."))?;
        if home.relay.as_deref().is_some_and(|r| !valid_relay(r)) {
            return Err(LinkError::other("The relay address saved for the home computer is not one."));
        }
        self.rt().block_on(async {
            let _turn = self.turn.lock().await;
            if let Some(r) = &lock(&self.inner).running
                && r.home == home
            {
                return Ok(r.ports.clone());
            }
            self.halt().await;
            let wanted = lock(&self.inner).last_ports;
            let bound = |e: std::io::Error| LinkError::other(format!("This phone could not open a local port ({e})."));
            let api = Port::bind(wanted.map(|w| w.0), Target::Api).await.map_err(bound)?;
            let hub = Port::bind(wanted.map(|w| w.1), Target::Hub).await.map_err(bound)?;
            let ports = LinkPorts {
                api: api.port().map_err(bound)?,
                hub: hub.port().map_err(bound)?,
                gate: self.gate.to_string(),
            };
            let carrier = Arc::new(Carrier::default());
            // (reading a file: off the threads that carry everything else)
            let dir = self.dir.clone();
            let cache = tokio::task::spawn_blocking(move || away::list_in(&dir, id))
                .await
                .map_err(|e| LinkError::other(e.to_string()))?;
            let link = self.dial(id, &home, &carrier, &cache).await?;
            let told = {
                let (inner, listener) = (self.inner.clone(), self.listener.clone());
                Box::new(move || tell(&inner, &listener))
            };
            let reach = Arc::new(away::Reach { carrier: carrier.clone(), cache: cache.clone(), changed: told });
            let opts = standin::Options { away_after: self.away_after, ..standin::Options::default() };
            let standin = StandIn::new(reach, opts);
            standin.set_hub_port(ports.hub);
            let tasks = vec![
                tokio::spawn(api.run(self.gate.clone(), standin.clone())),
                tokio::spawn(hub.run(self.gate.clone(), standin.clone())),
                tokio::spawn(standin.clone().run()),
                tokio::spawn(watch(self.inner.clone(), self.listener.clone(), self.dir.clone(), id)),
            ];
            let mut inner = lock(&self.inner);
            inner.last_ports = Some((ports.api, ports.hub));
            let link = Some(link);
            inner.running = Some(Running { home, ports: ports.clone(), carrier, cache, standin, link, tasks });
            Ok(ports)
        })
    }

    /// The connection as it is now.
    pub fn status(&self) -> LinkStatus {
        status_of(&lock(&self.inner))
    }

    /// The phone's network changed (Wi-Fi to mobile data, a VPN came up), or the app came back to the front
    /// after a long time away: the connection is made again now instead of waiting to be found dead. The ports
    /// stay as they are. Nothing happens when stopped.
    pub fn network_changed(&self) {
        self.rt().block_on(async {
            let _turn = self.turn.lock().await;
            let Some((home, carrier, cache, old)) = ({
                let mut inner = lock(&self.inner);
                inner.running.as_mut().map(|r| (r.home.clone(), r.carrier.clone(), r.cache.clone(), r.link.take()))
            }) else {
                return;
            };
            carrier.set(None);
            close(old).await;
            let Ok(id) = home.id.parse::<DeviceId>() else { return };
            // (a failure here leaves the ports open with nothing behind them, which reads as "Connecting")
            let link = self.dial(id, &home, &carrier, &cache).await.ok();
            if let Some(r) = lock(&self.inner).running.as_mut() {
                r.link = link;
            }
        })
    }

    /// Closes the connection and the two ports. For when the app goes to the background, and before pairing
    /// again. `start` brings everything back, on the same ports when they are still free.
    pub fn stop(&self) {
        self.rt().block_on(async {
            let _turn = self.turn.lock().await;
            self.halt().await;
        });
        self.tell();
    }

    /// Who to tell when the connection changes, in place of whoever was told before. `status` can be asked at
    /// any time either way.
    pub fn set_listener(&self, listener: Box<dyn LinkListener>) {
        *lock(&self.listener) = Some(Arc::from(listener));
    }

    /// Nobody is told from now on.
    pub fn clear_listener(&self) {
        *lock(&self.listener) = None;
    }
}

impl HomeLink {
    /// `new` with other clocks (tests shorten them).
    pub fn with_clocks(
        key: Vec<u8>,
        data_dir: String,
        relay: Option<String>,
        clocks: Clocks,
    ) -> Result<Arc<HomeLink>, LinkError> {
        HomeLink::with_timing(key, data_dir, relay, clocks, standin::AWAY_AFTER_PHONE)
    }

    /// `with_clocks`, and how long home may be out of reach before the ports answer without it.
    pub fn with_timing(
        key: Vec<u8>,
        data_dir: String,
        relay: Option<String>,
        clocks: Clocks,
        away_after: Duration,
    ) -> Result<Arc<HomeLink>, LinkError> {
        let bytes: [u8; KEY_LEN] =
            key.try_into().map_err(|_| LinkError::other("This phone's key could not be read (it is not 32 bytes)."))?;
        let relay = relay.map(|r| r.trim().to_string()).filter(|r| !r.is_empty());
        if let Some(r) = &relay
            && !valid_relay(r)
        {
            return Err(LinkError::other(format!(
                "{r} is not a relay address. It should look like https://relay.example.com"
            )));
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("switchboard-link")
            .enable_all()
            .build()
            .map_err(|e| LinkError::other(format!("The connection could not start ({e}).")))?;
        Ok(Arc::new(HomeLink {
            rt: Some(rt),
            key: DeviceKey::from_bytes(bytes),
            dir: PathBuf::from(data_dir),
            relay,
            clocks,
            away_after,
            gate: gate::new_secret().into(),
            inner: Arc::new(Mutex::new(Inner { running: None, last_ports: None, last_told: None })),
            listener: Arc::default(),
            turn: tokio::sync::Mutex::new(()),
        }))
    }

    fn rt(&self) -> &Runtime {
        self.rt.as_ref().expect("the runtime lives as long as the link")
    }

    /// `pair`, with addresses to try for home when neither the link nor this phone names a relay (the same
    /// network, in tests).
    pub fn pair_at(&self, input: &str, name: &str, addrs: Vec<SocketAddr>) -> Result<HomeInfo, LinkError> {
        let tidy = words::input(input)?;
        let input = PairInput::parse(&tidy).map_err(|e| LinkError::Input { text: e.to_string() })?;
        let home = self.rt().block_on(async {
            let _turn = self.turn.lock().await;
            self.halt().await; // one endpoint with this key at a time
            self.join(input, name, addrs.clone()).await
        })?;
        self.tell();
        if !addrs.is_empty() {
            store::save(&self.dir, &home.id, &addrs);
        }
        Ok(home)
    }

    async fn join(&self, input: PairInput, name: &str, addrs: Vec<SocketAddr>) -> Result<HomeInfo, LinkError> {
        let own = resolve_relay(None, self.relay.as_deref(), None, BUILT_IN_RELAY).0;
        let (peer, secret) = match input {
            PairInput::Link(link) => {
                let relay = link.relay.clone().or(own);
                (Peer { id: link.home, relay, addrs }, Secret::from(&link))
            }
            PairInput::Code(code) => {
                if own.is_none() && addrs.is_empty() {
                    return Err(LinkError::other(words::NEEDS_LINK));
                }
                // (slow on purpose, so off the threads that carry everything else)
                let c = code.clone();
                let id = tokio::task::spawn_blocking(move || c.rendezvous_id())
                    .await
                    .map_err(|e| LinkError::other(e.to_string()))?;
                (Peer { id, relay: own, addrs }, Secret::from(code))
            }
        };
        if peer.relay.is_none() && peer.addrs.is_empty() {
            return Err(LinkError::other(words::NEEDS_ADDRESS));
        }
        let ep = LinkEndpoint::bind(&self.key, self.options(&peer.relay), Arc::new(Nobody))
            .await
            .map_err(|e| LinkError::other(format!("The connection could not start ({e}).")))?;
        let about = About {
            name: if name.trim().is_empty() { "Phone".into() } else { name.trim().to_string() },
            kind: Kind::Phone,
            os: os_name().into(),
            // (the app holds which home the phone has, and asks before it replaces one: nothing to say here)
            stays_with: None,
        };
        let result = async {
            let mut out = ep.pair_dial(&peer).await.map_err(|_| words::unreachable())?;
            let channel = out.channel;
            let binding = Binding { host: out.host.0, joiner: out.joiner.0, channel: &channel };
            let r = pairing::join(&mut out.stream, &secret, &binding, &about).await.map_err(|e| words::pair_error(&e));
            out.finish();
            r
        }
        .await;
        ep.close().await;
        drop(ep);
        let welcome = result?;
        Ok(HomeInfo {
            id: welcome.home_id.to_string(),
            name: welcome.home_name.clone(),
            relay: welcome.relay.clone().or(peer.relay),
        })
    }

    fn options(&self, relay: &Option<String>) -> Options {
        Options { relay: relay.clone(), clocks: self.clocks.clone(), ..Options::default() }
    }

    /// A new endpoint and a tunnel to home on it, set as what the ports carry into, with the way to the other
    /// computers that goes with that endpoint.
    async fn dial(
        &self,
        id: DeviceId,
        home: &HomeInfo,
        carrier: &Arc<Carrier>,
        cache: &Arc<RosterCache>,
    ) -> Result<(Arc<LinkEndpoint>, Arc<Tunnel>, JoinHandle<()>), LinkError> {
        let relay = resolve_relay(None, self.relay.as_deref(), home.relay.as_deref(), BUILT_IN_RELAY).0;
        let ep = LinkEndpoint::bind(&self.key, self.options(&relay), Arc::new(Nobody))
            .await
            .map_err(|e| LinkError::other(format!("The connection could not start ({e}).")))?;
        let peer = Peer { id, relay, addrs: store::load(&self.dir, &home.id) };
        let tunnel = Tunnel::new(ep.clone(), peer);
        let task = tokio::spawn(tunnel.clone().run());
        let others = Others::new(ep.clone(), cache.clone(), away::take_into(cache), Arc::new(|_| {}));
        carrier.set(Some((tunnel.clone(), Arc::new(others))));
        Ok((ep, tunnel, task))
    }

    /// Stops everything that is running. The caller holds `turn`.
    async fn halt(&self) {
        let Some(mut r) = lock(&self.inner).running.take() else { return };
        for t in r.tasks.drain(..) {
            t.abort();
        }
        r.carrier.set(None);
        close(r.link.take()).await;
    }

    /// Tells the listener where things stand, when that is news.
    fn tell(&self) {
        tell(&self.inner, &self.listener);
    }
}

impl Drop for HomeLink {
    fn drop(&mut self) {
        if let Some(r) = lock(&self.inner).running.take() {
            for t in &r.tasks {
                t.abort();
            }
            if let Some((_, tunnel, task)) = &r.link {
                tunnel.close();
                task.abort();
            }
        }
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
        }
    }
}

async fn close(link: Option<(Arc<LinkEndpoint>, Arc<Tunnel>, JoinHandle<()>)>) {
    let Some((ep, tunnel, task)) = link else { return };
    task.abort();
    tunnel.close();
    // (a moment for the goodbye to leave; the phone may have no network at all, and must not wait on it)
    let _ = tokio::time::timeout(Duration::from_millis(500), ep.close()).await;
}

fn status_of(inner: &Inner) -> LinkStatus {
    let Some(r) = &inner.running else { return LinkStatus::stopped() };
    let Some((ep, tunnel, _)) = &r.link else {
        let (mut line, fix) = words::line(LinkState::Connecting, None, None, false);
        let away = words::away_line(r.standin.away().as_ref());
        if let Some(a) = &away {
            line = a.clone();
        }
        let (state, away) = (LinkState::Connecting, away.is_some());
        return LinkStatus { state, path: None, rtt_ms: None, line, fix, detail: None, away };
    };
    let s = tunnel.status();
    let state = if s.connected {
        LinkState::Connected
    } else if s.refused {
        LinkState::Removed
    } else {
        LinkState::Connecting
    };
    let path = s.path.map(|p| match p {
        PathKind::Direct => LinkPath::Direct,
        PathKind::Relay => LinkPath::Relay,
    });
    let relay = ep.relay();
    let relay_down = relay.url.is_some() && !relay.connected && relay.error.is_some();
    let (mut line, fix) = words::line(state, path, s.error.as_deref(), relay_down);
    let detail = if s.connected { None } else { s.error.clone().or(relay.error) };
    // Away is what the ports are answering with, so it is said for exactly as long as they do: for a moment
    // after home is reached again it is still true, and a request sent then would still be answered without
    // home. A phone that home removed is never away (see `away::Reach::home_up`).
    let away = words::away_line(r.standin.away().as_ref());
    if let Some(a) = &away {
        line = a.clone();
    }
    let rtt_ms = s.rtt_ms.map(|r| r.min(u64::from(u32::MAX)) as u32);
    LinkStatus { state, path, rtt_ms, line, fix, detail, away: away.is_some() }
}

fn tell(inner: &Arc<Mutex<Inner>>, listener: &Arc<Mutex<Option<Arc<dyn LinkListener>>>>) {
    let now = {
        let mut inner = lock(inner);
        let now = status_of(&inner);
        if inner.last_told.as_ref().is_some_and(|last| !last.differs(&now)) {
            return;
        }
        inner.last_told = Some(now.clone());
        now
    };
    // (called with no lock held: the app may ask for the status from inside its answer)
    let to = lock(listener).clone();
    if let Some(l) = to {
        l.changed(now);
    }
}

/// How often the home computer is asked for its list of devices while it is reached.
const LIST_EVERY: Duration = Duration::from_secs(60);

/// While running: tells the listener when the connection changes, remembers where home was reached, and keeps
/// the list of devices fresh.
async fn watch(
    inner: Arc<Mutex<Inner>>,
    listener: Arc<Mutex<Option<Arc<dyn LinkListener>>>>,
    dir: PathBuf,
    home: DeviceId,
) {
    let mut kept: Option<SocketAddr> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    // The list of devices is asked for each time the home computer is reached, and once a minute after that.
    let (mut was_up, mut asked) = (false, None::<std::time::Instant>);
    let mut shown = None::<std::time::Instant>;
    loop {
        tick.tick().await;
        tell(&inner, &listener);
        let (remote, link) = {
            let inner = lock(&inner);
            let r = inner.running.as_ref();
            let remote = r.and_then(|r| r.link.as_ref()).and_then(|(_, t, _)| t.status().remote);
            (remote, r.and_then(|r| r.carrier.get()))
        };
        let up = link.as_ref().is_some_and(|(home, _)| home.status().connected);
        // A home computer that refuses this phone is shown the list this phone holds: it may be one brought
        // back from a backup made before this phone was paired (sb-link reach.rs `show`).
        if let Some((home, others)) = &link
            && home.status().refused
            && shown.is_none_or(|t| t.elapsed() >= LIST_EVERY)
        {
            shown = Some(std::time::Instant::now());
            let (home, others) = (home.clone(), others.clone());
            tokio::spawn(async move {
                let _ = others.show(&home.peer()).await;
            });
        }
        if let (true, Some((home, others))) = (up, link)
            && (!was_up || asked.is_none_or(|t| t.elapsed() >= LIST_EVERY))
        {
            asked = Some(std::time::Instant::now());
            // (on its own task: a home computer that is slow to answer must not hold up what the app is told)
            tokio::spawn(async move {
                let _ = others.pull(&home).await;
            });
        }
        was_up = up;
        if let Some(addr) = remote
            && kept != Some(addr)
        {
            kept = Some(addr);
            let (dir, id) = (dir.clone(), home.to_string());
            let _ = tokio::task::spawn_blocking(move || store::save(&dir, &id, &[addr])).await;
        }
    }
}

#[cfg(test)]
mod tests;
