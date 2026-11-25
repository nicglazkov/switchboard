use super::*;
use crate::keys::home::{Folders, Fut, Kit, Packed, Parts, Staged, StateMover};
use std::sync::atomic::AtomicBool;

/// What travels with the home computer in these tests: the files of a home computer that are in its folder.
const STATE: [&str; 6] = ["server.json", "paired.json", "roster.json", "roster-state.json", "backup.key", "data.txt"];

/// 64 hex digits that change when the bytes do. (The real one is SHA-256, worked out by homestate.py.)
fn digest(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    (0u8..4)
        .map(|i| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (i, bytes).hash(&mut h);
            format!("{:016x}", h.finish())
        })
        .collect()
}

/// Packs and unpacks as homestate.py does, in a few lines: a staging folder, a hash that is checked, and a swap.
#[derive(Default)]
struct FakeMover {
    held: AtomicBool,
}

fn damaged() -> Refusal {
    Refusal::new("damaged", "The file is damaged. Nothing was changed.")
}

impl StateMover for FakeMover {
    fn present(&self) -> bool {
        true
    }

    fn pack<'a>(&'a self, f: &'a Folders, out: &'a Path) -> Fut<'a, Result<Packed, Refusal>> {
        Box::pin(async move {
            let mut files = serde_json::Map::new();
            for name in STATE {
                if let Ok(text) = std::fs::read_to_string(f.state.join(name)) {
                    files.insert(name.into(), json!(text));
                }
            }
            let bytes = Value::Object(files).to_string().into_bytes();
            std::fs::write(out, &bytes).unwrap();
            Ok(Packed { sha256: digest(&bytes), bytes: bytes.len() as u64 })
        })
    }

    fn stage<'a>(
        &'a self,
        f: &'a Folders,
        archive: &'a Path,
        sha256: Option<&'a str>,
    ) -> Fut<'a, Result<Packed, Refusal>> {
        Box::pin(async move {
            let bytes = std::fs::read(archive).map_err(|_| damaged())?;
            let sha = digest(&bytes);
            if sha256.is_some_and(|s| s != sha) {
                return Err(damaged());
            }
            let files: serde_json::Map<String, Value> = serde_json::from_slice(&bytes).map_err(|_| damaged())?;
            let staging = f.state.join(".home-staging");
            let _ = std::fs::remove_dir_all(&staging);
            std::fs::create_dir_all(&staging).unwrap();
            for (name, text) in &files {
                if !STATE.contains(&name.as_str()) {
                    return Err(damaged());
                }
                std::fs::write(staging.join(name), text.as_str().ok_or_else(damaged)?).unwrap();
            }
            std::fs::write(staging.join("note"), &sha).unwrap();
            Ok(Packed { sha256: sha, bytes: bytes.len() as u64 })
        })
    }

    fn swap<'a>(&'a self, f: &'a Folders) -> Fut<'a, Result<(), Refusal>> {
        Box::pin(async move {
            let staging = f.state.join(".home-staging");
            for name in STATE {
                let _ = std::fs::remove_file(f.state.join(name));
                let _ = std::fs::remove_file(f.state.join(format!("{name}.bak")));
                let _ = std::fs::rename(staging.join(name), f.state.join(name));
            }
            let _ = std::fs::remove_dir_all(&staging);
            Ok(())
        })
    }

    fn discard<'a>(&'a self, f: &'a Folders) -> Fut<'a, Result<(), Refusal>> {
        Box::pin(async move {
            let _ = std::fs::remove_dir_all(f.state.join(".home-staging"));
            Ok(())
        })
    }

    fn status<'a>(&'a self, f: &'a Folders) -> Fut<'a, Result<Staged, Refusal>> {
        Box::pin(async move {
            Ok(Staged {
                staged: std::fs::read_to_string(f.state.join(".home-staging/note")).ok(),
                held: self.held.load(Ordering::SeqCst),
            })
        })
    }
}

/// The server and the hub on a computer: what they were told, in order.
#[derive(Default)]
struct FakeParts {
    said: Mutex<Vec<String>>,
}

