//! Keys in the helper: this computer's endpoint, and what it carries.
//!
//! Nothing here runs unless it is turned on. What turns it on, read again every few seconds:
//!
//! - **home**: the server's settings on this computer (`server.json`) say `"keys": "both"` or `"only"`. The
//!   helper then takes tunnels from paired devices and passes them to the server and the hub on this computer,
//!   pairs new devices, and asks the computers the hub already lists to switch to keys.
//! - **member**: `node.json` names a home computer (`home`), which pairing or home's request wrote. The helper
//!   then keeps a tunnel to home, opens two ports on this computer that lead to home's server and hub, and
//!   carries its own hub link through the tunnel.
//! - neither: off. No key is made, no port is opened, nothing is dialled. Home wins when both are true.
//!
//! One process at a time may use this computer's key: it holds `keys.lock` (next to node.json) while it does.

use crate::link::log;
use crate::own::{self, Surface};
use crate::{HomeConfig, NodeConfig};
use sb_core::durable::{Bak, Store};
use sb_link::code::{PairInput, PairLink, ShortCode, valid_relay};
use sb_link::control::{
    DeviceRow, ForwardRow, HomeRow, PairNew, RelayRow, RosterRow, Status, UpdateRow, resolve_relay,
};
use sb_link::endpoint::{Clocks, LinkEndpoint, Nobody, Options, OwnIncoming, PairIncoming, Peer, Tunnel};
use sb_link::identity::{DeviceId, DeviceKey, Found};
use sb_link::paired::{Device, Kind, Paired, Via};
use sb_link::pairing::{self, About, Binding, Grant, Offers, PairError, Request, Secret, Select, Welcome};
use sb_link::reach::Others;
use sb_link::roster::{self, Blocks, Body, Envelope, Roster, RosterCache, RosterDevice, RosterError, RosterGate};
use sb_link::standin::{self, StandIn};
use sb_link::tunnel::{Target, Targets};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// The relay this build uses when nothing else names one. Empty: none.
pub const BUILT_IN_RELAY: &str = match option_env!("SB_RELAY_DEFAULT") {
    Some(r) => r,
    None => "",
};

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The files keys use.
#[derive(Debug, Clone)]
pub struct Places {
    /// node.json.
    pub config: PathBuf,
    /// This computer's key: device.key next to node.json.
    pub key: PathBuf,
    /// Held by the one process that is using the key: keys.lock next to node.json.
    pub lock: PathBuf,
    /// Home's list of paired devices.
    pub paired: PathBuf,
    /// The server's settings, read for "keys" and never written.
    pub server_json: PathBuf,
    /// The home computer's signed list of devices, as this computer last took it (home: as it last signed it):
    /// roster.json next to node.json.
    pub roster: PathBuf,
    /// The devices this computer refuses although the list names them: blocked.json next to node.json.
    pub blocked: PathBuf,
    /// Home: the two numbers the list is signed with, roster-state.json next to the paired list.
    pub roster_state: PathBuf,
}

impl Places {
    pub fn new(config: &Path, home: &Path, env: &dyn Fn(&str) -> Option<String>) -> Places {
        // The paired list: a variable for one file, else the folder the server was given, else the user's
        // folder. The list's two numbers live beside it; the signed list and the blocks live beside node.json,
        // so every one of them follows the same folder rules.
        let paired = env("SWITCHBOARD_PAIRED")
            .or_else(|| env("SWITCHBOARD_STATE_DIR").map(|d| format!("{d}/paired.json")))
            .map(PathBuf::from)
            .unwrap_or_else(|| sb_link::paired::paired_path(home));
        Places {
            config: config.to_path_buf(),
            key: config.with_file_name("device.key"),
            lock: config.with_file_name("keys.lock"),
            roster: config.with_file_name("roster.json"),
            blocked: config.with_file_name("blocked.json"),
            roster_state: paired.with_file_name("roster-state.json"),
            paired,
            server_json: env("SWITCHBOARD_SERVER_JSON")
                .map(PathBuf::from)
                .unwrap_or_else(|| sb_hub::config::config_dir(home, env).join("server.json")),
        }
    }
}

/// The line home's log gets when a device is removed.
pub(crate) fn removed_line(device: Option<&sb_link::paired::Device>, id: &DeviceId) -> String {
    let name = device.map(|d| d.name.trim()).filter(|n| !n.is_empty()).unwrap_or("a device");
    let machine = device.and_then(|d| d.machine.as_deref()).map(|m| format!(", the computer {m}")).unwrap_or_default();
    format!("keys: removed {name} ({}){machine}: it is refused from now on", id.short())
}

/// The addresses keys use, all on this computer.
#[derive(Debug, Clone)]
pub struct Ports {
    /// The control API.
    pub control: SocketAddr,
    /// Home: where a tunnel's "api" and "hub" streams go.
    pub api_target: SocketAddr,
    pub hub_target: SocketAddr,
    /// Member: the ports that lead to home's server and hub. None: not opened.
    pub api_forward: Option<SocketAddr>,
    pub hub_forward: Option<SocketAddr>,
    /// Home: the hub's own address, for the routes it answers this computer only.
    pub hub: SocketAddr,
    /// One UDP address for the endpoint instead of every interface.
    pub bind: Option<SocketAddr>,
    /// Member: how long the home computer may be out of reach before this computer's two ports answer
    /// without it.
    pub away_after: Duration,
}

fn local(port: u16) -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], port))
}

/// An address on this computer, from "127.0.0.1:8137" or "http://127.0.0.1:8137".
fn loopback_addr(name: &str, v: &str) -> Result<SocketAddr, String> {
    let t = v.trim().trim_start_matches("http://").trim_end_matches('/');
    let a: SocketAddr = t.parse().map_err(|_| format!("{name}: {v} is not an address like 127.0.0.1:8137"))?;
    if !a.ip().to_canonical().is_loopback() {
        return Err(format!("{name}: {v} is not on this computer (use 127.0.0.1 and a port)"));
    }
    Ok(a)
}

impl Ports {
    pub fn new(env: &dyn Fn(&str) -> Option<String>) -> Result<Ports, String> {
        let at = |name: &str, default: u16| match env(name) {
            Some(v) => loopback_addr(name, &v),
            None => Ok(local(default)),
        };
        let forward = |name: &str, default: u16| match env(name) {
            Some(v) if v.trim() == "off" => Ok(None),
            Some(v) => loopback_addr(name, &v).map(Some),
            None => Ok(Some(local(default))),
        };
        let ports = Ports {
            control: at("SWITCHBOARD_NODE_CONTROL", sb_link::control::PORT)?,
            api_target: at("SWITCHBOARD_NODE_API_TARGET", 8130)?,
            hub_target: at("SWITCHBOARD_NODE_HUB_TARGET", sb_hub::config::TUNNEL_PORT)?,
            api_forward: forward("SWITCHBOARD_NODE_API_FORWARD", 8130)?,
            hub_forward: forward("SWITCHBOARD_NODE_HUB_FORWARD", 8132)?,
            hub: at("SWITCHBOARD_NODE_HUB", sb_hub::config::PORT)?,
            bind: match env("SWITCHBOARD_NODE_BIND") {
                Some(v) => Some(v.trim().parse().map_err(|_| format!("SWITCHBOARD_NODE_BIND: {v} is not an address"))?),
                None => None,
            },
            away_after: match env("SWITCHBOARD_NODE_AWAY_AFTER") {
                Some(v) => Duration::from_secs(
                    v.trim()
                        .parse::<u64>()
                        .ok()
                        .filter(|s| (1..=3600).contains(s))
                        .ok_or_else(|| format!("SWITCHBOARD_NODE_AWAY_AFTER: {v} is not a number of seconds"))?,
                ),
                None => standin::AWAY_AFTER,
            },
        };
        // What comes out of a tunnel goes to the hub's tunnel listener, which has none of the routes the hub
        // keeps for this computer. Never to the hub's main port.
        if ports.hub_target == ports.hub || ports.hub_target.port() == sb_hub::config::PORT {
            return Err(format!(
                "SWITCHBOARD_NODE_HUB_TARGET: {} is the hub's own port. Tunnels go to its tunnel listener ({}).",
                ports.hub_target,
                sb_hub::config::TUNNEL_PORT
            ));
        }
        Ok(ports)
    }
}

/// "keys" in the server's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Off,
    Both,
    Only,
}

/// "keys" from the server's settings, read exactly as the hub reads it (sb-hub config.rs `read_keys`): no file is
/// `Off`; a file that does not read falls back to its copy; and with no good copy, or a value nobody knows, it
/// is `Only`.
pub fn read_mode(path: &Path) -> Mode {
    match sb_hub::config::read_keys(path).keys {
        sb_hub::config::Keys::Off => Mode::Off,
        sb_hub::config::Keys::Both => Mode::Both,
        sb_hub::config::Keys::Only => Mode::Only,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    Off,
    Home,
    Member(HomeConfig),
}

/// The rule: the server's settings on this computer turn keys on, so this is home; else a home named in
/// node.json makes this a member; else keys are off.
pub fn role_of(mode: Mode, cfg: Option<&NodeConfig>) -> Role {
    if mode != Mode::Off {
        return Role::Home;
    }
    match cfg.and_then(|c| c.home.clone()) {
        Some(h) => Role::Member(h),
        None => Role::Off,
    }
}

/// True when an error is a Mac refusing this program the local network: connections to addresses on the LAN
/// then fail with "No route to host" although the network is fine.
pub fn local_network_off(os: &str, error: &str) -> bool {
    os == "macos" && (error.contains("No route to host") || error.contains("os error 65"))
}

/// Why something was turned down: `kind` is one word for a program to act on, `text` is for the person and may
/// change. It reads as its text wherever a string is wanted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub kind: &'static str,
    pub text: String,
}

