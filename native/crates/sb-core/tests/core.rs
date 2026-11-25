//! sb-core against fixtures shaped like the real API, the snapshot cache, and the LAN/Tailscale fallback over real
//! sockets (local fake servers, never the live one).

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use sb_core::actions::{self, ActionId};
use sb_core::api::{ApiFolders, ApiSessions};
use sb_core::board::{self, Column};
use sb_core::cache::{self, FolderCache};
use sb_core::model::{self, Lamp, Tone};
use sb_core::names::TagKind;
use sb_core::{Core, CoreConfig, CoreError, NewRequest};

const NOW: f64 = 1791782309.5;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn hand() -> model::Snapshot {
    let raw: ApiSessions = serde_json::from_slice(&fixture("api_sessions.json")).unwrap();
    model::normalize(&raw, NOW)
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("sb-core-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn hand_fixture_normalizes() {
    let s = hand();
    assert_eq!(s.machines.len(), 3);
    assert_eq!(s.machine("laptop").unwrap().replaced.len(), 0, "a malformed replaced list is dropped, not fatal");
    assert!(s.machine("tower").unwrap().is_offline());
    assert_eq!(s.machine("studio").unwrap().root_label, "Claude");
    assert_eq!(s.machine("laptop").unwrap().root_label, "Home folder");

    let lamp = |k: &str| s.session(k).unwrap().lamp(NOW);
    assert_eq!(lamp("studio:11111111-1111-4111-8111-111111111111"), Lamp::Held);
    assert_eq!(lamp("studio:22222222-2222-4222-8222-222222222222"), Lamp::Wait);
    assert_eq!(lamp("studio:33333333-3333-4333-8333-333333333333"), Lamp::Run);
    assert_eq!(lamp("studio:44444444-4444-4444-8444-444444444444"), Lamp::Warm);
    assert_eq!(lamp("studio:55555555-5555-4555-8555-555555555555"), Lamp::Cold);
    assert_eq!(lamp("studio:66666666-6666-4666-8666-666666666666"), Lamp::Idle);
    assert_eq!(lamp("studio:88888888-8888-4888-8888-888888888888"), Lamp::Stall);
    assert!(s.session("studio:77777777-7777-4777-8777-777777777777").unwrap().stale);

    let held = s.session("studio:11111111-1111-4111-8111-111111111111").unwrap();
    assert_eq!(held.title, "Garden planner (fresh)");
    assert_eq!(held.topic, "Garden planner");
    assert_eq!(held.tag.as_deref(), Some("fresh"));
    assert_eq!(held.tag_kind, Some(TagKind::Fresh));
    assert_eq!(held.ctx_label(), "210k");
    assert_eq!(held.model, "Opus 5.5");

    let run = s.session("studio:33333333-3333-4333-8333-333333333333").unwrap();
    assert_eq!(run.project, "switchboard");
    assert_eq!(run.tag_kind, Some(TagKind::Ultra));
    let ids: Vec<String> = run.chips(true).into_iter().map(|c| c.id).collect();
    assert_eq!(ids, vec!["chrome", "bg"], "the UltraCode chip is skipped when the tag already says it");
    assert_eq!(run.repeats, 2);

    let warm = s.session("studio:44444444-4444-4444-8444-444444444444").unwrap();
    assert_eq!(warm.project, "labv2");
    let chips = warm.chips(false);
    assert_eq!(chips.iter().map(|c| c.short.as_str()).collect::<Vec<_>>(), vec!["Sonnet", "Plan"]);
    assert_eq!(chips[0].long, "Sonnet 4.5");
    assert_eq!(warm.phrase(NOW), "Warm, 46m left");

    let cold = s.session("studio:55555555-5555-4555-8555-555555555555").unwrap();
    assert_eq!(cold.phrase(NOW), "Cold, handoff ready");
    assert_eq!(cold.tag_kind, Some(TagKind::Move));
    assert!(cold.recommend_fresh(NOW));

    let notes = s.session("studio:66666666-6666-4666-8666-666666666666").unwrap();
    assert_eq!(notes.folder, "Home folder");
    assert_eq!(notes.chips(false)[0].long, "Skips permissions");

    let farm = s.session("tower:dddddddd-dddd-4ddd-8ddd-dddddddddddd").unwrap();
    assert_eq!(farm.folder, "Claude/render");

    assert_eq!(s.plan.week_tone(), Tone::Ok);
    assert_eq!(s.plan.five_tone(), Tone::Warn);
    // The status note is the server's (notes.py); the app writes no advice of its own.
    assert_eq!(s.plan.note_text(), Some("Garden planner is waiting for you on Studio."));
    assert_eq!(
        s.plan.note.as_ref().and_then(|n| n.key()).as_deref(),
        Some("studio:11111111-1111-4111-8111-111111111111")
    );
}

#[test]
fn board_columns_and_order() {
    let s = hand();
    let b = board::build(&s, NOW, &HashSet::new(), None);
    assert_eq!(b.column_counts, vec![2, 2, 3, 2, 3], "Quiet takes an older chat too (QUIET_KEEP)");
    assert_eq!(b.cold_ctx, 570_000);
    let studio = b.lane("studio").unwrap();
    assert_eq!(studio.tally.n, 7);
    assert_eq!(studio.tally.need, 2);
    assert_eq!(studio.tally.stake, 250_000);
    let mb = b.lane("laptop").unwrap();
    assert_eq!(
        mb.cells[Column::Warm.index()],
        vec!["laptop:bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "laptop:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"],
        "warm is sorted by the time left, shortest first"
    );
    // a hidden (dismissed) chat leaves the board at once
    let hidden: HashSet<String> = ["studio:22222222-2222-4222-8222-222222222222".to_string()].into();
    let b2 = board::build(&s, NOW, &hidden, None);
    assert_eq!(b2.column_counts[0], 1);
    // a torn-off lane shows one machine
    let solo = board::build(&s, NOW, &HashSet::new(), Some("laptop"));
    assert_eq!(solo.lanes.len(), 1);

    let order = b.order(|_| 1, &HashSet::new(), &["tower".to_string()].into());
    assert_eq!(order.len(), 5 + 2, "one per non-empty cell, folded lane skipped");
    let first = order[0].clone();
    assert_eq!(b.locate(&first).unwrap().1, Column::Need);
    let right = b.across(Some(&first), 1).unwrap();
    assert_eq!(b.locate(&right).unwrap().1, Column::Run);
    assert_eq!(b.across(Some(&first), -1), None);
}

#[test]
fn quiet_keeps_the_newest_chats_however_old() {
    // Days-old chats (stale) stay in Quiet until QUIET_KEEP newer ones push them to History.
    let mut s = hand();
    let m = s.machines.iter_mut().find(|m| m.name == "studio").unwrap();
    let base = m.sessions[0].clone();
    for i in 0..12 {
        let mut o = base.clone();
        o.key = format!("studio:old-{i}");
        o.sid = format!("old-{i}");
        o.stale = true;
        o.active = NOW - 3. * 86400. - i as f64 * 60.;
        m.sessions.push(o);
    }
    let b = board::build(&s, NOW, &HashSet::new(), None);
    let q = &b.lane("studio").unwrap().cells[Column::Quiet.index()];
    assert_eq!(q.len(), board::QUIET_KEEP, "filled up to QUIET_KEEP with the newest old chats");
    assert!(q.contains(&"studio:old-0".to_string()), "the newest old chat is there");
    assert!(!q.contains(&"studio:old-11".to_string()), "the oldest is in History");
    let hidden: HashSet<String> = ["studio:old-0".to_string()].into();
    let b2 = board::build(&s, NOW, &hidden, None);
    assert!(!b2.lane("studio").unwrap().cells[Column::Quiet.index()].contains(&"studio:old-0".to_string()));
}

#[test]
fn actions_match_the_web_app() {
    let s = hand();
    let held = s.session("studio:11111111-1111-4111-8111-111111111111").unwrap();
    let a = actions::for_session(held, &s, NOW);
    let ids: Vec<ActionId> = a.iter().map(|x| x.id).collect();
    assert_eq!(
        ids,
        vec![
            ActionId::Fresh,
            ActionId::Open,
            ActionId::Terminal,
            ActionId::Pass,
            ActionId::Resume,
            ActionId::Continue,
            ActionId::Handoff,
            ActionId::Dismiss,
            ActionId::Rename,
            ActionId::Link
        ]
    );
    assert!(a[0].primary);
    let term = a.iter().find(|x| x.id == ActionId::Terminal).unwrap();
    assert!(term.disabled, "an interactive chat cannot be attached");
    assert_eq!(term.note, "It's open in a terminal on Studio. Only background chats open here.");
    assert_eq!(a.iter().find(|x| x.id == ActionId::Open).unwrap().key.as_deref(), Some("Enter"));
    assert!(a.iter().find(|x| x.id == ActionId::Resume).unwrap().disabled, "already open in a terminal");
    assert_eq!(
        a.iter().find(|x| x.id == ActionId::Pass).unwrap().note,
        "Re-reads 210k once, then the guard leaves it alone"
    );
    assert_eq!(a.iter().find(|x| x.id == ActionId::Dismiss).unwrap().note, "Hides it here. The terminal stays open.");

    let c = actions::fresh_confirm(held);
    assert_eq!(c.title, "Start Garden planner fresh?");
    assert_eq!(
        c.body,
        "Writes a summary on Studio and opens a new chat with your notes and the held message, so the next message doesn't re-read 210k. The old chat moves to Replaced."
    );

    let waiting = s.session("studio:22222222-2222-4222-8222-222222222222").unwrap();
    let a = actions::for_session(waiting, &s, NOW);
    assert_eq!(a[0].id, ActionId::Terminal, "a running background chat opens in a terminal tab");
    assert!(a[0].primary && !a[0].disabled);
    assert_eq!(a[0].key.as_deref(), Some("Enter"));
    let attach = a.iter().find(|x| x.id == ActionId::Attach).unwrap();
    assert!(!attach.primary);
    assert_eq!(attach.note, "claude attach abcd1234, then run it on Studio to answer");
    assert!(!a.iter().any(|x| x.id == ActionId::ResumeOpen), "it is running");
    assert!(a.iter().any(|x| x.id == ActionId::Stop && x.danger));
    assert_eq!(a.iter().find(|x| x.id == ActionId::Handoff).unwrap().title, "Hand off and stop");

    // Running in the background with a Claude link: the terminal takes Enter, Open in Claude stays as a secondary.
    let linked = s.session("studio:33333333-3333-4333-8333-333333333333").unwrap();
    let a = actions::for_session(linked, &s, NOW);
    assert_eq!(a[0].id, ActionId::Terminal);
    let open = a.iter().find(|x| x.id == ActionId::Open).unwrap();
    assert!(!open.primary && open.key.is_none() && open.href.is_some());

    // Stopped: Resume and open is the primary, Resume as is stays.
    let stopped = s.session("studio:66666666-6666-4666-8666-666666666666").unwrap();
    let a = actions::for_session(stopped, &s, NOW);
    let ro = a.iter().find(|x| x.id == ActionId::ResumeOpen).unwrap();
    assert_eq!(ro.title, "Resume and open");
    assert_eq!(ro.primary, !stopped.recommend_fresh(NOW));
    assert!(a.iter().any(|x| x.id == ActionId::Resume && !x.disabled));
    assert!(!a.iter().any(|x| x.id == ActionId::Terminal));
    assert!(!a.iter().any(|x| x.id == ActionId::Stop), "nothing to stop");
    assert_eq!(a.iter().find(|x| x.id == ActionId::Handoff).unwrap().title, "Write a handoff");

    let r = &s.machine("studio").unwrap().replaced[0];
    assert_eq!(r.how_text(|m| s.label_of(m)), "Started fresh");
    let ra = actions::for_replaced(r, &s);
    assert_eq!(
        ra.iter().map(|x| x.id).collect::<Vec<_>>(),
        vec![ActionId::Newer, ActionId::ArchiveInClaude, ActionId::Resume, ActionId::Dismiss]
    );
    assert_eq!(ra[0].note, "Garden planner (fresh)");

    assert!(actions::valid_folder_name("new thing_2.0"));
    assert!(!actions::valid_folder_name("a/b"));
    assert!(!actions::valid_folder_name(""));
    assert!(
        actions::ultra_warn(&s.plan, NOW)
            .unwrap()
            .starts_with("The 5-hour window is at 62%. Workflows stop if it fills mid-run. It resets at ")
    );
}

#[test]
fn design_lab_sample_parses() {
    let raw: ApiSessions = serde_json::from_slice(&fixture("sample.json")).unwrap();
    let now = raw.now.unwrap();
    let s = model::normalize(&raw, now);
    assert_eq!(s.machines.len(), 3);
    assert_eq!(s.sessions().len(), 76);
    let b = board::build(&s, now, &HashSet::new(), None);
    let total: u32 = b.column_counts.iter().sum();
    assert_eq!(total, 76 - 52 + 12, "older chats only fill Quiet up to QUIET_KEEP per computer");
    // list-form flags still produce chips
    let first = s.session("studio:c03ef0d9-fb19-40d7-ba39-eca0294f5d87").unwrap();
    assert_eq!(first.flags.effort.as_deref(), Some("ultracode"));
    assert_eq!(first.lamp(now), Lamp::Run);
}

#[test]
fn snapshot_cache_round_trip() {
    let dir = temp_dir("cache");
    let raw = fixture("api_sessions.json");
    cache::save_sessions(&dir, &raw, NOW, "http://127.0.0.1:1").unwrap();
    let (data, at, base) = cache::load_sessions(&dir, NOW + 60.0).unwrap();
    assert_eq!(at, NOW);
    assert_eq!(base.as_deref(), Some("http://127.0.0.1:1"));
    assert_eq!(data.machines.len(), 3);
    assert!(cache::load_sessions(&dir, NOW + cache::MAX_AGE + 1.0).is_none(), "a day-old snapshot is not shown");
    assert_eq!(
        cache::last_server(&dir).as_deref(),
        Some("http://127.0.0.1:1"),
        "but where it came from is still worth a try"
    );
    // a torn write never replaces a good file
    std::fs::write(dir.join("sessions.json.tmp-1"), b"{\"half").unwrap();
    assert!(cache::load_sessions(&dir, NOW).is_some());
    // garbage in the file is ignored, not fatal
    std::fs::write(dir.join("sessions.json"), b"not json").unwrap();
    assert!(cache::load_sessions(&dir, NOW).is_none());

    let mut fc = FolderCache::default();
    let flat: ApiFolders = serde_json::from_str(
        r#"{"ok":true,"root":"/Users/you/Desktop/Claude","folders":[{"folder":"a","recent":5},{"folder":"b","recent":9},{"folder":"c","recent":0},{"folder":"d","recent":7},{"folder":"e","recent":8}]}"#,
    )
    .unwrap();
    fc.take("studio", None, &flat, NOW);
    let tree: ApiFolders = serde_json::from_str(r#"{"ok":true,"root":"/Users/you/Desktop/Claude","parent":"","folders":[{"folder":"a","name":"a","kids":true}]}"#).unwrap();
    fc.take("studio", Some(""), &tree, NOW);
    fc.save(&dir).unwrap();
    let back = FolderCache::load(&dir);
    let m = back.machine("studio").unwrap();
    assert_eq!(m.recent.iter().map(|f| f.folder.as_str()).collect::<Vec<_>>(), vec!["b", "e", "d"]);
    assert_eq!(back.level("studio", "").unwrap()[0].kids, Some(true));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- fake servers over real sockets ----------

/// One request the fake server saw: (method, path, headers, body).
type Request = (String, String, Vec<(String, String)>, String);

#[derive(Default)]
struct Seen {
    requests: Vec<Request>,
}

/// A tiny HTTP server: `reply(method, path, body)` returns (status, json).
fn serve(reply: impl Fn(&str, &str, &str) -> (u16, String) + Send + Sync + 'static) -> (String, Arc<Mutex<Seen>>) {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Seen::default()));
    let s2 = seen.clone();
    let reply = Arc::new(reply);
    thread::spawn(move || {
        for stream in l.incoming().flatten() {
            let seen = s2.clone();
            let reply = reply.clone();
            thread::spawn(move || handle(stream, &*reply, &seen));
        }
    });
    (base, seen)
}

fn handle(stream: TcpStream, reply: &dyn Fn(&str, &str, &str) -> (u16, String), seen: &Mutex<Seen>) {
    let mut r = BufReader::new(stream.try_clone().unwrap());
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        let mut headers = Vec::new();
        let mut len = 0usize;
        loop {
            let mut h = String::new();
            r.read_line(&mut h).unwrap();
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                if k.eq_ignore_ascii_case("content-length") {
                    len = v.trim().parse().unwrap_or(0);
                }
                headers.push((k.to_ascii_lowercase(), v.trim().to_string()));
            }
        }
        let mut body = vec![0; len];
        r.read_exact(&mut body).unwrap();
        let body = String::from_utf8_lossy(&body).to_string();
        let (status, out) = reply(&method, &path, &body);
        seen.lock().unwrap().requests.push((method, path, headers, body));
        let mut w = stream.try_clone().unwrap();
        let _ = write!(
            w,
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{out}",
            out.len()
        );
        let _ = w.flush();
    }
}

/// Accepts connections and never answers, like a LAN address seen from another network.
fn black_hole() -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    thread::spawn(move || {
        let mut keep = Vec::new();
        for s in l.incoming().flatten() {
            keep.push(s);
        }
    });
    base
}

const SUMMARY: &str = r#"{"five_hour":8,"seven_day":59,"sessions":24,"running":0,"cold":9,"online":3,"machines":3}"#;

fn switchboard() -> (String, Arc<Mutex<Seen>>) {
    let sessions = String::from_utf8(fixture("api_sessions.json")).unwrap();
    serve(move |method, path, _body| match (method, path) {
        ("GET", "/api/summary") => (200, SUMMARY.into()),
        ("GET", "/api/sessions") => (200, sessions.clone()),
        ("GET", p) if p.starts_with("/api/folders/studio?parent=") => {
            (200, r#"{"ok":true,"root":"/r","parent":"","folders":[{"folder":"x","name":"x","kids":false}]}"#.into())
        }
        ("GET", "/api/job/server/s-abc") => (200, r#"{"ok":true,"state":"running","step":"handoff"}"#.into()),
        ("GET", p) if p.starts_with("/api/job/") => (404, r#"{"ok":false,"error":"no such job"}"#.into()),
        ("POST", "/api/action") => (200, r#"{"ok":true,"job":"s-abc","machine":"server"}"#.into()),
        ("POST", "/api/new") => (200, r#"{"ok":true,"job":"s-new","machine":"server"}"#.into()),
        _ => (404, r#"{"ok":false,"error":"not found"}"#.into()),
    })
}

#[test]
fn falls_back_to_tailscale_when_the_lan_hangs() {
    let lan = black_hole();
    let (ts, _) = switchboard();
    let dir = temp_dir("fallback");
    let core = Core::new(CoreConfig {
        servers: vec![lan, ts.clone()],
        cache_dir: Some(dir.display().to_string()),
        user_agent: None,
    });
    let t = Instant::now();
    let snap = core.refresh().unwrap();
    assert!(t.elapsed() < Duration::from_millis(2000), "should not wait out the LAN probe: {:?}", t.elapsed());
    assert_eq!(core.server().as_deref(), Some(ts.as_str()));
    assert_eq!(snap.machines.len(), 3);
    // the next start paints from the cache
    let cached = core.cached_snapshot().unwrap();
    assert_eq!(cached.sessions().len(), snap.sessions().len());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prefers_the_lan_and_skips_strangers() {
    let (lan, _) = switchboard();
    let (ts, _) = switchboard();
    let (stranger, _) = serve(|_, _, _| (200, r#"{"hello":"world"}"#.into()));
    let core = Core::new(CoreConfig {
        servers: vec![stranger, lan.clone(), ts],
        cache_dir: Some(temp_dir("prefer").display().to_string()),
        user_agent: None,
    });
    core.refresh().unwrap();
    assert_eq!(core.server().as_deref(), Some(lan.as_str()));
}

#[test]
fn unreachable_when_nothing_answers() {
    let core = Core::new(CoreConfig {
        servers: vec!["http://127.0.0.1:9".into()],
        cache_dir: Some(temp_dir("none").display().to_string()),
        user_agent: None,
    });
    assert_eq!(core.refresh().unwrap_err(), CoreError::Unreachable);
}

#[test]
fn a_typed_address_is_used_at_once() {
    let core = Core::new(CoreConfig {
        servers: vec!["http://127.0.0.1:9".into()],
        cache_dir: Some(temp_dir("typed").display().to_string()),
        user_agent: None,
    });
    assert_eq!(core.refresh().unwrap_err(), CoreError::Unreachable);
    assert!(!core.needs_server(), "an address is saved, it just does not answer");
    let (base, _) = switchboard();
    core.set_servers(vec![base.clone()]);
    assert_eq!(core.servers(), vec![base.clone()]);
    assert_eq!(core.refresh().unwrap().machines.len(), 3);
    assert_eq!(core.server().as_deref(), Some(base.as_str()));
}

#[test]
fn posts_carry_the_switchboard_headers() {
    let (base, seen) = switchboard();
    let core = Core::new(CoreConfig {
        servers: vec![base],
        cache_dir: Some(temp_dir("posts").display().to_string()),
        user_agent: Some("SwitchboardDesktop/test".into()),
    });
    let job = core.act("studio", "fresh", "11111111-1111-4111-8111-111111111111", None).unwrap();
    assert_eq!(job.as_deref(), Some("s-abc"));
    let j = core.poll_job("s-abc").unwrap();
    assert_eq!(j.state.as_deref(), Some("running"));
    assert!(matches!(core.poll_job("s-gone"), Err(CoreError::Server { status: 404, .. })));
    let job = core
        .start_new(NewRequest {
            machine: "studio".into(),
            folder: "x".into(),
            create: false,
            prompt: "hi".into(),
            model: None,
            mode: "normal".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(job.as_deref(), Some("s-new"));
    let lvl = core.folder_level("studio", "").unwrap();
    assert_eq!(lvl.folders[0].folder, "x");
    assert_eq!(core.cached_level("studio", "").unwrap().len(), 1);

    let seen = seen.lock().unwrap();
    let post = seen.requests.iter().find(|r| r.0 == "POST" && r.1 == "/api/action").unwrap();
    let h = |k: &str| post.2.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(h("x-switchboard").as_deref(), Some("1"));
    assert_eq!(h("content-type").as_deref(), Some("application/json"));
    assert_eq!(h("user-agent").as_deref(), Some("SwitchboardDesktop/test"));
    let body: serde_json::Value = serde_json::from_str(&post.3).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"machine": "studio", "verb": "fresh", "sid": "11111111-1111-4111-8111-111111111111"})
    );
    let new = seen.requests.iter().find(|r| r.1 == "/api/new").unwrap();
    let body: serde_json::Value = serde_json::from_str(&new.3).unwrap();
    assert_eq!(body["model"], serde_json::Value::Null);
    assert_eq!(body["mode"], "normal");
}

#[test]
fn same_board_ignores_fetch_times_only() {
    let raw: ApiSessions = serde_json::from_slice(&fixture("api_sessions.json")).unwrap();
    let a = model::normalize(&raw, NOW);
    // Ten seconds later the server's clock has moved on too; a few milliseconds of network delay is not a change.
    let mut later = raw.clone();
    later.now = later.now.map(|n| n + 10.0);
    let b = model::normalize(&later, NOW + 10.03);
    assert!(a.same_board(&b), "a later poll of the same data draws the same board");
    let mut c = b.clone();
    c.machines[0].sessions[0].title.push('!');
    assert!(!a.same_board(&c), "a changed title is a new board");
    let mut d = b.clone();
    d.machines[1].online = Some(d.machines[1].online == Some(false));
    assert!(!a.same_board(&d), "a machine going on or offline is a new board");
}

#[test]
fn claude_app_links() {
    use sb_core::links::claude_app_link;
    assert_eq!(
        claude_app_link("https://claude.ai/code/session_01Md7EirN5fTPEkccyZwU82A").as_deref(),
        Some("claude://code/session_01Md7EirN5fTPEkccyZwU82A")
    );
    assert_eq!(
        claude_app_link("https://claude.ai/code/session_01Abc?x=1#y").as_deref(),
        Some("claude://code/session_01Abc")
    );
    assert_eq!(claude_app_link("https://claude.ai/code/cse_9z-Q").as_deref(), Some("claude://code/cse_9z-Q"));
    assert_eq!(claude_app_link("https://claude.ai/code/onboarding"), None);
    assert_eq!(claude_app_link("https://claude.ai/code/session_"), None);
    assert_eq!(claude_app_link("https://claude.ai/code/session_a\"b"), None);
    assert_eq!(claude_app_link("https://claude.ai/chat/1234"), None);
    assert_eq!(claude_app_link("http://claude.ai/code/session_01Abc"), None);
    assert_eq!(claude_app_link("https://evil.example/https://claude.ai/code/session_01Abc"), None);
}

/// v2 (spec sections 2 to 5): reaper, Keep running, threads, memory, caps and dispatch tags, as the features server
/// sends them, plus odd shapes that must not sink a refresh.
#[test]
fn v2_fields() {
    let raw: ApiSessions = serde_json::from_slice(&fixture("api_sessions_v2.json")).unwrap();
    let s = model::normalize(&raw, NOW);
    let studio = s.machine("studio").unwrap();
    assert_eq!(
        studio.mem_line().as_deref(),
        Some("Claude is using 3.1 GB on Studio, 410 MB of it in a process nothing is using")
    );
    assert_eq!(studio.mem_short().as_deref(), Some("3.1 GB"));
    assert_eq!(studio.can.as_deref(), Some("iOS + iPhone, Docker"));
    assert!(studio.has_caps);
    let win = s.machine("tower").unwrap();
    assert_eq!(win.mem_line(), None, "under 2 GB says nothing");
    assert!(!win.has_caps && win.can.is_none(), "caps off: null");
    let mb = s.machine("laptop").unwrap();
    assert_eq!(mb.mem, None, "a mem that is not an object is dropped");
    assert_eq!(mb.mem_line().as_deref(), None);
    assert!(s.auto_ready());

    // Stopped by the reaper: the row, the word and the explanation say so; Resume continues it.
    let reaped = s.session("studio:aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa").unwrap();
    assert!(reaped.reaped_note());
    assert_eq!(reaped.word(NOW), "Stopped");
    assert_eq!(reaped.phrase(NOW), "Stopped to free memory");
    assert_eq!(model::plain_text(&reaped.explain(NOW)), "Stopped after 26 h idle to free memory. Resume continues it.");
    assert_eq!(reaped.history, 3);
    let a = actions::for_session(reaped, &s, NOW);
    assert!(!a.iter().any(|x| x.id == ActionId::Keep), "only a running background chat can be kept");
    let unlink = a.iter().find(|x| x.id == ActionId::Unlink).unwrap();
    assert_eq!(unlink.title, "Not the same topic");
    assert_eq!(unlink.note, "Splits it from the 3 earlier chats in its history");
    assert_eq!(a.last().unwrap().id, ActionId::Unlink);

    // A bad reaped value is ignored.
    let odd = s.session("studio:bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb").unwrap();
    assert!(odd.reaped.is_none() && !odd.reaped_note());

    // Kept running: the action says so and turns it off.
    let kept = s.session("studio:cccccccc-cccc-4ccc-8ccc-cccccccccccc").unwrap();
    assert!(kept.keep && kept.can_keep());
    let a = actions::for_session(kept, &s, NOW);
    let keep = a.iter().find(|x| x.id == ActionId::Keep).unwrap();
    assert_eq!(keep.title, "Kept running");
    assert_eq!(keep.note, "Switchboard leaves it running. Click to let Switchboard stop it when idle.");
    let stop_at = a.iter().position(|x| x.id == ActionId::Stop).unwrap();
    assert_eq!(a[stop_at - 1].id, ActionId::Keep, "Keep running sits right above Stop");
    assert!(!a.iter().any(|x| x.id == ActionId::Unlink), "no history, nothing to split");
    assert_eq!(a.iter().find(|x| x.id == ActionId::Link).unwrap().title, "This continues...");

    // Stalled and replaced reasons.
    let stalled = model::Reaped { at: 0.0, reason: "stalled".into(), idle_h: 50.0 };
    assert_eq!(stalled.text(), "Stopped after 2 days stuck on one step, to free memory. Resume continues it.");
    let rep = model::Reaped { at: 0.0, reason: "replaced".into(), idle_h: 1.0 };
    assert_eq!(rep.text(), "Stopped to free memory once a newer chat took over. Resume continues it.");

    // A dispatched chat: "(for Studio)" is its own kind of tag.
    let child = s.session("tower:dddddddd-dddd-4ddd-8ddd-dddddddddddd").unwrap();
    assert_eq!(child.topic, "Build the Android app");
    assert_eq!(child.tag.as_deref(), Some("for Studio"));
    assert_eq!(child.tag_kind, Some(TagKind::For));

    // A replaced chat folded into a thread can be split off; one that was renamed cannot.
    let r = s.replaced_row("studio:eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee").unwrap();
    assert_eq!(r.how_text(|m| s.label_of(m)), "A newer chat picked up the same topic");
    assert!(r.reaped_note());
    let a = actions::for_replaced(r, &s);
    assert_eq!(a.last().unwrap().id, ActionId::Split);
    assert_eq!(a.last().unwrap().note, "Puts it back on the list as a chat of its own");
    assert_eq!(ActionId::Split.verb(), Some("unlink"));

    // The memory line counts as a change on the board.
    let mut raw2 = raw.clone();
    raw2.machines[0].mem.as_mut().unwrap().claude_mb = Some(3500.0);
    assert!(!s.same_board(&model::normalize(&raw2, NOW)));
    let mut later = raw.clone();
    later.now = later.now.map(|n| n + 5.0);
    assert!(s.same_board(&model::normalize(&later, NOW + 5.0)));
}

#[test]
fn v2_thread_chats_and_route_replies() {
    let t: sb_core::api::ApiThread = serde_json::from_str(
        r#"{"ok": true, "thread": "aaaa", "chats": [
            {"machine": "studio", "label": "Studio", "sid": "a1", "title": "Topic", "started": 1, "ended": 2, "ctx": 84000, "how": "same-topic", "url": null, "this": false},
            {"machine": "studio", "sid": "a2", "how": "current", "this": true},
            "junk"]}"#,
    )
    .unwrap();
    assert_eq!(t.chats.len(), 2, "a row that is not an object is dropped");
    assert_eq!(t.chats[1].this, Some(true));
    let r: sb_core::api::ApiRoute =
        serde_json::from_str(r#"{"ok": true, "machine": "laptop", "label": "Laptop", "folder": "", "create": false, "why": "Has Xcode 26.", "by": "rules"}"#).unwrap();
    assert_eq!(r.label.as_deref(), Some("Laptop"));
    assert_eq!(r.by.as_deref(), Some("rules"));
}

#[test]
fn resume_or_fresh_says_which_and_why() {
    use sb_core::actions::{Pick, suggest};
    let s = hand();
    for x in s.sessions() {
        let g = suggest(x, NOW);
        assert!(!g.why.is_empty() && !g.why.contains("UltraCode"), "{}: {}", x.title, g.why);
        match x.lamp(NOW) {
            sb_core::Lamp::Run => assert_eq!(g.pick, None),
            sb_core::Lamp::Warm => assert_eq!(g.pick, Some(Pick::Resume)),
            sb_core::Lamp::Cold if !x.held => assert_eq!(g.pick, Some(Pick::Fresh), "{}", x.title),
            _ => {}
        }
        if x.cache_left(NOW) > 0 {
            assert!(g.resume_cost.starts_with("Pays only"));
        }
    }
}

/// One chat built from the wire shape, so the size, cache and cost can be set per test.
fn one(ctx: f64, cache_left: f64, cost: Option<serde_json::Value>) -> model::Session {
    let mut s = serde_json::json!({"sid": "aaaaaaaa-1111-4111-8111-111111111111", "title": "Bay Area planning",
        "mtime": NOW - 27.0 * 3600.0, "active": NOW - 27.0 * 3600.0, "ctx": ctx, "cache_left": cache_left});
    if let Some(c) = cost {
        s["cost"] = c;
    }
    let raw: ApiSessions =
        serde_json::from_value(serde_json::json!({"machines": [{"name": "studio", "label": "Studio",
        "online": true, "updated": NOW, "sessions": [s]}], "summary": {}}))
        .unwrap();
    model::normalize(&raw, NOW).machines[0].sessions[0].clone()
}

#[test]
fn three_size_bands_with_the_actual_size() {
    use sb_core::model::Band;
    assert_eq!(Band::words(40_000, false), "40k, small, so going back is cheap");
    assert_eq!(
        Band::words(145_000, false),
        "145k, resuming re-reads 145k, so Start fresh is cheaper if you don't need the history"
    );
    assert_eq!(Band::words(200_000, false), "200k, big: resuming re-reads all 200k at full price");
    for k in [40_000, 145_000, 200_000] {
        assert!(Band::words(k, true).ends_with("reply while warm"), "a warm chat is cheap at any size");
    }
    assert_eq!(one(40_000.0, 0.0, None).band_words(NOW), "40k, small, so going back is cheap");
    assert_eq!(one(145_000.0, 600.0, None).band_words(NOW), "145k, reply while warm");
    let warm_medium: String = one(145_000.0, 600.0, None).explain(NOW).iter().map(|r| r.text.as_str()).collect();
    assert!(warm_medium.contains("resuming re-reads all 145k"), "{warm_medium}");
    let cold_small: String = one(40_000.0, 0.0, None).explain(NOW).iter().map(|r| r.text.as_str()).collect();
    assert!(cold_small.contains("going back is cheap"), "{cold_small}");
}

#[test]
fn a_cold_102k_chat_is_not_called_small() {
    use sb_core::actions::{Pick, suggest};
    let g = suggest(&one(102_000.0, 0.0, None), NOW);
    assert_eq!(g.pick, Some(Pick::Fresh));
    assert!(!g.why.contains("small"), "{}", g.why);
    assert!(g.why.contains("re-reads all 102k"), "{}", g.why);
    let g = suggest(&one(40_000.0, 0.0, None), NOW);
    assert_eq!(g.pick, Some(Pick::Resume));
    let g = suggest(&one(102_000.0, 900.0, None), NOW);
    assert_eq!(g.pick, Some(Pick::Resume), "warm: reply while warm");
}

#[test]
fn the_agents_cost_is_shown_for_the_cache_as_it_is_now() {
    use sb_core::actions::{Pick, suggest};
    let cost = serde_json::json!({"ctx": 145000, "band": "medium", "condensed": 55000, "note": 6000,
        "resume": {"warm": 14500, "cold": 290000, "each": 14500},
        "fresh": {"summary": 79375, "first": 12000, "each": 600, "total": 91375},
        "pick": {"warm": "resume", "cold": "fresh"},
        "text": {"warm": "Start fresh: about 91k (summary 79k + new chat 12k). Resume: about 15k, because the cache is warm.",
                 "cold": "Start fresh: about 91k (summary 79k + new chat 12k). Resume: about 290k, because the cache is cold and all 145k is read again."},
        "how": "How this is estimated: ..."});
    let warm = one(145_000.0, 120.0, Some(cost.clone()));
    assert!(warm.cost_line(NOW).unwrap().ends_with("because the cache is warm."));
    assert!(warm.cost_line(NOW + 121.0).unwrap().contains("because the cache is cold"), "flips the second it cools");
    let g = suggest(&warm, NOW + 121.0);
    assert_eq!(g.pick, Some(Pick::Fresh));
    assert!(g.cost.unwrap().starts_with("Start fresh: about 91k"));
    assert_eq!(g.how.as_deref(), Some("How this is estimated: ..."));
    // The agent says resume is cheaper (a chat with a long condensed transcript): the app follows it.
    let mut c2 = cost.clone();
    c2["pick"]["cold"] = "resume".into();
    assert_eq!(suggest(&one(145_000.0, 0.0, Some(c2)), NOW).pick, Some(Pick::Resume));
    // An older agent sends nothing: no numbers are made up.
    assert!(one(145_000.0, 0.0, None).cost_line(NOW).is_none());
    // A broken cost never breaks the board.
    assert!(one(145_000.0, 0.0, Some(serde_json::json!("nope"))).cost.is_none());
}

#[test]
fn plan_numbers_say_how_old_they_are() {
    let plan = |updated: f64, week_reset: f64, five_reset: f64| model::Plan {
        week: Some(81.0),
        five_hour: Some(12.0),
        week_reset: Some(week_reset),
        five_hour_reset: Some(five_reset),
        updated: Some(updated),
        ..Default::default()
    };
    let fresh = plan(NOW - 300.0, NOW + 86400.0, NOW + 3600.0);
    assert!(!fresh.limits_old(NOW));
    assert_eq!((fresh.as_of(NOW), fresh.week_label(NOW), fresh.five_label(NOW)), (None, "81%".into(), "12%".into()));
    let forty = plan(NOW - 2400.0, NOW + 86400.0, NOW + 3600.0);
    assert!(forty.limits_old(NOW));
    assert_eq!(forty.as_of(NOW).as_deref(), Some("as of 40m ago"));
    let hours = plan(NOW - 3.0 * 3600.0, NOW + 86400.0, NOW + 3600.0);
    assert!(hours.as_of(NOW).unwrap().starts_with("as of ") && !hours.as_of(NOW).unwrap().contains("ago"));
    let old = plan(NOW - 6.0 * 3600.0, NOW + 86400.0, NOW - 60.0);
    assert_eq!(old.week_label(NOW), "at least 81%");
    assert_eq!(old.five_label(NOW), "--", "its window reset since");
    let reset = plan(NOW - 6.0 * 3600.0, NOW - 60.0, NOW - 60.0);
    assert_eq!(reset.week_label(NOW), "--", "the week reset since");
    let raw: ApiSessions = serde_json::from_value(
        serde_json::json!({"machines": [], "summary": {"seven_day": 40, "limits_updated": NOW - 60.0}}),
    )
    .unwrap();
    assert_eq!(model::normalize(&raw, NOW).plan.updated, Some(NOW - 60.0));
}

#[test]
fn settings_reads_a_machines_folder_versions_capabilities_and_mcp_servers() {
    use sb_core::api::{ApiCapsView, ApiMachines, ApiMcp};
    let m: ApiMachines = serde_json::from_value(serde_json::json!({"machines": [
        {"name": "studio", "label": "Studio", "root": "~/Desktop/Claude", "root_shown": "~/Desktop/Claude", "needs_root": false,
         "versions": {"agent": "abc1234", "claude": "2.1.296"}, "features": ["mcp", "root_dirs"], "auto": true, "can": "iOS, Docker"},
        {"name": "tower", "label": "Tower", "versions": "garbage", "features": 7}
    ]})).unwrap();
    assert_eq!(m.machines[0].versions.as_ref().and_then(|v| v.claude.as_deref()), Some("2.1.296"));
    assert!(m.machines[0].has("mcp") && !m.machines[1].has("mcp"));
    assert!(m.machines[1].versions.is_none(), "an odd field never sinks the list");
    let mcp: ApiMcp = serde_json::from_value(serde_json::json!({"servers": [
        {"name": "github", "scope": "user", "type": "stdio", "command": "npx", "args": ["-y", "server-github", "--token", "•••"], "env": ["GITHUB_TOKEN"]},
        {"name": "linear", "scope": "project", "type": "http", "url": "https://mcp.linear.app/sse"},
        {"name": "local-one", "scope": "local", "folder": "proj", "command": "node", "args": ["server.js"]},
        {"name": "browser", "scope": "plugin", "plugin": "tools", "command": "browser-mcp"}
    ]})).unwrap();
    let s = &mcp.servers;
    assert_eq!(s[0].line(), "npx -y server-github --token •••");
    assert_eq!(s[1].line(), "https://mcp.linear.app/sse");
    assert_eq!(
        s.iter().map(|x| x.scope_label()).collect::<Vec<_>>(),
        ["User", "Project", "Local, in proj", "Plugin tools"]
    );
    assert_eq!(s[0].kind.as_deref(), Some("stdio"));
    let caps: ApiCapsView =
        serde_json::from_value(serde_json::json!({"on": true, "extra": ["Can print labels"], "off": ["cuda"],
        "tools": [{"key": "cuda", "name": "CUDA", "detail": "13.2 on the RTX GPU", "on": false}], "can": "Docker"}))
        .unwrap();
    assert!(caps.on && !caps.tools.unwrap()[0].on);
}

#[test]
fn a_rename_shows_at_once_and_says_where_it_cannot() {
    let mut s = hand();
    let k = "studio:11111111-1111-4111-8111-111111111111";
    assert!(s.retitle(k, "Widget fix (part 2)"));
    let c = s.session(k).unwrap();
    assert_eq!(
        (c.topic.as_str(), c.tag.as_deref(), c.raw_title.as_str()),
        ("Widget fix", Some("part 2"), "Widget fix (part 2)")
    );
    assert!(!s.retitle("studio:nope", "x"));
    let r =
        actions::for_session(s.session(k).unwrap(), &s, NOW).into_iter().find(|a| a.id == ActionId::Rename).unwrap();
    assert_eq!((r.title.as_str(), r.icon.as_str()), ("Rename chat", "pencil"));
    if let Some(m) = s.machines.iter_mut().find(|m| m.name == "studio") {
        m.online = Some(true);
        m.features = vec!["browse".into()];
    }
    assert!(!s.can_rename("studio"), "an older agent cannot keep a name");
    assert!(
        actions::for_session(s.session(k).unwrap(), &s, NOW).iter().any(|a| a.id == ActionId::Rename && a.disabled)
    );
}

#[test]
fn a_cleanup_stop_says_what_is_still_running() {
    let r = serde_json::json!({"ok": true, "stopped": ["studio/a"], "skipped": ["studio/b"],
        "failed": [{"key": "mbp/c", "error": "Claude Code could not confirm it stopped."}], "queued": [{"key": "studio/d", "job": "j1"}]});
    let s = sb_core::CleanStopped::read(&r);
    assert_eq!(s.queued, vec![("studio/d".to_string(), "j1".to_string())]);
    assert_eq!(
        s.not_stopped,
        vec![
            ("studio/b".to_string(), String::new()),
            ("mbp/c".to_string(), "Claude Code could not confirm it stopped.".to_string())
        ]
    );
    assert_eq!(sb_core::CleanStopped::read(&serde_json::json!({"ok": true})), sb_core::CleanStopped::default());
}

#[test]
fn model_picker_list() {
    let list: sb_core::api::ApiModels = serde_json::from_str(
        r#"{"ok": true, "models": [{"id": "fable", "label": "Fable", "detail": "Fable 5.1"}, {"id": "opus", "label": "Opus"},
            {"id": "claude-zephyr-1"}]}"#,
    )
    .unwrap();
    let got = actions::model_choices(Some(&list.models));
    let ids: Vec<&str> = got.iter().map(|m| m.0.as_str()).collect();
    let shown: Vec<&str> = got.iter().map(|m| m.1.as_str()).collect();
    assert_eq!(ids, ["", "fable", "opus", "claude-zephyr-1"]);
    assert_eq!(shown, ["Default", "Fable 5.1", "Opus", "Zephyr 1"]);
    let fallback: Vec<String> = actions::model_choices(None).into_iter().map(|m| m.0).collect();
    assert_eq!(fallback, ["", "fable", "opus", "sonnet", "haiku"]);
    assert_eq!(actions::model_word("claude-opus-5-5"), "Opus 5.5");
    assert_eq!(actions::model_word("claude-haiku-4-5-20251001"), "Haiku 4.5");
    assert_eq!(actions::model_word("claude-opus-4-7[1m]"), "Opus 4.7");
    assert_eq!(actions::model_word("sonnet"), "Sonnet");
}

#[test]
fn model_menu_from_the_catalog() {
    let list: sb_core::api::ApiModels = serde_json::from_str(
        r#"{"ok": true, "models": [{"id": "fable", "label": "Fable", "detail": "Fable 5.2", "description": "For your toughest challenges",
              "version": "claude-fable-5-2"}, {"id": "opus", "label": "Opus", "detail": "Opus 5.5", "version": "claude-opus-5-5"}],
            "more": [{"id": "claude-fable-5-1", "label": "Fable 5.1"}, {"id": "claude-opus-4-8"}, {"id": "opus"}]}"#,
    )
    .unwrap();
    let menu = actions::model_menu(Some(&list));
    let rows: Vec<(&str, &str, &str, bool)> =
        menu.iter().map(|c| (c.id.as_str(), c.name.as_str(), c.desc.as_str(), c.more)).collect();
    assert_eq!(
        rows,
        [
            ("", "Default", "", false),
            ("fable", "Fable 5.2", "For your toughest challenges", false),
            ("opus", "Opus 5.5", "", false),
            ("claude-fable-5-1", "Fable 5.1", "", true),
            ("claude-opus-4-8", "Opus 4.8", "", true),
        ]
    );
    assert_eq!(actions::model_menu_name(&menu, "fable"), "Fable 5.2");
    assert_eq!(actions::model_menu_name(&menu, "claude-opus-5-5"), "Opus 5.5");
    assert_eq!(actions::model_menu_name(&menu, "claude-sonnet-4-6"), "Sonnet 4.6");
    assert_eq!(actions::model_menu_name(&menu, ""), "Default");
    // an older server: no more, no descriptions
    let old: sb_core::api::ApiModels =
        serde_json::from_str(r#"{"models": [{"id": "opus", "label": "Opus"}]}"#).unwrap();
    assert_eq!(actions::model_menu(Some(&old)).len(), 2);
    assert_eq!(actions::model_menu(None).len(), 5);
}

#[test]
fn back_to_back_refreshes_save_the_snapshot_once() {
    // The board changes about once a second while chats work; rewriting the 400 KB snapshot each time was ~2 GB of
    // disk writes a day (macOS flags it). The first refresh saves it; one right after leaves the file alone.
    let (base, _) = switchboard();
    let dir = temp_dir("save-once");
    let core =
        Core::new(CoreConfig { servers: vec![base], cache_dir: Some(dir.display().to_string()), user_agent: None });
    core.refresh().unwrap();
    let path = dir.join("sessions.json");
    assert!(path.exists(), "the first refresh saves the snapshot");
    std::fs::write(&path, b"marker").unwrap();
    core.refresh().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"marker", "a refresh right after the last save does not write");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A chat whose computer says where it compacts.
fn one_with_limit(ctx: f64, limit: serde_json::Value) -> model::Session {
    let raw: ApiSessions =
        serde_json::from_value(serde_json::json!({"machines": [{"name": "studio", "label": "Studio",
        "online": true, "updated": NOW, "sessions": [{"sid": "aaaaaaaa-1111-4111-8111-111111111111",
        "title": "Bay Area planning", "mtime": NOW - 27.0 * 3600.0, "active": NOW - 27.0 * 3600.0, "ctx": ctx,
        "cache_left": 0.0, "limit": limit}]}], "summary": {}}))
        .unwrap();
    model::normalize(&raw, NOW).machines[0].sessions[0].clone()
}

#[test]
fn a_chat_is_measured_against_where_it_really_compacts() {
    let s = one_with_limit(250_000.0, serde_json::json!(500_000));
    assert_eq!((s.limit(), s.over(), s.ctx_frac()), (500_000, false, 0.5));
    assert_eq!(s.ctx_of_limit(), "250k of 500k");
    let s = one_with_limit(210_000.0, serde_json::json!(200_000));
    assert!(s.over(), "a model with a 200k window is over at 200k whatever the setting says");
    assert_eq!(one_with_limit(400_000.0, serde_json::json!(1_000_000)).ctx_of_limit(), "400k of 1M");
}

#[test]
fn a_chat_with_no_limit_or_an_odd_one_is_measured_against_300k_as_before() {
    for odd in [
        serde_json::Value::Null,
        serde_json::json!(0),
        serde_json::json!(-5),
        serde_json::json!(99_999),
        serde_json::json!(5_000_000),
        serde_json::json!("300000"),
        serde_json::json!([1]),
        serde_json::json!(1e30),
    ] {
        let s = one_with_limit(150_000.0, odd.clone());
        assert_eq!((s.limit(), s.ctx_frac()), (300_000, 0.5), "{odd}");
    }
    assert_eq!(one(150_000.0, 0.0, None).limit(), 300_000, "an older helper sends no limit at all");
    assert_eq!(one(150_000.0, 0.0, None).ctx_of_limit(), "150k of 300k");
}

#[test]
fn start_fresh_is_suggested_near_the_chats_own_limit() {
    use sb_core::actions::{Pick, suggest};
    // 280k: close to compacting at 300k, far from it at 500k
    let near = suggest(&one_with_limit(280_000.0, serde_json::json!(300_000)), NOW);
    assert_eq!(near.pick, Some(Pick::Fresh));
    assert!(near.why.contains("close to the 300k point"), "{}", near.why);
    let far = suggest(&one_with_limit(280_000.0, serde_json::json!(500_000)), NOW);
    assert!(!far.why.contains("squeezes"), "{}", far.why);
    let at = suggest(&one_with_limit(450_000.0, serde_json::json!(500_000)), NOW);
    assert!(at.why.contains("At 450k it's close to the 500k point"), "{}", at.why);
    let small = suggest(&one_with_limit(180_000.0, serde_json::json!(200_000)), NOW);
    assert!(small.why.contains("close to the 200k point"), "{}", small.why);
    // no limit known: 270k, as before
    assert!(suggest(&one(270_000.0, 0.0, None), NOW).why.contains("close to the 300k point"));
    assert!(!suggest(&one(269_000.0, 0.0, None), NOW).why.contains("squeezes"));
}

// ---------- keys: the server's routes and the node's control API, over real sockets ----------

fn json(body: &str) -> serde_json::Value {
    serde_json::from_str(body).unwrap_or_default()
}

fn core_at(base: String, tag: &str) -> Core {
    Core::new(CoreConfig {
        servers: vec![base],
        cache_dir: Some(temp_dir(tag).display().to_string()),
        user_agent: None,
    })
}

#[test]
fn a_server_from_before_keys_has_no_keys() {
    // Every keys route is a 404 there, and that is "off", not an error.
    let (base, _) = switchboard();
    let core = core_at(base, "keys-old");
    assert_eq!(core.keys_readiness().unwrap(), None);
    assert!(sb_core::keys::Keys { server: core.keys_readiness().unwrap(), node: None }.hidden());
    assert!(matches!(core.keys_devices(), Err(CoreError::Server { status: 404, .. })));
}

#[test]
fn the_keys_routes_decode_as_the_server_sends_them() {
    use sb_core::keys::{Mode, Path};
    let (base, seen) = serve(|method, path, body| {
        match (method, path) {
        ("GET", "/api/summary") => (200, SUMMARY.into()),
        ("GET", "/api/keys/readiness") => (
            200,
            r#"{"ok":true,"keys":"both","ready":false,"days":7,"watching_since":1799000000.0,
                "blockers":[{"kind":"machine","text":"Tower is not paired with this home computer. Pair it with a code first, or it loses its connection.","machine":"tower","label":"Tower","paired":false},
                            {"kind":"direct","text":"The iPhone app connected by address 2 hours ago. It would stop working. Pair the phone with this home computer in the app first.","client":"iPhone","last":1799990000.0}],
                "machines":[{"name":"studio","label":"Studio","home":true,"paired":false,"seen":null,"path":null},
                            {"name":"laptop","label":"Netbook","home":false,"paired":true,"seen":1799999990.0,"path":"direct"}],
                "direct":[{"client":"iPhone","last":1799990000.0}]}"#
                .into(),
        ),
        ("GET", "/api/keys/devices") => (
            200,
            r#"{"ok":true,"devices":[{"id":"k1","name":"Netbook","kind":"computer","machine":"laptop","paired":1799000000.0,"via":"code","last_seen":1799999990.0,"path":"direct"},
                                     {"id":"k2","name":"Phone","kind":"phone","paired":1799000100.0,"via":"link","last_seen":null,"path":null}]}"#
                .into(),
        ),
        ("POST", "/api/keys/devices") if json(body)["remove"] == "k2" => (200, r#"{"ok":true}"#.into()),
        ("POST", "/api/keys/devices") => (400, r#"{"ok":false,"error":"Say which device to remove."}"#.into()),
        ("POST", "/api/pair_device") => (
            200,
            r#"{"ok":true,"code":"ABCD-EFGH","link":"switchboard://pair/1?h=00ff&s=0011","expires":1800000600.0}"#.into(),
        ),
        _ => (404, r#"{"ok":false,"error":"not found"}"#.into()),
    }
    });
    let core = core_at(base, "keys-new");
    let r = core.keys_readiness().unwrap().expect("the route is there");
    assert_eq!(r.keys, Mode::Both);
    assert!(!r.ready);
    assert_eq!(r.blockers.len(), 2);
    assert_eq!(r.machines.len(), 2);
    assert_eq!(sb_core::keys::machine_line(&r, "laptop"), Some("Direct"));
    let d = core.keys_devices().unwrap();
    assert_eq!(d.len(), 2);
    assert_eq!((d[0].id.as_str(), d[0].path), ("k1", Some(Path::Direct)));
    assert_eq!(d[1].last_seen, None);
    core.keys_remove_device("k2").unwrap();
    let offer = core.pair_device().unwrap();
    assert_eq!(offer.code, "ABCD-EFGH");
    assert!(offer.link.starts_with("switchboard://pair/1?"));
    assert_eq!(offer.expires, 1_800_000_600.0);
    let seen = seen.lock().unwrap();
    let post = seen.requests.iter().find(|r| r.0 == "POST" && r.1 == "/api/keys/devices").unwrap();
    assert!(post.2.iter().any(|(k, v)| k == "x-switchboard" && v == "1"), "the header every POST carries");
}

#[test]
fn the_mode_route_is_called_and_its_answers_kept() {
    use sb_core::keys::ModeRefusal;
    let (base, seen) = serve(|method, path, body| {
        match (method, path) {
        ("GET", "/api/summary") => (200, SUMMARY.into()),
        ("GET", "/api/keys/readiness") => (200, r#"{"ok":true,"keys":"off","switch":true,"here":true}"#.into()),
        ("POST", "/api/keys/mode") => match json(body)["keys"].as_str() {
            Some("both") => (200, r#"{"ok":true,"keys":"both","restarting":true}"#.into()),
            Some("off") => (200, r#"{"ok":true,"keys":"off","restarting":false}"#.into()),
            Some("only") => (
                409,
                r#"{"ok":false,"error":"Not yet: switching to keys only now would cut something off. Tower is not paired.","blockers":[{"kind":"machine","text":"Tower is not paired."}]}"#.into(),
            ),
            Some("by-hand") => (
                409,
                r#"{"ok":false,"error":"This server cannot restart itself from here. To change this, run on the home computer: switchboard keys on","command":"switchboard keys on"}"#.into(),
            ),
            _ => (400, r#"{"ok":false,"error":"keys must be off, both or only"}"#.into()),
        },
        _ => (404, r#"{"ok":false,"error":"not found"}"#.into()),
    }
    });
    let core = core_at(base, "keys-mode");
    let k = sb_core::keys::Keys { server: core.keys_readiness().unwrap(), node: None };
    assert!(k.can_turn_on() && k.devices_page() && !k.on());
    assert_eq!(core.keys_set_mode("both").unwrap(), Ok(true));
    assert_eq!(core.keys_set_mode("off").unwrap(), Ok(false));
    assert_eq!(
        core.keys_set_mode("only").unwrap(),
        Err(ModeRefusal {
            text: "Not yet: switching to keys only now would cut something off.".into(),
            command: None,
            blockers: vec!["Tower is not paired.".into()],
        })
    );
    let by_hand = core.keys_set_mode("by-hand").unwrap().unwrap_err();
    assert_eq!(by_hand.command.as_deref(), Some("switchboard keys on"));
    assert!(!by_hand.text.contains("switchboard keys on"), "the command is shown once, apart");
    let e = core.keys_set_mode("sideways").unwrap_err();
    assert!(e.to_string().contains("keys must be off, both or only"), "{e}");
    let seen = seen.lock().unwrap();
    let first = seen.requests.iter().find(|r| r.0 == "POST" && r.1 == "/api/keys/mode").unwrap();
    assert_eq!(json(&first.3), serde_json::json!({"keys": "both"}));
    assert!(first.2.iter().any(|(k, v)| k == "x-switchboard" && v == "1"));
}

#[test]
fn a_home_node_that_does_not_answer_is_said_in_the_servers_words() {
    let (base, _) = serve(|method, path, _| match (method, path) {
        ("GET", "/api/summary") => (200, SUMMARY.into()),
        ("GET", "/api/keys/devices") | ("POST", "/api/pair_device") => {
            (502, r#"{"ok":false,"error":"The node on this computer did not answer (connection refused)."}"#.into())
        }
        _ => (404, r#"{"ok":false,"error":"not found"}"#.into()),
    });
    let core = core_at(base, "keys-502");
    for e in [core.keys_devices().err().unwrap(), core.pair_device().err().unwrap()] {
        assert!(e.to_string().contains("The node on this computer did not answer"), "{e}");
    }
}

#[test]
fn the_node_control_client_speaks_the_assumed_shapes() {
    use sb_core::keys::{HttpNode, JoinProblem, NodeControl, NodeError, Path, Role, join_problem};
    let (base, seen) = serve(|method, path, body| {
        match (method, path) {
        ("GET", "/status") => (
            200,
            r#"{"ok":true,"keys":true,"role":"member","machine":"laptop","id":"k1","home":{"id":"h1","name":"Studio"},
                "relay":"https://relay.example","peers":[{"id":"h1","name":"Studio","path":"direct","rtt_ms":3.5}],"problems":[]}"#
                .into(),
        ),
        ("POST", "/pair/join") if body.contains("ABCD-EFGH") => {
            (200, r#"{"ok":true,"home":{"id":"h1","name":"Studio"},"machine":"laptop"}"#.into())
        }
        ("POST", "/pair/join") => (403, r#"{"ok":false,"error":"That code is wrong. Check it and try again."}"#.into()),
        ("GET", "/devices") => (200, r#"{"ok":true,"devices":[{"id":"k1","name":"Netbook","kind":"computer"}]}"#.into()),
        ("POST", "/devices/remove") => (200, r#"{"ok":true}"#.into()),
        ("POST", "/pair/new") => (200, r#"{"ok":true,"code":"WXYZ-2345","link":"switchboard://pair/1?h=1","expires":9}"#.into()),
        ("POST", "/relay") => (200, r#"{"ok":true}"#.into()),
        _ => (404, "Not Found".into()),
    }
    });
    let node = HttpNode::at(&base).expect("the fake listens on loopback");
    let s = node.status().unwrap();
    assert!(s.keys);
    assert_eq!(s.role, Role::Member);
    assert_eq!(s.peers[0].path, Some(Path::Direct));
    assert_eq!(sb_core::keys::home_line(&s).unwrap().text, "Direct");
    assert_eq!(node.join("ABCD-EFGH", Some("Laptop"), &[], false).unwrap().home.as_deref(), Some("Studio"));
    let no = node.join("WWWW-WWWW", None, &["192.0.2.20:41641".to_string()], false).unwrap_err();
    assert!(matches!(no, NodeError::Said { status: 403, .. }));
    assert_eq!(join_problem(&no), JoinProblem::WrongCode);
    assert_eq!(node.devices().unwrap().len(), 1);
    node.remove_device("k1").unwrap();
    assert_eq!(node.pair_new().unwrap().code, "WXYZ-2345");
    node.set_relay("https://relay.example").unwrap();
    let seen = seen.lock().unwrap();
    let body_of = |p: &str| seen.requests.iter().find(|r| r.0 == "POST" && r.1 == p).map(|r| json(&r.3)).unwrap();
    assert_eq!(body_of("/pair/join"), serde_json::json!({"input": "ABCD-EFGH", "name": "Laptop"}));
    let second = seen.requests.iter().filter(|r| r.0 == "POST" && r.1 == "/pair/join").nth(1).map(|r| json(&r.3));
    assert_eq!(second, Some(serde_json::json!({"input": "WWWW-WWWW", "addrs": ["192.0.2.20:41641"]})));
    assert_eq!(body_of("/devices/remove"), serde_json::json!({"id": "k1"}));
    assert_eq!(body_of("/relay"), serde_json::json!({"url": "https://relay.example"}));
}

#[test]
fn no_node_on_this_computer_is_absent_not_an_error_to_show() {
    use sb_core::keys::{HttpNode, NodeControl, NodeError};
    // A port nothing listens on.
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    drop(l);
    assert_eq!(HttpNode::at(&base).unwrap().status(), Err(NodeError::Absent));
    // Something else holds the port and knows no such route.
    let (other, _) = serve(|_, _, _| (404, "Not Found".into()));
    assert_eq!(HttpNode::at(&other).unwrap().status(), Err(NodeError::Absent));
}

// ---- the home computer is away (keys phase 4a.10) ----

fn away_reply(away: serde_json::Value) -> ApiSessions {
    let mut v: serde_json::Value = serde_json::from_slice(&fixture("api_sessions.json")).unwrap();
    v["away"] = away;
    serde_json::from_value(v).unwrap()
}

#[test]
fn a_board_says_when_the_home_computer_is_away_and_one_without_the_word_does_not() {
    assert!(hand().away.is_none(), "a server's own answer has no away");
    let raw = away_reply(serde_json::json!({
        "since": 1795362330u64,
        "home": {"id": "ab".repeat(32), "name": "Studio", "last_seen": 1795362329u64},
        "showing": [{"machine": "laptop", "label": "Laptop"}, {"machine": "tower", "label": "Tower"}],
        "missing": [{"machine": "mini", "label": "Mini", "why": "needs_update"},
                    {"machine": "studio", "label": "Studio", "why": "is_home"}],
        "roster": {"at": 1795362332u64, "expires": 1795967132u64},
        "line": "Home is away. Showing chats from Laptop and Tower.",
        "notes": ["Mini needs an update before it can answer while the home computer is away."],
        "lacks": ["new_chat", "move", "history"],
        "lacks_line": "Not available while the home computer is away: new chats, moving a chat, history.",
    }));
    let s = model::normalize(&raw, NOW);
    let a = s.away.as_ref().expect("away is read");
    assert_eq!(a.line(), "Home is away. Showing chats from Laptop and Tower.");
    assert_eq!(a.reason(), "Not available while the home computer is away: new chats, moving a chat, history.");
    assert_eq!(a.showing.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Laptop", "Tower"]);
    assert_eq!(a.missing[0].why.as_deref(), Some("needs_update"));
    assert_eq!(
        (a.missing[0].sentence().as_str(), a.missing[0].word()),
        ("Mini needs an update before it can answer while the home computer is away.", "Needs an update")
    );
    let home = a.missing_of("studio").unwrap();
    assert_eq!((home.sentence().as_str(), home.word()), ("The home computer is away.", "Away"));
    assert!(a.missing_of("laptop").is_none());
    assert_eq!(a.home.as_ref().and_then(|h| h.name.as_deref()), Some("Studio"));
    assert_eq!(a.roster.as_ref().and_then(|r| r.at), Some(1795362332.0));
    assert_eq!(a.notes.len(), 1);
    // the board is drawn again when away begins, ends, or says something else
    assert!(!s.same_board(&hand()));
    let other = away_reply(serde_json::json!({"line": "Home is away. No other computer can be reached."}));
    assert!(!s.same_board(&model::normalize(&other, NOW)));
    assert!(s.same_board(&model::normalize(&raw, NOW)));

    // every line the helper has is shown as it is; an answer with none still says the one thing known
    for line in [
        "Home is away. Showing chats from Laptop.",
        "Home is away. Showing chats from Laptop, Tower and Mini.",
        "Home is away. No other computer can be reached.",
        "Home is away. Asking the other computers for their chats.",
    ] {
        let s = model::normalize(&away_reply(serde_json::json!({ "line": line })), NOW);
        assert_eq!(s.away.unwrap().line(), line);
    }
    let bare = model::normalize(&away_reply(serde_json::json!({})), NOW).away.unwrap();
    assert_eq!((bare.line(), bare.reason()), ("Home is away.", sb_core::api::AWAY_REASON));
    // a shape this app does not know is no reason to lose the board
    let odd = model::normalize(&away_reply(serde_json::json!("soon")), NOW);
    assert!(odd.away.is_none() && odd.machines.len() == 3);
}

#[test]
fn while_the_home_computer_is_away_only_what_a_chats_own_computer_does_is_on() {
    let s = model::normalize(&away_reply(serde_json::json!({"line": "Home is away. Showing chats from Laptop."})), NOW);
    let home = hand();
    let mut seen = 0;
    for chat in s.sessions() {
        let a = actions::for_session(chat, &s, NOW);
        let was = actions::for_session(home.session(&chat.key).unwrap(), &home, NOW);
        assert_eq!(a.len(), was.len(), "nothing is hidden, so nothing moves: {}", chat.key);
        for x in &a {
            let before = was.iter().find(|w| w.id == x.id).unwrap();
            if actions::works_away(x.id) {
                assert_eq!((x.disabled, &x.note), (before.disabled, &before.note), "{:?} is as it was", x.id);
            } else {
                assert!(x.disabled && !x.primary && x.key.is_none(), "{:?} needs the home computer", x.id);
                assert_eq!(x.note, sb_core::api::AWAY_REASON);
                seen += 1;
            }
        }
        assert!(a.iter().filter(|x| x.primary).all(|x| !x.disabled), "the primary one is one that works");
    }
    assert!(seen > 0);
    for id in
        [ActionId::Terminal, ActionId::ResumeOpen, ActionId::Stop, ActionId::Dismiss, ActionId::Pass, ActionId::Resume]
    {
        assert!(actions::works_away(id), "{id:?}");
    }
    for id in [ActionId::Fresh, ActionId::Continue, ActionId::Handoff, ActionId::Keep, ActionId::Rename, ActionId::Link]
    {
        assert!(!actions::works_away(id), "{id:?}");
    }
}

#[test]
fn a_computer_shows_the_list_it_holds_and_blocks_a_device_on_it() {
    use sb_core::keys::{self, FakeNode, HeldList, NodeControl, NodeStatus};
    let v = serde_json::json!({
        "ok": true,
        "roster": {"epoch": 1, "seq": 4, "at": NOW - 7200.0, "expires": NOW - 7200.0 + 7.0 * 86400.0},
        "devices": [
            {"id": "aa", "name": "Studio", "kind": "computer", "machine": "studio", "home": true, "this": false, "blocked": false},
            {"id": "bb", "name": "Laptop", "kind": "computer", "machine": "laptop", "home": false, "this": true, "blocked": false},
            {"id": "cc", "name": "Tower", "kind": "computer", "machine": "tower", "home": false, "this": false, "blocked": true},
            {"id": "dd", "name": "Sam's iPhone", "kind": "phone", "home": false, "this": false, "blocked": false},
            {"name": "no id"},
        ],
    });
    let held = keys::read_held(&v);
    assert_eq!(held.devices.len(), 4);
    assert_eq!(held.devices.iter().map(|d| d.can_block()).collect::<Vec<_>>(), [false, false, true, true]);
    assert!(held.devices[2].blocked && held.devices[3].phone && held.devices[0].home && held.devices[1].here);
    assert_eq!(
        keys::held_line(&held, NOW),
        "This computer answers the devices on the home computer's list. Its copy was signed 2h ago."
    );
    assert_eq!(keys::held_line(&held, NOW + 8.0 * 86400.0), keys::HELD_STALE);
    let none = keys::read_held(&serde_json::json!({"ok": true, "roster": null, "devices": []}));
    assert_eq!((none.clone(), keys::held_line(&none, NOW)), (HeldList::default(), keys::HELD_NONE.to_string()));

    let node = FakeNode::with(NodeStatus::default());
    node.state.lock().unwrap().held = held;
    node.block("dd", true).unwrap();
    assert!(node.held().unwrap().devices[3].blocked);
    node.block("dd", false).unwrap();
    assert!(!node.held().unwrap().devices[3].blocked);
    assert!(node.block("aa", true).is_err(), "the home computer cannot be blocked");
    assert_eq!(node.state.lock().unwrap().calls, ["block dd", "held", "unblock dd", "held", "block aa"]);
    for line in [keys::HELD_TITLE, keys::HELD_NONE, keys::HELD_STALE, keys::BLOCK_EXPLAINS] {
        assert!(!line.contains('\u{2014}') && !line.contains('!'), "{line}");
    }
}