impl Parts for FakeParts {
    fn present(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, verb: &'a str) -> Fut<'a, Result<(), String>> {
        Box::pin(async move {
            self.said.lock().unwrap().push(verb.to_string());
            Ok(())
        })
    }
}

/// A computer with the parts a home computer needs, as far as these tests go.
async fn able(dir: &Path, hub: SocketAddr, server: SocketAddr, bind: SocketAddr) -> (Arc<Shared>, Arc<FakeParts>) {
    let parts = Arc::new(FakeParts::default());
    let folders = Folders { config: dir.to_path_buf(), state: dir.to_path_buf() };
    let kit = Kit::with(folders, Arc::new(FakeMover::default()), parts.clone());
    (computer_at(dir, hub, server, bind).await.with_kit(kit), parts)
}

fn key_id(dir: &Path) -> Option<DeviceId> {
    DeviceKey::load(&dir.join("device.key")).ok().flatten().map(|(k, _)| k.id())
}

fn note_why(dir: &Path) -> Option<String> {
    home::stood_down(dir).map(|n| n.why)
}

async fn ask_long(k: &Shared, path: &str, body: Value) -> (u16, Value) {
    crate::http_json(&k.ports.control.to_string(), "POST", path, Some(&body), Duration::from_secs(90)).await.unwrap()
}

/// Looks at the files of each computer, as its helper does every few seconds, until `f` holds.
async fn looking(what: &str, ks: &[&Arc<Shared>], mut f: impl FnMut() -> bool) {
    let end = std::time::Instant::now() + Duration::from_secs(40);
    loop {
        for k in ks {
            k.tick().await;
        }
        if f() {
            return;
        }
        assert!(std::time::Instant::now() < end, "never happened: {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn reaches_home(k: &Shared) -> bool {
    k.status().is_ok_and(|s| s.home.is_some_and(|h| h.connected))
}

fn is_home(k: &Shared) -> bool {
    k.status().is_ok_and(|s| s.role == "home")
}

fn epoch_of(k: &Shared) -> u64 {
    k.roster().map_or(0, |r| r.body.epoch)
}

/// One question to `to`'s own surface, asked from `from`'s endpoint.
async fn own_call(from: &Shared, to: Peer, method: &str, path: &str, body: Option<&[u8]>) -> (u16, Value) {
    let ep = from.runtime().unwrap().ep.clone();
    let tunnel = Tunnel::new(ep, to);
    let task = tokio::spawn(tunnel.clone().run());
    until("the two are connected", || tunnel.status().connected).await;
    let mut s = tunnel.open().await.unwrap();
    sb_link::tunnel::write_header(&mut s, Target::Own).await.unwrap();
    let a = sb_link::http1::ask(&mut s, method, path, body, 64 * 1024).await.unwrap();
    tunnel.close();
    task.abort();
    (a.status, a.json().unwrap_or(Value::Null))
}

struct Three {
    hd: PathBuf,
    td: PathBuf,
    home: Arc<Shared>,
    home_parts: Arc<FakeParts>,
    tower: Arc<Shared>,
    tower_parts: Arc<FakeParts>,
    laptop: Arc<Shared>,
    id: DeviceId,
    tower_id: DeviceId,
    hub: SocketAddr,
    server: SocketAddr,
    binds: (SocketAddr, SocketAddr),
}

/// A home computer with something to lose, a tower that could take over, and a laptop that only watches.
async fn three() -> Three {
    let (_fake, hub) = fake_hub().await;
    let server = fake_server("hello from home").await;
    let (hd, td, ld) = (scratch("home"), scratch("tower"), scratch("laptop"));
    let binds = (free_port().await, free_port().await);
    linked(&hd, "homebox");
    keys_on(&hd);
    std::fs::write(hd.join("data.txt"), "everything the home computer knows").unwrap();
    let (home, home_parts) = able(&hd, hub, server, binds.0).await;
    home.tick().await;
    let (_, v) = control(&home, "POST", "/pair/new", Some(json!({}))).await;
    let (tower, tower_parts) = able(&td, hub, server, binds.1).await;
    tower.join(v["link"].as_str().unwrap(), Some("Tower".into()), home.addrs(), false).await.unwrap();
    let (_, v) = control(&home, "POST", "/pair/new", Some(json!({}))).await;
    let laptop = computer(&ld, hub, server).await;
    laptop.join(v["link"].as_str().unwrap(), Some("Laptop".into()), home.addrs(), false).await.unwrap();
    tower.tick().await;
    laptop.tick().await;
    until("both reach home", || reaches_home(&tower) && reaches_home(&laptop)).await;
    until("home sees both", || home.devices().unwrap().iter().all(|d| d.path.is_some())).await;
    home.publish_now().await;
    let now = home.roster().unwrap();
    assert_eq!((now.body.epoch, now.body.devices.len()), (1, 3));
    for m in [&tower, &laptop] {
        for _ in 0..50 {
            if m.pull_now().await.is_ok() && seq_of(m) == now.body.seq {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(seq_of(m), now.body.seq);
    }
    // (the list every computer holds says where the tower is: that is how the laptop finds the home computer
    // again once it has moved there, in a lab with no relay)
    assert!(!now.device(&tower.status().unwrap().id).unwrap().addrs.is_empty());
    let (id, tower_id) = (home.status().unwrap().id, tower.status().unwrap().id);
    Three { hd, td, home, home_parts, tower, tower_parts, laptop, id, tower_id, hub, server, binds }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_home_computer_moves_to_another_computer_and_back_and_the_others_follow() {
    let t = three().await;
    std::fs::write(t.hd.join("backup.key"), "what seals this home computer's backups").unwrap();
    let (status, v) = ask_long(&t.home, "/home/move", json!({"to": t.tower_id.to_string()})).await;
    assert_eq!((status, v["state"].as_str(), v["retired"].as_bool()), (200, Some("done"), Some(false)), "{v}");
    assert!(!t.hd.join("backup.key").exists(), "what seals the backups does not stay on the old computer");
    assert_eq!(v["home"]["id"], t.id.to_string(), "the home computer is who it was");

    // the computer that was home: stopped its server, holds a new key of its own, and has a home now
    assert_eq!(*t.home_parts.said.lock().unwrap(), ["stop"]);
    let new_id = key_id(&t.hd).unwrap();
    assert_ne!(new_id, t.id, "the home's key is not on the old computer any more");
    assert!(!t.hd.join("device.key.next").exists() && !t.hd.join("home-move.json").exists());
    assert_eq!(note_why(&t.hd).as_deref(), Some("moved"));
    let cfg = crate::load_raw(&t.hd.join("node.json")).unwrap();
    assert_eq!((cfg.home.as_ref().map(|h| h.id), cfg.hub.as_str(), cfg.machine.as_str()), (Some(t.id), "", "homebox"));

    // the computer that took over: the key, the state, and its server started
    assert_eq!(key_id(&t.td), Some(t.id));
    assert_eq!(std::fs::read_to_string(t.td.join("data.txt")).unwrap(), "everything the home computer knows");
    assert!(!t.td.join("device.key.home").exists() && !t.td.join("home-arrive.json").exists());
    looking("the tower answers as home", &[&t.tower], || is_home(&t.tower)).await;
    until("its server was started", || *t.tower_parts.said.lock().unwrap() == ["start"]).await;
    let cfg = crate::load_raw(&t.td.join("node.json")).unwrap();
    assert!(cfg.home.is_none() && cfg.hub == t.hub.to_string());

    // the list: a higher epoch, the old home computer on it under its machine name, the tower's old key gone
    until("the list is signed with a higher epoch", || epoch_of(&t.tower) == 2).await;
    let list = t.tower.roster().unwrap();
    assert!(list.device(&t.tower_id).is_none(), "the key the tower had is on no list");
    assert_eq!(list.device(&new_id).and_then(|d| d.machine.clone()).as_deref(), Some("homebox"));
    assert_eq!(list.body.devices.len(), 3);

    // the laptop was told nothing: it finds the home computer where the tower was, and takes the new list
    looking("the laptop follows", &[&t.laptop, &t.home], || reaches_home(&t.laptop) && epoch_of(&t.laptop) == 2).await;
    looking("the old home computer is one of its computers", &[&t.home], || reaches_home(&t.home)).await;
    assert_eq!(t.home.status().unwrap().role, "member");
    until("home sees both", || t.tower.devices().unwrap().iter().all(|d| d.path.is_some())).await;

    // and back again
    t.tower.publish_now().await;
    let (status, v) = ask_long(&t.tower, "/home/move", json!({"to": new_id.to_string()})).await;
    assert_eq!((status, v["state"].as_str()), (200, Some("done")), "{v}");
    assert_eq!(key_id(&t.hd), Some(t.id));
    assert_eq!(note_why(&t.hd), None, "it is the home computer again, so the note is gone");
    assert_eq!(note_why(&t.td).as_deref(), Some("moved"));
    looking("the first computer answers as home again", &[&t.home], || is_home(&t.home) && epoch_of(&t.home) == 3)
        .await;
    looking("the others follow again", &[&t.laptop, &t.tower], || {
        reaches_home(&t.laptop) && reaches_home(&t.tower) && epoch_of(&t.laptop) == 3
    })
    .await;
    assert_eq!(std::fs::read_to_string(t.hd.join("data.txt")).unwrap(), "everything the home computer knows");
    for k in [&t.home, &t.tower, &t.laptop] {
        k.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_move_that_stops_after_stepping_down_leaves_nobody_home_and_resume_finishes_it() {
    let t = three().await;
    *t.home.kit.stop_at.lock().unwrap() = Some("stepped_down");
    let (status, v) = ask_long(&t.home, "/home/move", json!({"to": t.tower_id.to_string()})).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("stopped")), "{v}");

    // nobody is home: the old one says so and holds the key; the other holds the state, staged, and no key
    assert_eq!(note_why(&t.hd).as_deref(), Some("moving"));
    assert_eq!(key_id(&t.hd), Some(t.id));
    let (status, v) = control(&t.home, "GET", "/status", None).await;
    assert_eq!((status, v["kind"].as_str()), (503, Some("stood_down")), "{v}");
    assert!(v["error"].as_str().unwrap().contains("home move --resume"), "{v}");
    assert_eq!(t.home.role(), Role::Off);
    assert_eq!(key_id(&t.td), Some(t.tower_id));
    assert!(t.td.join(".home-staging/note").exists() && !t.td.join("data.txt").exists());
    assert_eq!(t.tower.status().unwrap().role, "member");
    assert!(t.tower_parts.said.lock().unwrap().is_empty());

    // the helper starts again (a power cut): still nobody, pairing is held back, and no second move starts
    t.home.shutdown().await;
    let (home, _parts) = able(&t.hd, t.hub, t.server, t.binds.0).await;
    home.tick().await;
    home.tick().await;
    assert!(home.status().is_err() && home.role() == Role::Off);
    let (status, v) = control(&home, "POST", "/pair/new", Some(json!({}))).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("moving")), "{v}");
    let (status, v) = ask_long(&home, "/home/move", json!({"to": t.tower_id.to_string()})).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("unfinished")), "{v}");
    let (_, v) = control(&home, "GET", "/home/status", None).await;
    assert_eq!((v["move"]["state"].as_str(), v["role"].as_str()), (Some("stepped_down"), Some("off")), "{v}");

    // --resume finishes it
    let (status, v) = ask_long(&home, "/home/move", json!({"resume": true})).await;
    assert_eq!((status, v["state"].as_str()), (200, Some("done")), "{v}");
    assert_eq!(key_id(&t.td), Some(t.id));
    assert_ne!(key_id(&t.hd), Some(t.id));
    looking("the tower answers as home", &[&t.tower], || is_home(&t.tower)).await;
    looking("the old home computer is one of its computers", &[&home], || reaches_home(&home)).await;
    let (status, v) = control(&t.tower, "POST", "/pair/new", Some(json!({}))).await;
    assert_eq!(status, 200, "pairing works again, at the new home computer: {v}");
    for k in [&home, &t.tower, &t.laptop] {
        k.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_move_that_stops_before_stepping_down_is_called_off_and_one_after_it_is_undone() {
    let t = three().await;
    // stopped after packing: nothing has changed anywhere, and --undo says so
    *t.home.kit.stop_at.lock().unwrap() = Some("packed");
    let (status, v) = ask_long(&t.home, "/home/move", json!({"to": t.tower_id.to_string()})).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("stopped")), "{v}");
    assert!(is_home(&t.home) && note_why(&t.hd).is_none());
    let (status, v) = ask_long(&t.home, "/home/move", json!({"undo": true})).await;
    assert_eq!((status, v["state"].as_str()), (200, Some("undone")), "{v}");
    assert!(!t.hd.join("home-move.json").exists());
    assert_eq!(*t.home_parts.said.lock().unwrap(), ["stop", "start"]);

    // stopped after stepping down: --undo puts the home computer back, and the other throws the state away
    *t.home.kit.stop_at.lock().unwrap() = Some("stepped_down");
    let (status, _) = ask_long(&t.home, "/home/move", json!({"to": t.tower_id.to_string()})).await;
    assert_eq!(status, 409);
    assert!(t.td.join(".home-staging/note").exists());
    *t.home.kit.stop_at.lock().unwrap() = None;
    let (status, v) = ask_long(&t.home, "/home/move", json!({"undo": true})).await;
    assert_eq!((status, v["state"].as_str(), v["told"].as_bool()), (200, Some("undone"), Some(true)), "{v}");
    assert!(!t.td.join(".home-staging").exists(), "the tower threw away what it was sent");
    assert_eq!((note_why(&t.hd), key_id(&t.hd)), (None, Some(t.id)));
    assert!(!t.hd.join("device.key.next").exists());
    looking("it answers as home again", &[&t.home, &t.tower, &t.laptop], || {
        is_home(&t.home) && reaches_home(&t.tower) && reaches_home(&t.laptop)
    })
    .await;
    assert_eq!(epoch_of(&t.home), 1, "nothing moved, so the epoch is what it was");
    for k in [&t.home, &t.tower, &t.laptop] {
        k.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn when_the_answer_to_the_commit_is_lost_undo_is_refused_and_resume_finds_out() {
    let t = three().await;
    *t.home.kit.stop_at.lock().unwrap() = Some("committed");
    let (status, v) = ask_long(&t.home, "/home/move", json!({"to": t.tower_id.to_string()})).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("stopped")), "{v}");
    looking("the tower answers as home", &[&t.tower], || is_home(&t.tower)).await;
    assert_eq!((key_id(&t.td), key_id(&t.hd)), (Some(t.id), Some(t.id)), "the old one does not know yet");
    assert!(t.home.status().is_err(), "but it stepped down before it sent the key, so there is one home computer");
    *t.home.kit.stop_at.lock().unwrap() = None;

    // calling it off now would make two home computers: refused, nothing changed
    let (status, v) = ask_long(&t.home, "/home/move", json!({"undo": true})).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("unsure")), "{v}");
    assert_eq!(note_why(&t.hd).as_deref(), Some("moving"));
    t.home.tick().await;
    assert!(t.home.status().is_err());