impl Refusal {
    pub fn new(kind: &'static str, text: impl Into<String>) -> Refusal {
        Refusal { kind, text: text.into() }
    }
}

impl std::ops::Deref for Refusal {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

const SAME_KEY: &str = "Another computer is using this computer's key (a copied disk or a restored backup). Remove \
                        this computer at the home computer and pair it again.";

/// The line about this computer's key being in use twice, when it is worth saying. The relay's word for it
/// (`same_id`) also turns up while a computer the home computer removed keeps trying to come back, and there the
/// one thing to say is that it was removed: pairing it again is what settles both.
fn same_key_line(same_id: bool, refused_by_home: bool) -> Option<&'static str> {
    (same_id && !refused_by_home).then_some(SAME_KEY)
}

/// Why a code or link from another home computer is turned down: this computer has a home, and nobody said to
/// replace it. One sentence for the command line and for the app, which has a button for it.
fn other_home(old: &HomeConfig) -> Refusal {
    let name = if old.name.trim().is_empty() { old.id.short() } else { old.name.trim().to_string() };
    Refusal::new(
        "other_home",
        format!(
            "This computer is paired with the home computer {name} already, so nothing was changed. To pair it \
             with another home computer instead, run the same command again with --replace at the end, or choose \
             Replace in the app."
        ),
    )
}

const LOCAL_NETWORK: &str = "This Mac is not letting switchboard-node use the local network. Turn it on in System \
                             Settings, Privacy & Security, Local Network.";

/// The two numbers home signs its list of devices with. `seq` rises whenever who is on the list changes;
/// `epoch` only when the home computer moves or is restored.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
struct Counters {
    v: u32,
    epoch: u64,
    seq: u64,
    /// When this home computer was brought back from a backup, and the epoch it has used since. For seven days
    /// it takes what the other devices show it of the list as it was after the backup was made (home.rs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restored: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    restored_epoch: Option<u64>,
    /// The newest list it took that way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    adopted: Option<roster::Stamp>,
}

/// Takes a list of devices the home computer signed, if it is newer than the one held: the blocks it outdates
/// end, and devices it no longer names are cut off at once.
fn take_roster(
    cache: &RosterCache,
    blocks: &Blocks,
    ep: Option<&LinkEndpoint>,
    envelope: &Envelope,
) -> Result<bool, RosterError> {
    let took = cache.offer(envelope, unix_now());
    if let Some(t) = cache.trouble() {
        log(format!("keys: {t}"));
    }
    if took? {
        if let Some(r) = cache.get() {
            blocks.prune(r.stamp());
            log(format!(
                "keys: took the home computer's list of devices (number {}, {} on it)",
                r.body.seq,
                r.body.devices.len()
            ));
        }
        match ep.map(|ep| ep.sweep()).unwrap_or(0) {
            0 => {}
            1 => log("keys: a device that is no longer on the list was cut off"),
            n => log(format!("keys: {n} devices that are no longer on the list were cut off")),
        }
        return Ok(true);
    }
    Ok(false)
}

/// What a computer's answer to `node.roster` says about its helper. Some(true): it knows what a list of devices
/// is (it took this one, or said in its own words why not). Some(false): its helper is too old to: the request
/// went on to its script, which knows no such verb and says so without a `kind`. None: nothing either way.
fn took_of(answer: &Value) -> Option<bool> {
    if answer["ok"] == true && answer["have"].is_object() {
        return Some(true);
    }
    if answer["is_home"] == true {
        return None;
    }
    match (answer["ok"].as_bool(), answer["kind"].as_str()) {
        (Some(false), Some(_)) => Some(true),
        (Some(false), None) => Some(false),
        _ => None,
    }
}

/// The home computer's own surface holds the list it signed, and takes none from anybody. It does look at what
/// it is shown: a list its own key signed with a higher epoch means another computer answers as this home
/// (home.rs `home_sees`).
struct HomeLists(Arc<RosterCache>, std::sync::Weak<Shared>);

impl own::Lists for HomeLists {
    fn held(&self) -> Option<Arc<Roster>> {
        self.0.get()
    }
    fn offer(&self, envelope: &Envelope) -> Result<bool, RosterError> {
        match self.1.upgrade() {
            Some(s) => s.home_sees(envelope),
            None => Ok(false),
        }
    }
}

struct MemberLists {
    cache: Arc<RosterCache>,
    blocks: Arc<Blocks>,
    ep: Arc<LinkEndpoint>,
}

impl own::Lists for MemberLists {
    fn held(&self) -> Option<Arc<Roster>> {
        self.cache.get()
    }
    fn offer(&self, envelope: &Envelope) -> Result<bool, RosterError> {
        take_roster(&self.cache, &self.blocks, Some(&self.ep), envelope)
    }
}

struct HomeSide {
    paired: Arc<Paired>,
    offers: Offers,
    name: String,
    /// Signs the list of devices.
    key: DeviceKey,
    /// The list as it was last signed, on disk as roster.json.
    held: Arc<RosterCache>,
    /// Held while the list is signed, so two changes never sign with one number.
    signing: tokio::sync::Mutex<()>,
    /// The linked computers the list was sent to since this helper started: whether each took it (false: its
    /// helper is too old to know what a list is). The next list says so of each (`RosterDevice::own`), which is
    /// how it outlives this process and reaches the other computers.
    took: Mutex<HashMap<String, bool>>,
    /// The computers the hub lists that were asked to switch to keys: the version that was asked, and what
    /// came of it.
    asked: Mutex<HashMap<String, (String, String)>>,
    /// Machines removed here: never asked again by this process.
    removed: Mutex<HashSet<String>>,
    /// The computers on the list, as this home computer reaches them to ask which list they hold (home.rs).
    witness: Arc<Others>,
}

/// A computer that is not home, as the stand-in sees it (sb-link standin.rs): how the home computer and the
/// other computers are reached from here.
struct Reach {
    id: DeviceId,
    machine: String,
    home: Arc<Tunnel>,
    cache: Arc<RosterCache>,
    surface: Arc<Surface>,
    after: Duration,
    /// A connection to each other computer that was asked for its chats, kept while the home computer is away,
    /// and the comparing of lists that goes with it (sb-link reach.rs, which a phone uses too).
    others: Others,
}

impl standin::Backend for Reach {
    fn home_up(&self) -> bool {
        self.home.status().connected
    }

    fn open_home(&self, target: Target) -> standin::Fut<'_, std::io::Result<standin::Io>> {
        Box::pin(async move { Ok(Box::new(sb_link::reach::open_to(&self.home, target).await?) as standin::Io) })
    }

    fn roster(&self) -> Option<Arc<Roster>> {
        self.cache.get()
    }

    fn open_own(&self, device: &RosterDevice) -> standin::Fut<'_, std::io::Result<standin::Io>> {
        let device = device.clone();
        Box::pin(async move {
            if device.id == self.id {
                // this computer's own chats: asked here, with no network in between
                return Ok(Box::new(self.surface.open_local()) as standin::Io);
            }
            Ok(Box::new(self.others.open(&device).await?) as standin::Io)
        })
    }

    fn event(&self, body: &Value) {
        // a hook's or the status line's word about a chat on this computer
        if body["event"] == "ping" && body["machine"].as_str().is_none_or(|m| m == self.machine) {
            self.surface.ping(body);
        }
    }

    fn away_changed(&self, away: bool) {
        if away {
            log(format!(
                "keys: the home computer has not answered for {} seconds. Until it does, programs here are shown \
                 this computer's own chats and what the other computers say about theirs.",
                self.after.as_secs().max(1)
            ));
        } else {
            log("keys: the home computer answers again");
            self.others.drop_all();
        }
    }
}

struct MemberSide {
    home: HomeConfig,
    tunnel: Arc<Tunnel>,
    forwards: Vec<ForwardRow>,
    reach: Arc<Reach>,
    standin: Arc<StandIn>,
    /// The home computer's list of devices: who may reach this computer's own chats.
    cache: Arc<RosterCache>,
    blocks: Arc<Blocks>,
}

/// Keys, running.
pub struct Runtime {
    id: DeviceId,
    ep: Arc<LinkEndpoint>,
    relay: (Option<String>, &'static str),
    /// What it was started from: when this no longer matches the files, it is stopped and started again.
    print: String,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    home: Option<HomeSide>,
    member: Option<MemberSide>,
}

impl Runtime {
    fn spawn(&self, f: impl Future<Output = ()> + Send + 'static) {
        self.tasks.lock().unwrap_or_else(|e| e.into_inner()).push(tokio::spawn(f));
    }

    async fn stop(&self) {
        for t in self.tasks.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
            t.abort();
        }
        if let Some(m) = &self.member {
            m.reach.others.drop_all();
            m.tunnel.close();
        }
        if let Some(h) = &self.home {
            h.witness.drop_all();
        }
        self.ep.close().await;
    }
}

/// Everything about keys in this process.
pub struct Shared {
    pub places: Places,
    pub ports: Ports,
    env_relay: Option<String>,
    built_in: String,
    clocks: Clocks,
    runtime: RwLock<Option<Arc<Runtime>>>,
    /// Held while keys are started, stopped or paired, so those never overlap.
    busy: tokio::sync::Mutex<()>,
    reload: tokio::sync::Notify,
    lock: Mutex<Option<std::fs::File>>,
    control: Mutex<Option<JoinHandle<()>>>,
    /// The last reason the control port could not be opened, so it is logged once.
    control_said: Mutex<Option<String>>,
    /// Why keys could not start, when they could not.
    trouble: Mutex<Option<String>>,
    /// Moving the home computer, keeping a copy of it, bringing it back (home.rs).
    pub kit: home::Kit,
}

/// How often a computer that is not home asks the home computer for its list of devices.
const PULL_EVERY: Duration = Duration::from_secs(60);

fn print_of(role: &Role, relay: &Option<String>) -> String {
    match role {
        Role::Off => "off".into(),
        Role::Home => format!("home {relay:?}"),
        Role::Member(h) => format!("member {} {relay:?}", h.id),
    }
}

