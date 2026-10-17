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
    assert_eq!(b.column_counts, vec![2, 2, 3, 2, 2]);
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
    assert_eq!(term.note, "It is open in a terminal on Studio. Only background chats open here.");
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
        "Writes a summary on Studio and opens a new chat with your notes and the held message, so the next message does not re-read 210k. The old chat moves to Replaced."
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
    assert_eq!(total, 76 - 52, "stale chats stay out of the board");
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
        claude_app_link("https://claude.ai/code/session_01TpXMroLvVvsYTKwF6n1BUm").as_deref(),
        Some("claude://code/session_01TpXMroLvVvsYTKwF6n1BUm")
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
    assert_eq!(keep.note, "Switchboard leaves it running. Click to let it stop it when idle.");
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
        "145k, resuming re-reads 145k; Start fresh is cheaper if you do not need the history"
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