    // --resume asks who answers as home, and finishes here
    let (status, v) = ask_long(&t.home, "/home/move", json!({"resume": true})).await;
    assert_eq!((status, v["state"].as_str()), (200, Some("done")), "{v}");
    assert_ne!(key_id(&t.hd), Some(t.id));
    looking("the old home computer is one of its computers", &[&t.home], || reaches_home(&t.home)).await;
    let (status, v) = ask_long(&t.home, "/home/move", json!({"undo": true})).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("nothing")), "{v}");
    for k in [&t.home, &t.tower, &t.laptop] {
        k.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_its_home_computer_can_ask_a_computer_to_take_over_and_only_with_the_right_things() {
    let t = three().await;
    let tower = Peer { id: t.tower_id, relay: None, addrs: vec![t.binds.1] };
    let key = hex_of(DeviceKey::generate().secret_bytes());
    let commit =
        json!({"staged": "0".repeat(64), "epoch": 1, "key": key, "old": {"id": null, "name": "x"}}).to_string();

    // another paired computer: refused whatever it asks, and nothing is looked at
    for (method, path, body) in [
        ("GET", "/own/v1/home/status".to_string(), None),
        ("POST", format!("/own/v1/home/receive?sha256={}", "0".repeat(64)), Some(b"{}".as_slice())),
        ("POST", "/own/v1/home/commit".to_string(), Some(commit.as_bytes())),
        ("POST", "/own/v1/home/discard".to_string(), Some(b"{}".as_slice())),
    ] {
        let (status, v) = own_call(&t.laptop, tower.clone(), method, &path, body).await;
        assert_eq!((status, v["kind"].as_str()), (403, Some("not_home")), "{path}: {v}");
    }
    let (_, v) = own_call(&t.laptop, tower.clone(), "GET", "/own/v1/hello", None).await;
    assert_eq!(v["can_host"], true, "{v}");

    // the home computer itself: a file that is not what it says is not staged, a key that is not the home's is
    // not taken, and a commit of something that is not staged changes nothing
    let (status, v) = own_call(&t.home, tower.clone(), "GET", "/own/v1/home/status", None).await;
    assert_eq!((status, v["state"].as_str(), v["can_host"].as_bool()), (200, Some("idle"), Some(true)), "{v}");
    let path = format!("/own/v1/home/receive?sha256={}", "0".repeat(64));
    let (status, v) = own_call(&t.home, tower.clone(), "POST", &path, Some(b"{}")).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("damaged")), "{v}");
    let (status, v) = own_call(&t.home, tower.clone(), "POST", "/own/v1/home/commit", Some(commit.as_bytes())).await;
    assert_eq!((status, v["kind"].as_str()), (403, Some("bad_key")), "{v}");
    let mine = hex_of(DeviceKey::load(&t.hd.join("device.key")).unwrap().unwrap().0.secret_bytes());
    let commit = json!({"staged": "0".repeat(64), "epoch": 1, "key": mine, "old": {"id": null}}).to_string();
    let (status, v) = own_call(&t.home, tower.clone(), "POST", "/own/v1/home/commit", Some(commit.as_bytes())).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("nothing")), "{v}");
    assert_eq!(key_id(&t.td), Some(t.tower_id));
    assert!(!t.td.join(".home-staging").exists() && !t.td.join("home-arrive.json").exists());
    assert!(!t.td.join("device.key.home").exists() && !t.td.join("home-incoming/state.tar").exists());
    assert_eq!(t.tower.status().unwrap().role, "member");

    // a computer without the parts a home computer needs is not moved to, and nothing is stopped for it
    let laptop_id = t.laptop.status().unwrap().id;
    let (status, v) = ask_long(&t.home, "/home/move", json!({"to": laptop_id.to_string()})).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("no_parts")), "{v}");
    assert!(v["error"].as_str().unwrap().contains("--standby"), "{v}");
    let (status, v) = ask_long(&t.home, "/home/move", json!({"to": DeviceKey::generate().id().to_string()})).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("not_found")), "{v}");
    assert!(t.home_parts.said.lock().unwrap().is_empty() && is_home(&t.home) && !t.hd.join("home-move.json").exists());
    let (status, v) = ask_long(&t.tower, "/home/move", json!({"to": laptop_id.to_string()})).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("not_home")), "{v}");
    for k in [&t.home, &t.tower, &t.laptop] {
        k.shutdown().await;
    }
}