impl Shared {
    /// With nothing to pack the home computer's state with, and no server to stop or start: `with_kit` gives
    /// it those (so a test never runs this computer's real ones).
    pub fn new(places: Places, ports: Ports, env_relay: Option<String>, built_in: &str) -> Arc<Shared> {
        let kit = home::Kit::none(&places);
        Arc::new(Shared {
            kit,
            places,
            ports,
            env_relay,
            built_in: built_in.to_string(),
            clocks: Clocks::default(),
            runtime: RwLock::new(None),
            busy: tokio::sync::Mutex::new(()),
            reload: tokio::sync::Notify::new(),
            lock: Mutex::new(None),
            control: Mutex::new(None),
            control_said: Mutex::new(None),
            trouble: Mutex::new(None),
        })
    }

    /// From this process's environment.
    pub fn here(config: &Path) -> Result<Arc<Shared>, String> {
        let env = |k: &str| crate::env(k);
        let places = Places::new(config, &crate::home(), &env);
        let kit = home::Kit::new(&places, &env);
        Ok(Shared::new(places, Ports::new(&env)?, env("SWITCHBOARD_RELAY"), BUILT_IN_RELAY).with_kit(kit))
    }

    pub fn with_kit(self: Arc<Shared>, kit: home::Kit) -> Arc<Shared> {
        let mut s = Arc::try_unwrap(self).ok().expect("not shared yet");
        s.kit = kit;
        Arc::new(s)
    }

    #[cfg(test)]
    pub fn with_clocks(self: Arc<Shared>, clocks: Clocks) -> Arc<Shared> {
        let mut s = Arc::try_unwrap(self).ok().expect("not shared yet");
        s.clocks = clocks;
        Arc::new(s)
    }

    fn runtime(&self) -> Option<Arc<Runtime>> {
        self.runtime.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn config(&self) -> Option<NodeConfig> {
        crate::load_raw(&self.places.config)
    }

    pub fn role(&self) -> Role {
        role_of(self.mode(), self.config().as_ref())
    }

    /// The tunnel to home, on a computer that is not home and has keys on: the hub link goes through it.
    pub fn tunnel(&self) -> Option<Arc<Tunnel>> {
        self.runtime().and_then(|r| r.member.as_ref().map(|m| m.tunnel.clone()))
    }

    fn relay_for(&self, cfg: Option<&NodeConfig>, home: Option<&HomeConfig>) -> (Option<String>, &'static str) {
        resolve_relay(
            self.env_relay.as_deref(),
            cfg.and_then(|c| c.relay.as_deref()),
            home.and_then(|h| h.relay.as_deref()),
            &self.built_in,
        )
    }

    /// Takes keys.lock. False when another process on this computer has it.
    fn take_lock(&self) -> bool {
        let mut held = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if held.is_some() {
            return true;
        }
        if let Some(dir) = self.places.lock.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false).open(&self.places.lock);
        match file {
            Ok(f) if !matches!(f.try_lock(), Err(std::fs::TryLockError::WouldBlock)) => {
                *held = Some(f);
                true
            }
            _ => false,
        }
    }

    fn drop_lock(&self) {
        *self.lock.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Whether another process on this computer is using the key right now.
    pub fn held_elsewhere(&self) -> bool {
        self.lock.lock().unwrap_or_else(|e| e.into_inner()).is_none()
            && self.places.lock.exists()
            && crate::link::held_elsewhere(&self.places.lock)
    }

    fn set_trouble(&self, why: Option<String>) {
        let mut t = self.trouble.lock().unwrap_or_else(|e| e.into_inner());
        if *t != why
            && let Some(w) = &why
        {
            log(format!("keys: {w}"));
        }
        *t = why;
    }

    async fn stop(&self) {
        let old = self.runtime.write().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(rt) = old {
            rt.stop().await;
        }
    }

    /// Keeps keys matching the files: looks every `every`, and at once when something here changed them.
    pub async fn supervise(self: Arc<Shared>, every: Duration) {
        loop {
            self.tick().await;
            tokio::select! {
                _ = tokio::time::sleep(every) => {}
                _ = self.reload.notified() => {}
            }
        }
    }

    /// One look at the files: starts, restarts or stops keys to match them.
    pub async fn tick(self: &Arc<Shared>) {
        let _busy = self.busy.lock().await;
        if self.kit.held_back() {
            return; // an answer is on its way to the computer that asked this one to take over
        }
        // (a stop half way through taking over as the home computer is finished first)
        if let Err(e) = self.settle_locked().await {
            log(format!("keys: taking over as the home computer is not finished yet: {e}"));
        }
        self.kit.look();
        let cfg = self.config();
        let role = role_of(self.mode(), cfg.as_ref());
        let home = match &role {
            Role::Member(h) => Some(h),
            _ => None,
        };
        let relay = self.relay_for(cfg.as_ref(), home);
        let print = print_of(&role, &relay.0);
        let mut was_on = false;
        if let Some(rt) = self.runtime() {
            if rt.print == print {
                return self.upkeep(&rt, cfg.as_ref());
            }
            if role != Role::Off {
                log("keys: the settings changed, starting again");
            }
            self.stop().await;
            was_on = true;
        }
        if role == Role::Off {
            // Off: no key, no endpoint, no forwarders. The control port alone stays, so the app on this computer
            // can hear that keys are off and can join a home computer (`join`, which turns this into a member).
            if was_on {
                log("keys: off");
            }
            self.drop_lock();
            self.set_trouble(None);
            self.start_control().await;
            return;
        }
        if !self.take_lock() {
            return self.set_trouble(Some(
                "another switchboard-node on this computer is already using this computer's key, so this one \
                 leaves keys alone"
                    .into(),
            ));
        }
        self.start_control().await;
        match self.start(&role, relay, print).await {
            Ok(rt) => {
                *self.runtime.write().unwrap_or_else(|e| e.into_inner()) = Some(rt);
                self.set_trouble(None);
            }
            Err(e) => self.set_trouble(Some(e)),
        }
    }

    async fn start_control(self: &Arc<Shared>) {
        if self.control.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
            return;
        }
        match tokio::net::TcpListener::bind(self.ports.control).await {
            Ok(l) => {
                let shared = self.clone();
                let task = tokio::spawn(async move {
                    if let Err(e) = crate::control::serve(shared, l).await {
                        log(format!("keys: the control port stopped ({e})"));
                    }
                });
                *self.control.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
                *self.control_said.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
            Err(e) => {
                // (said once, not at every look: another helper on this computer may simply hold the port)
                let why = e.to_string();
                let mut said = self.control_said.lock().unwrap_or_else(|e| e.into_inner());
                if said.as_deref() != Some(why.as_str()) {
                    log(format!(
                        "keys: cannot listen on {} ({why}), so the app cannot ask this helper about keys",
                        self.ports.control
                    ));
                    *said = Some(why);
                }
            }
        }
    }

    fn key(&self) -> Result<DeviceKey, String> {
        match DeviceKey::load_or_create(&self.places.key) {
            Ok((key, found)) => {
                match found {
                    Found::Made => log(format!("keys: made this computer's key ({})", key.id().short())),
                    Found::Recovered { kept } => log(format!(
                        "keys: this computer's key could not be read: its copy is in use, and the file as it was \
                         is kept as {kept:?}"
                    )),
                    Found::Loaded => {}
                }
                Ok(key)
            }
            Err(e) => Err(format!("this computer's key cannot be read or made at {} ({e})", self.places.key.display())),
        }
    }

    fn options(&self, relay: &Option<String>, tunnel: bool, pair: bool) -> Options {
        Options {
            relay: relay.clone(),
            bind: self.ports.bind,
            relay_only: false,
            tunnel,
            pair,
            clocks: self.clocks.clone(),
        }
    }

    async fn start(
        self: &Arc<Shared>,
        role: &Role,
        relay: (Option<String>, &'static str),
        print: String,
    ) -> Result<Arc<Runtime>, String> {
        if let Some(r) = &relay.0
            && !valid_relay(r)
        {
            return Err(format!("the relay address {r} is not one (it should look like https://relay.example.com)"));
        }
        let key = self.key()?;
        let id = key.id();
        let bound = |e: std::io::Error| format!("the endpoint could not start ({e})");
        match role {
            Role::Off => Err("keys are off".into()),
            Role::Home => {
                let paired = Arc::new(Paired::load(&self.places.paired));
                if let Some(t) = paired.trouble() {
                    log(format!("keys: {t}"));
                }
                let ep = LinkEndpoint::bind(&key, self.options(&relay.0, true, true), paired.clone())
                    .await
                    .map_err(bound)?;
                {
                    let me = Arc::downgrade(self);
                    ep.on_shown(Arc::new(move |list: Vec<u8>| me.upgrade().and_then(|s| s.home_shown(&list))));
                }
                // (a list another home computer signed, from when this computer was not home, is not this
                // one's: the first list signed here replaces it)
                let held = Arc::new(RosterCache::load(&self.places.roster, id));
                let _ = held.trouble();
                let home = HomeSide {
                    paired,
                    offers: Offers::new(),
                    name: crate::hostname_label(),
                    key: DeviceKey::from_bytes(*key.secret_bytes()),
                    held: held.clone(),
                    signing: tokio::sync::Mutex::new(()),
                    took: Mutex::default(),
                    asked: Mutex::default(),
                    removed: Mutex::default(),
                    witness: {
                        let me = Arc::downgrade(self);
                        let take: sb_link::reach::Take = Arc::new(move |envelope: &Envelope| match me.upgrade() {
                            Some(s) => s.home_sees(envelope),
                            None => Ok(false),
                        });
                        Arc::new(Others::new(ep.clone(), held.clone(), take, Arc::new(|_| {})))
                    },
                };
                let machine = self.config().map(|c| c.machine).unwrap_or_default();
                let lists = Arc::new(HomeLists(held.clone(), Arc::downgrade(self)));
                let surface = Surface::new(id, true, self.places.config.clone(), &machine, lists);
                let rt = Arc::new(Runtime {
                    id,
                    ep: ep.clone(),
                    relay,
                    print,
                    tasks: Mutex::default(),
                    home: Some(home),
                    member: None,
                });
                let targets = Targets { api: Some(self.ports.api_target), hub: Some(self.ports.hub_target) };
                let (tx, mut rx) = mpsc::channel::<PairIncoming>(8);
                let (owns, own_streams) = mpsc::channel::<OwnIncoming>(64);
                rt.spawn(async move { ep.serve_with(targets, tx, Some(owns)).await });
                rt.spawn(own::serve(surface, own_streams));
                let (me, r) = (self.clone(), rt.clone());
                rt.spawn(async move {
                    while let Some(inc) = rx.recv().await {
                        tokio::spawn(me.clone().host_one(r.clone(), inc, None));
                    }
                });
                let (me, r) = (self.clone(), rt.clone());
                rt.spawn(async move {
                    me.publish(&r).await;
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    let (mut round, mut heard) = (0u64, false);
                    loop {
                        // Has another computer taken this one's place? Asked at the start, again every few
                        // seconds until one computer has answered (a minute at most, then once a minute), and
                        // every ten minutes after that.
                        if !heard || round.is_multiple_of(10) {
                            heard = me.ask_witnesses(&r).await;
                        }
                        round = round.wrapping_add(1);
                        me.walk(&r).await;
                        // (signs again only when something changed, or the last one is six hours old)
                        me.publish(&r).await;
                        let wait = if !heard && round <= 12 { 5 } else { 60 };
                        tokio::time::sleep(Duration::from_secs(wait)).await;
                    }
                });
                log(format!(
                    "keys: on, this is the home computer ({}), relay {}",
                    id.short(),
                    rt.relay.0.as_deref().unwrap_or("none")
                ));
                Ok(rt)
            }
            Role::Member(home) => {
                // The devices on the home computer's list may reach this computer's own chats (own.rs), and
                // nothing else here: `api` and `hub` lead nowhere on a computer that is not home.
                let cache = Arc::new(RosterCache::load(&self.places.roster, home.id));
                if let Some(t) = cache.trouble() {
                    log(format!("keys: {t}"));
                }
                let blocks = Arc::new(Blocks::load(&self.places.blocked));
                let gate = Arc::new(RosterGate::new(cache.clone(), blocks.clone()));
                let ep = LinkEndpoint::bind(&key, self.options(&relay.0, true, false), gate).await.map_err(bound)?;
                let peer = Peer { id: home.id, relay: relay.0.clone(), addrs: home.addrs.clone() };
                let tunnel = Tunnel::new(ep.clone(), peer);
                let mut tasks = vec![tokio::spawn(tunnel.clone().run())];
                let machine = self.config().map(|c| c.machine).unwrap_or_default();
                let lists = Arc::new(MemberLists { cache: cache.clone(), blocks: blocks.clone(), ep: ep.clone() });
                let surface = Surface::new(id, false, self.places.config.clone(), &machine, lists);
                surface.set_moves(Arc::new(home::Taker { shared: Arc::downgrade(self), home: home.id }));
                let (pairs, _nobody_pairs_here) = mpsc::channel::<PairIncoming>(1);
                let (owns, own_streams) = mpsc::channel::<OwnIncoming>(64);
                let serving = ep.clone();
                tasks.push(tokio::spawn(async move {
                    serving.serve_with(Targets { api: None, hub: None }, pairs, Some(owns)).await
                }));
                tasks.push(tokio::spawn(own::serve(surface.clone(), own_streams)));
                // The two ports that lead to the home computer, and stand in for it while it is away
                // (sb-link standin.rs).
                let take: sb_link::reach::Take = {
                    let (cache, blocks, ep) = (cache.clone(), blocks.clone(), ep.clone());
                    Arc::new(move |envelope: &Envelope| take_roster(&cache, &blocks, Some(&ep), envelope))
                };
                let reach = Arc::new(Reach {
                    id,
                    machine,
                    home: tunnel.clone(),
                    cache: cache.clone(),
                    surface: surface.clone(),
                    after: self.ports.away_after,
                    others: Others::new(ep.clone(), cache.clone(), take, Arc::new(|s| log(format!("keys: {s}")))),
                });
                let opts = standin::Options { away_after: self.ports.away_after, ..standin::Options::default() };
                let standin = StandIn::new(reach.clone(), opts);
                standin.set_hub_port(self.ports.hub_forward.map_or(8132, |a| a.port()));
                tasks.push(tokio::spawn(standin.clone().run()));
                let mut forwards = vec![];
                for (addr, target) in [(self.ports.api_forward, Target::Api), (self.ports.hub_forward, Target::Hub)] {
                    let Some(addr) = addr else { continue };
                    let mut row =
                        ForwardRow { target: target.name().into(), addr: addr.to_string(), ok: true, error: None };
                    // (loopback only: the port is this computer's door to home, and it is not for the network)
                    let bound = if addr.ip().to_canonical().is_loopback() {
                        tokio::net::TcpListener::bind(addr).await
                    } else {
                        Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not an address on this computer"))
                    };
                    match bound {
                        Ok(listener) => {
                            if let (Target::Hub, Ok(at)) = (target, listener.local_addr()) {
                                standin.set_hub_port(at.port());
                            }
                            tasks.push(tokio::spawn(standin.clone().serve(listener, target)));
                        }
                        Err(e) => {
                            log(format!(
                                "keys: {addr} on this computer is in use ({e}), so programs here cannot reach the \
                                 home computer's {} through it. Everything else keeps running.",
                                if target == Target::Api { "server" } else { "hub" }
                            ));
                            (row.ok, row.error) = (false, Some(e.to_string()));
                        }
                    }
                    forwards.push(row);
                }
                log(format!(
                    "keys: on, connecting to the home computer {} ({}), relay {}",
                    home.name,
                    home.id.short(),
                    relay.0.as_deref().unwrap_or("none")
                ));
                let rt = Arc::new(Runtime {
                    id,
                    ep,
                    relay,
                    print,
                    tasks: Mutex::new(tasks),
                    home: None,
                    member: Some(MemberSide {
                        home: home.clone(),
                        tunnel: tunnel.clone(),
                        forwards,
                        reach,
                        standin,
                        cache,
                        blocks,
                    }),
                });
                // The list is asked for each time the home computer is reached, and once a minute after that.
                let (me, r) = (self.clone(), rt.clone());
                rt.spawn(async move {
                    let (mut was_up, mut last) = (false, None::<std::time::Instant>);
                    let mut shown = None::<std::time::Instant>;
                    let mut shown_wait = PULL_EVERY;
                    loop {
                        let status = tunnel.status();
                        let up = status.connected;
                        // A home computer that refuses this device is shown the list this device holds: it
                        // may be one brought back from a backup made before this device was paired.
                        if status.refused
                            && shown.is_none_or(|t| t.elapsed() >= shown_wait)
                            && let Some(m) = &r.member
                        {
                            shown = Some(std::time::Instant::now());
                            // (one that did not get through is tried again soon)
                            shown_wait = match m.reach.others.show(&tunnel.peer()).await {
                                Ok(_) => PULL_EVERY,
                                Err(_) => Duration::from_secs(5),
                            };
                        }
                        if up && (!was_up || last.is_none_or(|t| t.elapsed() >= PULL_EVERY)) {
                            let _ = me.pull(&r).await;
                            last = Some(std::time::Instant::now());
                        }
                        was_up = up;
                        tokio::time::sleep(Duration::from_millis(250)).await;
                    }
                });
                Ok(rt)
            }
        }
    }

    /// Now and then, while running: remembers where home was last reached, so the next start can go straight
    /// there.
    fn upkeep(&self, rt: &Runtime, cfg: Option<&NodeConfig>) {
        let (Some(m), Some(cfg)) = (&rt.member, cfg) else { return };
        let Some(home) = &cfg.home else { return };
        let Some(at) = m.tunnel.status().remote else {
            // Not reached where it was: the home computer may have moved to one of the other computers, so the
            // places those were last reached are tried as well (its id is the same wherever it is).
            if !m.tunnel.status().connected
                && let Some(r) = m.cache.get()
            {
                let mut addrs = home.addrs.clone();
                for a in
                    r.body.devices.iter().filter(|d| d.kind == Kind::Computer && d.id != rt.id).flat_map(|d| &d.addrs)
                {
                    if !addrs.contains(a) && addrs.len() < 16 {
                        addrs.push(*a);
                    }
                }
                m.tunnel.set_addrs(addrs);
            }
            return;
        };
        if home.addrs.first() != Some(&at) {
            let mut addrs = vec![at];
            addrs.extend(home.addrs.iter().filter(|a| **a != at).take(3));
            m.tunnel.set_addrs(addrs.clone());
            let mut next = cfg.clone();
            next.home = Some(HomeConfig { addrs, ..home.clone() });
            let _ = crate::save_to(&self.places.config, &next);
        }
    }

    // ---- home: pairing ---------------------------------------------------------------------------------------

    fn home_of(rt: &Runtime) -> Result<&HomeSide, String> {
        rt.home.as_ref().ok_or_else(|| "Only the home computer does that.".to_string())
    }

    /// A new short code and link, good for ten minutes.
    pub async fn pair_new(self: &Arc<Shared>) -> Result<PairNew, Refusal> {
        if self.kit.frozen() {
            return Err(Refusal::new("moving", home::MOVING));
        }
        let rt = self.runtime().ok_or_else(|| self.idle())?;
        let home = Shared::home_of(&rt).map_err(|e| Refusal::new("not_home", e))?;
        let now = unix_now();
        let (secret, expires) = home.offers.mint_link(now);
        let link = PairLink { home: rt.id, relay: rt.relay.0.clone(), secret, name: home.name.clone() }.to_string();
        let mut code = None;
        if rt.relay.0.is_some() {
            let (c, until) = home.offers.mint_code(now);
            match self.clone().meet(rt.clone(), c.clone(), until).await {
                Ok(()) => code = Some(c.to_string()),
                Err(e) => {
                    home.offers.revoke(&Secret::from(c));
                    log(format!("keys: no short code this time ({e})"));
                }
            }
        }
        Ok(PairNew { ok: true, code, link, expires })
    }

    /// The place a short code leads to: an endpoint whose key comes from the code, at the relay, until the code
    /// is used or runs out.
    async fn meet(self: Arc<Shared>, rt: Arc<Runtime>, code: ShortCode, until: u64) -> Result<(), String> {
        let c = code.clone();
        let secret = tokio::task::spawn_blocking(move || c.rendezvous_key()).await.map_err(|e| e.to_string())?;
        let key = DeviceKey::from_bytes(secret);
        let ep = LinkEndpoint::bind(&key, self.options(&rt.relay.0, false, true), Arc::new(Nobody))
            .await
            .map_err(|e| e.to_string())?;
        if !ep.relay_ready(Duration::from_secs(8)).await {
            ep.close().await;
            return Err("the relay did not answer".into());
        }
        let (tx, mut rx) = mpsc::channel::<PairIncoming>(4);
        let serving = ep.clone();
        let serve = tokio::spawn(async move { serving.serve(Targets { api: None, hub: None }, tx).await });
        let r = rt.clone();
        rt.spawn(async move {
            let live = |r: &Runtime| r.home.as_ref().is_some_and(|h| h.offers.codes(unix_now()).contains(&code));
            while live(&r) && unix_now() < until {
                tokio::select! {
                    inc = rx.recv() => match inc {
                        Some(inc) => self.clone().host_one(r.clone(), inc, Some(code.clone())).await,
                        None => break,
                    },
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                }
            }
            serve.abort();
            ep.close().await;
        });
        Ok(())
    }

    /// One device's pairing exchange.
    async fn host_one(self: Arc<Shared>, rt: Arc<Runtime>, mut inc: PairIncoming, code: Option<ShortCode>) {
        let Some(home) = &rt.home else { return };
        let channel = inc.channel;
        let binding = Binding { host: inc.host.0, joiner: inc.joiner.0, channel: &channel };
        let about = pairing::Home { id: rt.id, name: home.name.clone(), relay: rt.relay.0.clone() };
        let select = match &code {
            Some(c) => Select::Code(c),
            None => Select::Link,
        };
        let me = self.clone();
        let minting = rt.clone();
        let mint = |req: Request| async move { me.mint(&minting, req).await };
        let joiner = inc.joiner;
        match pairing::host(&mut inc.stream, &home.offers, select, &binding, &about, unix_now(), mint).await {
            Ok(j) => {
                log(format!("keys: paired {} ({})", j.about.name, j.id.short()));
                self.publish(&rt).await;
            }
            Err(PairError::Undelivered { machine, error }) => {
                // it was added and never heard so: take it out again, here and at the hub
                let _ = home.paired.remove(&joiner);
                if let Some(m) = machine {
                    let _ = self.hub("POST", &format!("/internal/unpair/{m}"), Some(&json!({})), 20).await;
                }
                log(format!("keys: a device paired and the answer never reached it ({error}); it was not kept"));
            }
            Err(e) => log(format!("keys: a pairing attempt failed: {e}")),
        }
        inc.finish().await;
    }

    async fn hub(&self, method: &str, path: &str, body: Option<&Value>, secs: u64) -> Result<(u16, Value), String> {
        crate::http_json(&self.ports.hub.to_string(), method, path, body, Duration::from_secs(secs)).await
    }

    /// Adds a device that proved it holds a code or link: a computer also gets a name and a token at the hub,
    /// made the way Add a machine makes them, so the hub needs to know nothing new.
    async fn mint(&self, rt: &Runtime, req: Request) -> Result<Grant, String> {
        if self.kit.frozen() {
            return Err(home::MOVING.into());
        }
        let home = Shared::home_of(rt)?;
        let before = home.paired.get(&req.id);
        let mut grant = Grant::default();
        if req.about.kind == Kind::Computer {
            // a computer that pairs again keeps its name: its old link is taken away first
            let known = before.and_then(|d| d.machine);
            if let Some(m) = &known {
                let _ = self.hub("POST", &format!("/internal/unpair/{m}"), Some(&json!({})), 20).await;
            }
            let base = known.unwrap_or_else(|| crate::name_from(&req.about.name));
            let mut made = None;
            for n in 1..=9u32 {
                let name = if n == 1 { base.clone() } else { format!("{}{n}", &base[..base.len().min(31)]) };
                let (status, v) = self.hub("POST", "/internal/pair", Some(&json!({})), 20).await?;
                let Some(code) = v["code"].as_str().filter(|_| status == 200) else {
                    return Err(format!("the hub on the home computer answered {status}"));
                };
                let body = json!({"code": code, "name": name, "label": req.about.name, "os": req.about.os});
                let (status, v) = self.hub("POST", "/pair", Some(&body), 20).await?;
                match (status, v["machine"].as_str(), v["token"].as_str()) {
                    (200, Some(m), Some(t)) => {
                        made = Some((m.to_string(), t.to_string()));
                        break;
                    }
                    (409, ..) => continue, // that name is another computer's: try the next
                    _ => {
                        let why = v["error"].as_str().map(str::to_string);
                        return Err(why.unwrap_or_else(|| format!("the hub on the home computer answered {status}")));
                    }
                }
            }
            let (machine, token) = made.ok_or("every name for this computer is taken at the hub")?;
            home.removed.lock().unwrap_or_else(|e| e.into_inner()).remove(&machine);
            grant = Grant { machine: Some(machine), token: Some(token) };
        }
        let device = Device {
            id: req.id,
            name: req.about.name.clone(),
            kind: req.about.kind,
            machine: grant.machine.clone(),
            paired: unix_now(),
            via: req.via,
        };
        if let Err(e) = home.paired.add(device) {
            if let Some(m) = &grant.machine {
                let _ = self.hub("POST", &format!("/internal/unpair/{m}"), Some(&json!({})), 20).await;
            }
            return Err(format!("the list of paired devices could not be saved ({e})"));
        }
        Ok(grant)
    }

    // ---- home: the list ---------------------------------------------------------------------------------------

    pub fn devices(&self) -> Result<Vec<DeviceRow>, String> {
        let rt = self.running()?;
        let home = Shared::home_of(&rt)?;
        let peers = rt.ep.peers();
        let held = home.held.get();
        let took = home.took.lock().unwrap_or_else(|e| e.into_inner()).clone();
        Ok(home
            .paired
            .list()
            .into_iter()
            .map(|d| {
                let peer = peers.iter().find(|p| p.id == d.id);
                DeviceRow {
                    ver: held.as_ref().and_then(|r| r.device(&d.id)).and_then(|k| k.ver.clone()),
                    // (what the last list says stands until this helper has asked for itself, so a home
                    // computer that just started still says who answers while it is away)
                    answers_away: d
                        .machine
                        .as_ref()
                        .and_then(|m| took.get(m).copied())
                        .or_else(|| held.as_ref().and_then(|r| r.device(&d.id)).and_then(|k| k.own)),
                    // A connection that is up now was seen now, however long ago it last opened a stream.
                    last_seen: if peer.is_some() { Some(unix_now()) } else { rt.ep.last_seen(&d.id) },
                    path: peer.and_then(|p| p.path).map(|p| p.name().to_string()),
                    rtt_ms: peer.and_then(|p| p.rtt_ms),
                    ..DeviceRow::of(d)
                }
            })
            .collect())
    }

    /// Forgets a device: it is refused from now on, what it had open is closed, and a computer's link at the
    /// hub is taken away with it. Ok(false) when no such device was paired.
    pub async fn remove(self: &Arc<Shared>, id: &DeviceId) -> Result<bool, String> {
        if self.kit.frozen() {
            return Err(home::MOVING.into());
        }
        let rt = self.running()?;
        let home = Shared::home_of(&rt)?;
        let device = home.paired.get(id);
        let was =
            home.paired.remove(id).map_err(|e| format!("The list of paired devices could not be saved ({e})."))?;
        rt.ep.cut(id);
        rt.ep.sweep();
        if was {
            log(removed_line(device.as_ref(), id));
            // the other computers hear it now: the list is signed without it and sent to each of them
            let (me, r) = (self.clone(), rt.clone());
            tokio::spawn(async move { me.publish(&r).await });
        }
        if let Some(m) = device.and_then(|d| d.machine) {
            home.removed.lock().unwrap_or_else(|e| e.into_inner()).insert(m.clone());
            match self.hub("POST", &format!("/internal/unpair/{m}"), Some(&json!({})), 20).await {
                Ok((200 | 404, _)) => {}
                Ok((s, _)) => log(format!("keys: removed {m}, but the hub answered {s} when asked to forget it")),
                Err(e) => log(format!("keys: removed {m}, but the hub could not be told ({e})")),
            }
        }
        Ok(was)
    }

    // ---- home: the signed list of devices ---------------------------------------------------------------------

    /// Signs the list of devices again when it changed, or when the last one is `RESIGN_EVERY` old, and sends
    /// it to every linked computer. A change in who is on it raises `seq`; anything else moves `at` only.
    pub async fn publish(&self, rt: &Runtime) {
        let Some(home) = &rt.home else { return };
        let signed = {
            let _one = home.signing.lock().await;
            let nodes = match self.hub("GET", "/internal/nodes", None, 20).await {
                Ok((200, v)) => v["nodes"].as_array().cloned().unwrap_or_default(),
                _ => vec![],
            };
            let last = home.held.get();
            let peers = rt.ep.peers();
            let here = |a: &SocketAddr| !a.ip().is_unspecified() && !a.ip().is_loopback();
            let machine = self.config().map(|c| c.machine).filter(|m| !m.is_empty());
            // This computer is named as the server names it: the label the hub has for its machine. When the
            // hub does not say just now, the name on the last list stands, so the list is not signed again for
            // a hub that was slow to answer; the host name is for a home computer that runs no chats.
            let label = machine
                .as_deref()
                .and_then(|m| nodes.iter().find(|n| n["name"] == m))
                .and_then(|n| n["label"].as_str())
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .or_else(|| last.as_ref().and_then(|l| l.device(&rt.id)).map(|d| d.name.clone()))
                .filter(|l| !l.is_empty())
                .unwrap_or_else(|| home.name.clone());
            let took = home.took.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let mut devices = vec![RosterDevice {
                id: rt.id,
                name: label.clone(),
                kind: Kind::Computer,
                machine,
                ver: Some(crate::VERSION.to_string()),
                own: Some(true),
                addrs: rt.ep.addrs().into_iter().filter(here).take(roster::MAX_ADDRS).collect(),
            }];
            for d in home.paired.list() {
                // (what was known and cannot be asked just now is kept: a computer that is off has a version too)
                let known = last.as_ref().and_then(|l| l.device(&d.id));
                let node = d.machine.as_deref().and_then(|m| nodes.iter().find(|n| n["name"] == m));
                let ver = node
                    .and_then(|n| n["version"].as_str())
                    .filter(|v| !v.is_empty())
                    .map(str::to_string)
                    .or_else(|| known.and_then(|k| k.ver.clone()));
                let mut addrs: Vec<SocketAddr> =
                    peers.iter().find(|p| p.id == d.id).and_then(|p| p.remote).into_iter().collect();
                for a in known.map(|k| k.addrs.as_slice()).unwrap_or(&[]) {
                    if !addrs.contains(a) && addrs.len() < roster::MAX_ADDRS {
                        addrs.push(*a);
                    }
                }
                // (what this helper found out since it started, else what the last list said)
                let own = match (&d.kind, &d.machine) {
                    (Kind::Computer, Some(m)) => took.get(m).copied().or_else(|| known.and_then(|k| k.own)),
                    _ => None,
                };
                devices.push(RosterDevice {
                    id: d.id,
                    name: d.name,
                    kind: d.kind,
                    machine: d.machine,
                    ver,
                    own,
                    addrs,
                });
            }
            let store = Store::new(&self.places.roster_state).private().bak(Bak::Fresh);
            let mut c = store.load_json::<Counters>().0.unwrap_or_default();
            if let Some(l) = &last
                && (l.body.epoch, l.body.seq) > (c.epoch, c.seq)
            {
                (c.epoch, c.seq) = (l.body.epoch, l.body.seq);
            }
            (c.v, c.epoch) = (1, c.epoch.max(1));
            let now = unix_now();
            let at = now.max(last.as_ref().map_or(0, |l| l.body.at + 1));
            let body = |seq: u64, devices: Vec<RosterDevice>| Body {
                home: rt.id,
                home_name: label.clone(),
                epoch: c.epoch,
                seq,
                at,
                relay: rt.relay.0.clone(),
                devices,
            };
            let no = |e: RosterError| log(format!("keys: the list of devices could not be signed ({e})"));
            // Signed once to see it as the others will (names tidied), and compared with the last one.
            let draft = match roster::sign(&home.key, body(c.seq, devices)) {
                Ok(r) => r,
                Err(e) => return no(e),
            };
            let who = |b: &Body| {
                let devices: Vec<_> =
                    b.devices.iter().map(|d| (d.id, d.name.clone(), d.kind, d.machine.clone())).collect();
                (b.home_name.clone(), b.relay.clone(), devices)
            };
            let vers = |b: &Body| b.devices.iter().map(|d| (d.ver.clone(), d.own)).collect::<Vec<_>>();
            let same = last.as_ref().is_some_and(|l| who(&l.body) == who(&draft.body));
            // (an address nobody was told yet: without it two computers on one network would not find each
            // other when there is no relay)
            let news = |l: &Roster| {
                draft.body.devices.iter().any(|d| {
                    let had = l.device(&d.id).map(|k| k.addrs.as_slice()).unwrap_or(&[]);
                    d.addrs.iter().any(|a| !had.contains(a))
                })
            };
            if let Some(l) = &last
                && same
                && vers(&l.body) == vers(&draft.body)
                && !news(l)
                && now < l.body.at + roster::RESIGN_EVERY
            {
                return;
            }
            let signed = if same {
                draft
            } else {
                // the number is on disk before any list carries it, so it is never given out twice
                c.seq += 1;
                if let Err(e) = store.put_json(&c, true) {
                    return log(format!(
                        "keys: {} could not be saved ({e}), so the list of devices was not signed",
                        self.places.roster_state.display()
                    ));
                }
                match roster::sign(&home.key, body(c.seq, draft.body.devices)) {
                    Ok(r) => r,
                    Err(e) => return no(e),
                }
            };
            if let Err(e) = home.held.offer(&signed.envelope, at) {
                return no(e);
            }
            if let Some(t) = home.held.trouble() {
                log(format!("keys: {t}"));
            }
            if !same {
                log(format!(
                    "keys: signed the list of devices (number {}, {} on it)",
                    signed.body.seq,
                    signed.body.devices.len()
                ));
            }
            signed
        };
        let ask = json!({"verb": "node.roster", "roster": signed.envelope});
        for d in signed.body.devices.iter().filter(|d| d.id != rt.id) {
            let Some(m) = &d.machine else { continue };
            let took = match self.hub("POST", &format!("/internal/rpc/{m}?timeout=10"), Some(&ask), 15).await {
                Ok((200, a)) => took_of(&a),
                // not linked just now: it asks for the list itself the next time it connects
                _ => None,
            };
            if let Some(took) = took {
                home.took.lock().unwrap_or_else(|e| e.into_inner()).insert(m.clone(), took);
            }
        }
    }

    /// One look at whether the list needs signing, now (the supervisor's own runs once a minute).
    #[cfg(test)]
    pub async fn publish_now(&self) {
        if let Some(rt) = self.runtime() {
            self.publish(&rt).await;
        }
    }

    /// The list of devices this computer holds: the one it signed (home) or the one it took.
    pub fn roster(&self) -> Option<Arc<Roster>> {
        let rt = self.runtime()?;
        match (&rt.home, &rt.member) {
            (Some(h), _) => h.held.get(),
            (_, Some(m)) => m.cache.get(),
            _ => None,
        }
    }

    // ---- any other computer: the list of devices -------------------------------------------------------------

    /// Asks the home computer for its list of devices, over the tunnel. Ok(true): a newer one was taken.
    async fn pull(&self, rt: &Runtime) -> Result<bool, String> {
        let m = rt.member.as_ref().ok_or("this is the home computer")?;
        m.reach.others.pull(&m.tunnel).await
    }

    #[cfg(test)]
    pub async fn pull_now(&self) -> Result<bool, String> {
        match self.runtime() {
            Some(rt) => self.pull(&rt).await,
            None => Err("not running".into()),
        }
    }

    /// A computer's answer to home's `node.roster`, which reached it over the hub link: the list is taken if it
    /// is newer than the one held. Answered by this helper itself; the helper script never sees it.
    pub fn node_roster(&self, req: &Value) -> Value {
        let refuse = |kind: &str, why: String| json!({"ok": false, "error": why, "kind": kind});
        let Ok(envelope) = serde_json::from_value::<Envelope>(req["roster"].clone()) else {
            return refuse("bad_request", "That is not a list of devices.".into());
        };
        let rt = self.runtime();
        if rt.as_ref().is_some_and(|r| r.home.is_some()) || self.mode() != Mode::Off {
            return json!({"ok": false, "is_home": true, "error": "This is the home computer."});
        }
        let done = match rt.as_ref().and_then(|r| r.member.as_ref().map(|m| (r, m))) {
            Some((r, m)) => take_roster(&m.cache, &m.blocks, Some(&r.ep), &envelope).map(|t| (t, m.cache.stamp())),
            // keys are not running here just now: the list is kept for when they are
            None => match self.config().and_then(|c| c.home) {
                Some(h) => {
                    let cache = RosterCache::load(&self.places.roster, h.id);
                    let blocks = Blocks::load(&self.places.blocked);
                    take_roster(&cache, &blocks, None, &envelope).map(|t| (t, cache.stamp()))
                }
                None => return refuse("not_here", "This computer is not paired with a home computer.".into()),
            },
        };
        match done {
            Ok((took, have)) => json!({"ok": true, "took": took, "have": have}),
            Err(e) => refuse("bad_request", format!("That list of devices was not taken: {e}.")),
        }
    }

    // ---- any other computer: devices it does not answer ---------------------------------------------------

    fn member_of(rt: &Runtime) -> Result<&MemberSide, String> {
        rt.member.as_ref().ok_or_else(|| {
            "The home computer does not block devices. Remove one instead: switchboard-node forget <name>".to_string()
        })
    }

    /// The home computer's list of devices as this computer holds it, with the ones it blocks marked.
    pub fn roster_rows(&self) -> Result<Value, String> {
        let rt = self.running()?;
        let Some(r) = self.roster() else {
            return Ok(json!({"ok": true, "roster": null, "devices": []}));
        };
        let blocked = |id: &DeviceId| rt.member.as_ref().is_some_and(|m| m.blocks.blocks(id, r.stamp()));
        let devices: Vec<Value> = r
            .body
            .devices
            .iter()
            .map(|d| {
                json!({
                    "id": d.id.to_string(), "name": d.name, "kind": d.kind, "machine": d.machine, "ver": d.ver,
                    "home": d.id == r.body.home, "this": d.id == rt.id, "blocked": blocked(&d.id),
                })
            })
            .collect();
        Ok(json!({
            "ok": true,
            "roster": {"epoch": r.body.epoch, "seq": r.body.seq, "at": r.body.at, "expires": r.expires()},
            "devices": devices,
        }))
    }

    /// Stops answering a device here, although the home computer's list names it, and cuts it off now. It
    /// lasts until the home computer's list changes. Ok: the device's name.
    pub fn block(&self, id: &DeviceId) -> Result<String, String> {
        let rt = self.running()?;
        let m = Shared::member_of(&rt)?;
        let r = m.cache.get().ok_or("This computer holds no list of devices yet.")?;
        let device = r.device(id).ok_or("No such device is on the home computer's list.")?;
        if *id == r.body.home {
            return Err("The home computer cannot be blocked.".into());
        }
        if *id == rt.id {
            return Err("That is this computer.".into());
        }
        m.blocks
            .block(*id, &device.name, r.stamp(), unix_now())
            .map_err(|e| format!("{} could not be saved ({e}).", self.places.blocked.display()))?;
        rt.ep.cut(id);
        rt.ep.sweep();
        log(format!(
            "keys: {} ({}) is blocked on this computer until the home computer's list of devices changes",
            device.name,
            id.short()
        ));
        Ok(device.name.clone())
    }

    /// Ok(true): the device was blocked here and no longer is.
    pub fn unblock(&self, id: &DeviceId) -> Result<bool, String> {
        let rt = self.running()?;
        let m = Shared::member_of(&rt)?;
        let was =
            m.blocks.unblock(id).map_err(|e| format!("{} could not be saved ({e}).", self.places.blocked.display()))?;
        if was {
            log(format!("keys: {} is no longer blocked on this computer", id.short()));
        }
        Ok(was)
    }

    // ---- home: the computers that were here before keys ------------------------------------------------------

    /// Asks each computer the hub lists as online, and that is not paired yet, to switch to keys. A helper that
    /// knows how makes its key, notes where home is and answers with its id, and is paired on the spot (it
    /// already holds a token for the hub, which is how the question reached it). An older helper answers with
    /// an error: it needs an update. Each is asked once per version.
    pub async fn walk(&self, rt: &Runtime) {
        let Some(home) = &rt.home else { return };
        let Ok((200, v)) = self.hub("GET", "/internal/nodes", None, 20).await else { return };
        let paired = home.paired.list();
        for n in v["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let (Some(name), true) = (n["name"].as_str(), n["online"].as_bool().unwrap_or(false)) else { continue };
            let version = n["version"].as_str().unwrap_or("").to_string();
            if paired.iter().any(|d| d.machine.as_deref() == Some(name))
                || home.removed.lock().unwrap_or_else(|e| e.into_inner()).contains(name)
                || home.asked.lock().unwrap_or_else(|e| e.into_inner()).get(name).is_some_and(|a| a.0 == version)
            {
                continue;
            }
            let ask = json!({"verb": "node.keys", "home": {
                "id": rt.id.to_string(), "name": home.name, "relay": rt.relay.0, "addrs": rt.ep.addrs(),
            }});
            let Ok((status, a)) = self.hub("POST", &format!("/internal/rpc/{name}?timeout=30"), Some(&ask), 45).await
            else {
                continue;
            };
            if matches!(status, 503 | 504) || a["is_home"] == true {
                continue; // not linked just now, or it is this computer: nothing to note
            }
            let id = a["id"].as_str().and_then(|s| s.parse::<DeviceId>().ok()).filter(|id| id.is_key() && *id != rt.id);
            let state = match (a["ok"].as_bool(), id) {
                (Some(true), Some(id)) => {
                    let label = n["label"].as_str().filter(|l| !l.is_empty()).unwrap_or(name);
                    let device = Device {
                        id,
                        name: label.to_string(),
                        kind: Kind::Computer,
                        machine: Some(name.to_string()),
                        paired: unix_now(),
                        via: Via::Upgrade,
                    };
                    match home.paired.add(device) {
                        Ok(()) => {
                            log(format!("keys: {name} switched to keys ({})", id.short()));
                            "paired"
                        }
                        Err(e) => {
                            log(format!("keys: {name} could not be added to the paired list ({e})"));
                            continue;
                        }
                    }
                }
                _ => {
                    log(format!("keys: {name} needs an update before it can use keys"));
                    "needs update"
                }
            };
            home.asked.lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), (version, state.to_string()));
        }
    }