fn hex_of(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The same list as `k` holds, signed again by the key in `dir` with a higher epoch: what another holder of the
/// home computer's key would sign.
fn a_newer_list(k: &Shared, dir: &Path) -> Envelope {
    let key = DeviceKey::load(&dir.join("device.key")).unwrap().unwrap().0;
    let held = k.roster().unwrap();
    let body = Body { epoch: held.body.epoch + 1, at: held.body.at + 1, ..held.body.clone() };
    roster::sign(&key, body).unwrap().envelope
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_home_computer_that_was_replaced_stands_down_when_a_computer_shows_it_the_newer_list() {
    let t = three().await;
    // somewhere else, the home computer's key signed a list with a higher epoch, and the laptop holds it
    let newer = a_newer_list(&t.home, &t.hd);
    t.home.shutdown().await;
    assert_eq!(t.laptop.node_roster(&json!({"roster": newer}))["took"], true);
    assert_eq!(epoch_of(&t.laptop), 2);

    // the old home computer starts again, as a disk put back from a backup would
    let (home, parts) = able(&t.hd, t.hub, t.server, t.binds.0).await;
    home.tick().await;
    // (it is looked at too: a port just let go of is not always there to take again at once, on Windows)
    looking("it stands down", &[&home, &t.laptop], || note_why(&t.hd).is_some()).await;
    assert_eq!(note_why(&t.hd).as_deref(), Some("superseded"));
    home.tick().await;
    assert!(home.status().is_err() && home.role() == Role::Off, "keys are on in its settings, and it is not home");
    let (status, v) = control(&home, "GET", "/status", None).await;
    assert_eq!((status, v["kind"].as_str()), (503, Some("stood_down")), "{v}");
    assert!(v["error"].as_str().unwrap().contains("Another computer answers as the home computer now"), "{v}");
    until("its server was stopped", || *parts.said.lock().unwrap() == ["stop"]).await;
    assert_eq!(key_id(&t.hd), Some(t.id));
    // it never pairs with the key it held as home
    let (status, v) = control(&home, "POST", "/pair/join", Some(json!({"input": "sb1:nonsense"}))).await;
    assert_eq!(status, 400, "{v}");
    for k in [&home, &t.tower, &t.laptop] {
        k.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_home_computer_that_knows_none_of_the_devices_stands_down_when_one_it_refuses_shows_it_the_newer_list() {
    let t = three().await;
    let newer = a_newer_list(&t.home, &t.hd);
    t.home.shutdown().await;
    t.tower.shutdown().await;
    assert_eq!(t.laptop.node_roster(&json!({"roster": newer}))["took"], true);

    // the old home computer starts again knowing no device, as one brought back from a backup made before
    // every one of them was paired would: it refuses the laptop, and has nobody to ask
    let paired = Paired::load(&t.hd.join("paired.json"));
    for d in paired.list() {
        paired.remove(&d.id).unwrap();
    }
    assert!(Paired::load(&t.hd.join("paired.json")).list().is_empty());
    let (home, parts) = able(&t.hd, t.hub, t.server, t.binds.0).await;
    home.tick().await;
    // (it is looked at too: a port just let go of is not always there to take again at once, on Windows)
    looking("it stands down", &[&home, &t.laptop], || note_why(&t.hd).is_some()).await;
    assert_eq!(note_why(&t.hd).as_deref(), Some("superseded"));
    until("its server was stopped", || *parts.said.lock().unwrap() == ["stop"]).await;
    for k in [&home, &t.laptop] {
        k.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_backup_opens_with_its_code_only_and_the_restored_home_takes_removals_made_after_it() {
    let t = three().await;
    // the first backup makes the recovery code, and says it once
    let (status, v) = ask_long(&t.home, "/home/backup", json!({})).await;
    assert_eq!(status, 200, "{v}");
    let code = v["code"].as_str().unwrap().to_string();
    assert_eq!((code.len(), code.matches('-').count()), (31, 5), "five groups of five and one more");
    let file = PathBuf::from(v["file"].as_str().unwrap());
    assert!(file.starts_with(t.hd.join("home-backup")) && file.extension().is_some_and(|e| e == "sbk"));
    let sealed = std::fs::read(&file).unwrap();
    assert!(sealed.starts_with(b"SBHOME1\n"));
    let plain = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).any(|w| w == needle);
    assert!(!plain(&sealed, b"everything the home computer knows"), "the state is sealed");
    let key = DeviceKey::load(&t.hd.join("device.key")).unwrap().unwrap().0;
    assert!(!plain(&sealed, key.secret_bytes()), "and so is the key");
    // the code is in no file on the home computer
    let bare = code.replace('-', "");
    for entry in std::fs::read_dir(&t.hd).unwrap().flatten().filter(|e| e.path().is_file()) {
        // (a lock file held open cannot be read on Windows, and is empty)
        let Ok(bytes) = std::fs::read(entry.path()) else { continue };
        assert!(!plain(&bytes, code.as_bytes()) && !plain(&bytes, bare.as_bytes()), "{:?}", entry.path());
    }
    let (status, v) = ask_long(&t.home, "/home/backup", json!({"to": t.hd.join("second.sbk")})).await;
    assert_eq!((status, v["code"].is_null(), t.hd.join("second.sbk").exists()), (200, true, true), "{v}");
    let (status, v) = ask_long(&t.tower, "/home/backup", json!({})).await;
    assert_eq!((status, v["kind"].as_str()), (409, Some("not_home")), "{v}");

    let at_backup = t.home.roster().unwrap().envelope.clone();
    // after the backup the tower is removed, and the laptop hears of it
    let (status, _) = control(&t.home, "POST", "/devices/remove", Some(json!({"id": t.tower_id.to_string()}))).await;
    assert_eq!(status, 200);
    let before = t.home.roster().unwrap().body.seq;
    until("the list is signed without it", || t.home.roster().is_some_and(|r| r.device(&t.tower_id).is_none())).await;
    let after = t.home.roster().unwrap().body.seq;
    assert!(after > before || t.home.roster().unwrap().device(&t.tower_id).is_none());
    for _ in 0..50 {
        if t.laptop.pull_now().await.is_ok() && seq_of(&t.laptop) == after {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(seq_of(&t.laptop), after);
    let after_removal = t.laptop.roster().unwrap();
    std::fs::write(t.hd.join("data.txt"), "written after the backup").unwrap();

    // the home computer is gone for good
    t.home.shutdown().await;
    t.tower.shutdown().await;

    // a new computer: the wrong code, a damaged file and a cut file change nothing there
    let nd = scratch("new");
    let (new, new_parts) = able(&nd, t.hub, t.server, t.binds.0).await;
    new.tick().await;
    let untouched =
        || !nd.join("device.key").exists() && !nd.join("data.txt").exists() && !nd.join(".home-staging").exists();
    let wrong = "AAAAA-AAAAA-AAAAA-AAAAA-AAAAA-A";
    let e = new.home_restore(&file, wrong).await.unwrap_err();
    assert_eq!(e.kind, "wrong_code", "{e}");
    let e = new.home_restore(&file, "not a code").await.unwrap_err();
    assert_eq!(e.kind, "bad_code", "{e}");
    let mut bent = sealed.clone();
    let at = bent.len() - 40;
    bent[at] ^= 1;
    std::fs::write(nd.join("bent.sbk"), &bent).unwrap();
    let e = new.home_restore(&nd.join("bent.sbk"), &code).await.unwrap_err();
    // (a changed byte and a wrong code look the same to the seal: neither opens it, and the sentence says both)
    assert!(matches!(e.kind, "damaged" | "wrong_code"), "{e}");
    std::fs::write(nd.join("cut.sbk"), &sealed[..sealed.len() - 10]).unwrap();
    let e = new.home_restore(&nd.join("cut.sbk"), &code).await.unwrap_err();
    // (a changed byte and a wrong code look the same to the seal: neither opens it, and the sentence says both)
    assert!(matches!(e.kind, "damaged" | "wrong_code"), "{e}");
    assert!(untouched());
    assert!(new_parts.said.lock().unwrap().is_empty());

    // the right code: this computer is the home computer as it was when the backup was made
    let (status, v) = ask_long(&new, "/home/restore", json!({"file": file, "code": code.to_lowercase()})).await;
    assert_eq!(
        (status, v["home"].as_str(), v["epoch"].as_u64()),
        (200, Some(t.id.to_string().as_str()), Some(2)),
        "{v}"
    );
    assert_eq!(v["devices"].as_array().unwrap().len(), 2, "as the backup knew them: {v}");
    assert_eq!(key_id(&nd), Some(t.id));
    assert_eq!(std::fs::read_to_string(nd.join("data.txt")).unwrap(), "everything the home computer knows");
    assert_eq!(*new_parts.said.lock().unwrap(), ["start"]);
    assert!(!nd.join("home-incoming/restore.tar").exists() && !nd.join("device.key.home").exists());
    looking("it answers as home", &[&new], || is_home(&new) && epoch_of(&new) == 2).await;
    let e = new.home_restore(&file, &code).await.unwrap_err();
    assert_eq!(e.kind, "is_home", "{e}");

    // the laptop reaches it and shows the list it holds: the tower, removed after the backup, is removed again
    looking("the removal is taken from the laptop", &[&new, &t.laptop], || {
        reaches_home(&t.laptop) && new.devices().is_ok_and(|d| d.iter().all(|d| d.id != t.tower_id))
    })
    .await;
    new.publish_now().await;
    until("the new list is without it", || new.roster().is_some_and(|r| r.device(&t.tower_id).is_none())).await;
    assert_eq!(epoch_of(&new), 2);
    looking("the laptop takes the new home's list", &[&t.laptop], || epoch_of(&t.laptop) == 2).await;
    // A device the restored home refused would have shown that list outside the tunnel. It is answered with
    // the list signed here since, to carry to the home computer it came from; whoever shows any other list,
    // a device that was removed among them, is told nothing.
    let shown = |e: &Envelope| new.home_shown(&serde_json::to_vec(e).unwrap());
    let answer: Envelope = serde_json::from_slice(&shown(&after_removal.envelope).unwrap()).unwrap();
    assert!(roster::verify(&answer, &t.id, None).unwrap().stamp() > after_removal.stamp());
    assert!(shown(&at_backup).is_none() && new.home_shown(b"not a list").is_none());
    assert!(is_home(&new));

    // the old home computer is started again after all: it asks the computers on its list, hears of the newer
    // list from the laptop, and stands down
    let (old, old_parts) = able(&t.hd, t.hub, t.server, free_port().await).await;
    old.tick().await;
    looking("the old home computer stands down", &[&t.laptop], || note_why(&t.hd).is_some()).await;
    assert_eq!(note_why(&t.hd).as_deref(), Some("superseded"));
    old.tick().await;
    assert!(old.status().is_err());
    until("its server was stopped", || *old_parts.said.lock().unwrap() == ["stop"]).await;
    assert!(is_home(&new), "and the new one goes on");
    for k in [&old, &new, &t.laptop] {
        k.shutdown().await;
    }
}

#[test]
fn a_note_that_cannot_be_read_still_means_this_computer_is_not_home() {
    let d = scratch("note");
    assert!(home::stood_down(&d).is_none());
    std::fs::write(d.join("home-stepdown.json"), b"{ not json").unwrap();
    assert_eq!(home::stood_down(&d).unwrap().why, "unreadable");
    std::fs::write(d.join("home-stepdown.json"), br#"{"v":1,"why":"moved","at":5}"#).unwrap();
    let note = home::stood_down(&d).unwrap();
    assert_eq!((note.why.as_str(), note.at), ("moved", 5));
    assert!(home::stood_down_line(&note).starts_with(home::STOOD_DOWN));
}