    /// One walk, now (the supervisor's own runs once a minute).
    #[cfg(test)]
    pub async fn walk_now(&self) {
        if let Some(rt) = self.runtime() {
            self.walk(&rt).await;
        }
    }

    /// A computer's answer to home's `node.keys`, which reached it over the hub link it already has: makes
    /// this computer's key, notes where home is, and says who this computer is.
    pub fn node_keys(&self, req: &Value) -> Value {
        let refuse = |why: &str| json!({"ok": false, "error": why});
        if self.mode() != Mode::Off {
            return json!({"ok": false, "is_home": true, "error": "This is the home computer."});
        }
        let h = &req["home"];
        let Some(id) = h["id"].as_str().and_then(|s| s.parse::<DeviceId>().ok()).filter(DeviceId::is_key) else {
            return refuse("The home computer did not say who it is.");
        };
        let relay = h["relay"].as_str().filter(|r| !r.is_empty()).map(str::to_string);
        if relay.as_deref().is_some_and(|r| !valid_relay(r)) {
            return refuse("The relay address is not one.");
        }
        let addrs: Vec<SocketAddr> = h["addrs"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()?.parse().ok()).take(8).collect())
            .unwrap_or_default();
        let Some(mut cfg) = self.config() else { return refuse("This computer has no settings file.") };
        if cfg.home.as_ref().is_some_and(|old| old.id != id) {
            return refuse("This computer is paired with another home computer. Pair it again to move it.");
        }
        if self.held_elsewhere() {
            return refuse("Another switchboard-node on this computer is using its key.");
        }
        let key = match self.key() {
            Ok(k) => k,
            Err(e) => return refuse(&e),
        };
        let name: String = h["name"].as_str().unwrap_or("").chars().filter(|c| !c.is_control()).take(64).collect();
        cfg.home = Some(HomeConfig { id, relay, name, addrs });
        if let Err(e) = crate::save_to(&self.places.config, &cfg) {
            return refuse(&format!("The settings could not be saved ({e})."));
        }
        self.reload.notify_one();
        json!({"ok": true, "id": key.id().to_string(), "name": crate::hostname_label(), "os": crate::os_name(),
               "version": crate::VERSION})
    }

    // ---- any other computer: joining -------------------------------------------------------------------------

    /// Pairs this computer with a home computer, from a short code or a link, and writes what it learned to
    /// node.json. `addrs` are addresses to try for home when the link names no relay.
    ///
    /// A computer that is paired with one home computer is not moved to another unless `replace` says so: a code
    /// or link from a different home is turned down with kind `other_home`, and nothing is changed here or
    /// there (as `pair <hub> <code>` will not write over a link to another hub). Pairing again with the same
    /// home needs no such word.
    pub async fn join(
        &self,
        typed: &str,
        name: Option<String>,
        addrs: Vec<SocketAddr>,
        replace: bool,
    ) -> Result<Welcome, Refusal> {
        let input = PairInput::parse(typed)
            .map_err(|e| Refusal::new("bad_input", format!("That is not a pairing code or link ({e}).")))?;
        let _busy = self.busy.lock().await;
        // A link says which home computer it is from: one from another home is turned down before anything
        // here is stopped. (A code does not say; the home computer it leads to turns this computer down.)
        if let (PairInput::Link(link), Some(old), false) = (&input, self.config().and_then(|c| c.home), replace)
            && old.id != link.home
            && self.mode() == Mode::Off
        {
            return Err(other_home(&old));
        }
        let had_key = self.places.key.exists();
        let r = self.join_with(input, name, addrs, replace).await;
        if r.is_err() && !had_key {
            // a key made only for a pairing that did not happen is nobody's: the next try makes another
            let name = self.places.key.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let _ = std::fs::remove_file(&self.places.key);
            let _ = std::fs::remove_file(self.places.key.with_file_name(format!("{name}.bak")));
        }
        self.reload.notify_one(); // keys were stopped for this: start them again, paired or not
        r
    }

    async fn join_with(
        &self,
        input: PairInput,
        name: Option<String>,
        addrs: Vec<SocketAddr>,
        replace: bool,
    ) -> Result<Welcome, Refusal> {
        let no = |kind: &'static str, text: &str| Err(Refusal::new(kind, text));
        if self.mode() != Mode::Off {
            return no("is_home", "This is the home computer. Other computers pair with it, not the other way round.");
        }
        if self.kit.unfinished() {
            return no(
                "moving",
                "A move of the home computer was started on this computer and did not finish. Finish it first: \
                 switchboard-node home move --resume",
            );
        }
        let mut cfg = self.config().unwrap_or_default();
        let old = cfg.home.clone();
        self.stop().await; // one endpoint with this key at a time
        if !self.take_lock() {
            return no(
                "key_in_use",
                "Another switchboard-node on this computer is using its key. Stop it and try again.",
            );
        }
        // (a computer that stood down as the home computer still holds that key: it never pairs with it)
        self.drop_home_key();
        let key = self.key().map_err(|e| Refusal::new("key", e))?;
        // (a computer that was paired before knows its home's relay: pairing again by code goes through that)
        let own = self.relay_for(Some(&cfg), cfg.home.as_ref()).0;
        let (peer, secret) = match input {
            PairInput::Link(link) => {
                let relay = link.relay.clone().or(own);
                (Peer { id: link.home, relay, addrs: addrs.clone() }, Secret::from(&link))
            }
            PairInput::Code(code) => {
                let Some(relay) = own else {
                    return no(
                        "no_relay",
                        "A short code finds the home computer through a relay, and none is set here. Use the link \
                         instead, or set the relay first: switchboard-node relay <url>",
                    );
                };
                let c = code.clone();
                let id = tokio::task::spawn_blocking(move || c.rendezvous_id())
                    .await
                    .map_err(|e| Refusal::new("internal", e.to_string()))?;
                (Peer { id, relay: Some(relay), addrs: vec![] }, Secret::from(code))
            }
        };
        if peer.relay.is_none() && peer.addrs.is_empty() {
            return no(
                "link_no_relay",
                "That link names no relay, so this computer needs the home computer's address: add --addr <ip:port> \
                 (the home computer's status shows it).",
            );
        }
        let ep = LinkEndpoint::bind(&key, self.options(&peer.relay, false, false), Arc::new(Nobody))
            .await
            .map_err(|e| Refusal::new("endpoint", format!("The endpoint could not start ({e}).")))?;
        let about = About {
            name: name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
                let h = crate::hostname_label();
                if h.is_empty() { "computer".into() } else { h }
            }),
            kind: Kind::Computer,
            os: crate::os_name().into(),
            stays_with: old.as_ref().filter(|_| !replace).map(|o| o.id),
        };
        let result = async {
            let mut out = ep.pair_dial(&peer).await.map_err(|e| {
                if local_network_off(std::env::consts::OS, &e.to_string()) {
                    Refusal::new("local_network", LOCAL_NETWORK)
                } else {
                    Refusal::new("unreachable", format!("Cannot reach the home computer ({e})."))
                }
            })?;
            let channel = out.channel;
            let binding = Binding { host: out.host.0, joiner: out.joiner.0, channel: &channel };
            let r = pairing::join(&mut out.stream, &secret, &binding, &about).await.map_err(|e| match (&e, &old) {
                (PairError::OtherHome, Some(old)) => other_home(old),
                _ => Refusal::new(e.kind(), e.message()),
            });
            out.finish();
            r
        }
        .await;
        ep.close().await;
        drop(ep);
        let welcome = result?;
        let moved = old.as_ref().is_some_and(|o| o.id != welcome.home_id);
        if let (true, false, Some(old)) = (moved, replace, &old) {
            // (only a home computer too old to look at `stays_with` gets this far: nothing is saved here, and
            // it lists this computer until somebody removes it there)
            return Err(other_home(old));
        }
        cfg.home = Some(HomeConfig {
            id: welcome.home_id,
            relay: welcome.relay.clone().or(peer.relay),
            name: welcome.home_name.clone(),
            addrs,
        });
        if let (Some(m), Some(t)) = (&welcome.machine, &welcome.token) {
            (cfg.machine, cfg.token) = (m.clone(), t.clone());
        }
        crate::save_to(&self.places.config, &cfg).map_err(|e| {
            Refusal::new(
                "save",
                format!("Paired, but the settings could not be saved ({e}). Pair again with a new code."),
            )
        })?;
        if moved {
            // The old home computer's list of devices, and what was blocked under it, are not the new one's.
            for file in [&self.places.roster, &self.places.blocked] {
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let _ = std::fs::remove_file(file);
                let _ = std::fs::remove_file(file.with_file_name(format!("{name}.bak")));
            }
            log(format!(
                "keys: this computer left the home computer {} and is paired with {} now",
                old.as_ref().map(|o| o.id.short()).unwrap_or_default(),
                welcome.home_id.short()
            ));
        }
        Ok(welcome)
    }

    /// Sets, or with None clears, the relay in node.json.
    pub fn set_relay(&self, url: Option<&str>) -> Result<(), String> {
        if let Some(u) = url
            && !valid_relay(u)
        {
            return Err(format!("{u} is not a relay address. It should look like https://relay.example.com"));
        }
        let mut cfg = self.config().unwrap_or_default();
        cfg.relay = url.map(str::to_string);
        crate::save_to(&self.places.config, &cfg).map_err(|e| format!("The settings could not be saved ({e})."))?;
        self.reload.notify_one();
        Ok(())
    }

    // ---- status ------------------------------------------------------------------------------------------------

    /// Why nothing is running here: keys are off, or they are on and could not start.
    pub fn idle(&self) -> Refusal {
        if let Some(note) = home::stood_down(&self.kit.folders.config) {
            return Refusal::new("stood_down", home::stood_down_line(&note));
        }
        if self.role() == Role::Off {
            return Refusal::new("off", "Keys are off on this computer.");
        }
        let why = self.trouble.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match why {
            Some(w) => Refusal::new("not_running", format!("Keys are not running on this computer: {w}.")),
            None => Refusal::new("not_running", "Keys are not running on this computer yet."),
        }
    }

    fn running(&self) -> Result<Arc<Runtime>, String> {
        self.runtime().ok_or_else(|| self.idle().text)
    }

    pub fn status(&self) -> Result<Status, String> {
        let rt = self.running()?;
        let cfg = self.config();
        let state = rt.ep.relay();
        let relay = RelayRow {
            url: rt.relay.0.clone(),
            source: rt.relay.1.to_string(),
            connected: state.connected,
            error: state.error,
            same_id: state.same_id,
        };
        let os = std::env::consts::OS;
        let mut problems = vec![];
        match (&relay.url, relay.connected, &relay.error) {
            (None, ..) if rt.home.is_some() => problems.push(
                "No relay is set, so other computers reach this one only where they can connect to it directly \
                 (the same network or a tailnet)."
                    .to_string(),
            ),
            (Some(_), false, Some(e)) if local_network_off(os, e) => problems.push(LOCAL_NETWORK.to_string()),
            (Some(u), false, Some(e)) => problems.push(format!("Cannot reach the relay at {u} ({e}).")),
            _ => {}
        }
        let refused = rt.member.as_ref().is_some_and(|m| m.tunnel.status().refused);
        problems.extend(same_key_line(relay.same_id, refused).map(str::to_string));
        let mut home = None;
        let mut away = None;
        let mut forwarders = vec![];
        if let Some(m) = &rt.member {
            let s = m.tunnel.status();
            if !s.connected {
                match &s.error {
                    _ if s.refused => problems.push(
                        "The home computer no longer lists this computer. Pair it again: switchboard-node pair \
                         <code or link>"
                            .into(),
                    ),
                    Some(e) if local_network_off(os, e) => {
                        if !problems.iter().any(|p| p == LOCAL_NETWORK) {
                            problems.push(LOCAL_NETWORK.to_string());
                        }
                    }
                    Some(e) => problems.push(format!("Cannot reach the home computer ({e}).")),
                    None => {}
                }
            }
            for f in m.forwards.iter().filter(|f| !f.ok) {
                problems.push(format!(
                    "{} on this computer is in use by another program, so programs here cannot reach the home \
                     computer's {} through it.",
                    f.addr,
                    if f.target == "api" { "server" } else { "hub" }
                ));
            }
            forwarders = m.forwards.clone();
            away = m.standin.away();
            if m.blocks.unread() {
                problems.push(
                    "The list of devices blocked here cannot be read, so no device is answered by this computer."
                        .into(),
                );
            }
            if m.cache.get().is_some_and(|r| r.stale(unix_now())) {
                problems.push(
                    "The home computer's list of devices is more than 7 days old, so no device is answered by this \
                     computer until it reaches the home computer again."
                        .into(),
                );
            }
            home = Some(HomeRow {
                id: m.home.id,
                // What the home computer calls itself now, from the newest list it signed. The name written
                // down at pairing is for before any list arrives: it goes stale when the home is renamed, or
                // moved to another computer with its key.
                name: m
                    .cache
                    .get()
                    .map(|r| r.body.home_name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| m.home.name.clone()),
                connected: s.connected,
                path: s.path.map(|p| p.name().to_string()),
                rtt_ms: s.rtt_ms,
                error: s.error,
                refused: s.refused,
            });
        }
        let mut devices = 0;
        let mut updates = vec![];
        if let Some(h) = &rt.home {
            devices = h.paired.list().len();
            if h.paired.unread() {
                problems.push("The list of paired devices cannot be read, so every device is refused.".into());
            }
            let asked = h.asked.lock().unwrap_or_else(|e| e.into_inner());
            updates = asked.iter().map(|(m, a)| UpdateRow { machine: m.clone(), state: a.1.clone() }).collect();
            updates.sort_by(|a, b| a.machine.cmp(&b.machine));
            for u in updates.iter().filter(|u| u.state != "paired") {
                problems.push(format!("{} needs an update before it can use keys.", u.machine));
            }
        }
        Ok(Status {
            ok: true,
            version: crate::VERSION.to_string(),
            role: if rt.home.is_some() { "home" } else { "member" }.to_string(),
            id: rt.id,
            name: crate::hostname_label(),
            machine: cfg.map(|c| c.machine).filter(|m| !m.is_empty()),
            relay,
            home,
            forwarders,
            devices,
            updates,
            addrs: if rt.home.is_some() { rt.ep.addrs().iter().map(|a| a.to_string()).collect() } else { vec![] },
            roster: self.roster().map(|r| RosterRow {
                epoch: r.body.epoch,
                seq: r.body.seq,
                at: r.body.at,
                expires: r.expires(),
            }),
            away,
            problems,
        })
    }

    /// The addresses other computers can try for this one, for a link that names no relay.
    #[cfg(test)]
    pub fn addrs(&self) -> Vec<SocketAddr> {
        self.runtime().map(|r| r.ep.addrs()).unwrap_or_default()
    }

    /// Stops everything (the process is ending).
    pub async fn shutdown(&self) {
        let _busy = self.busy.lock().await;
        self.stop().await;
        if let Some(c) = self.control.lock().unwrap_or_else(|e| e.into_inner()).take() {
            c.abort();
        }
        self.drop_lock();
    }
}

#[path = "home.rs"]
pub mod home;

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
