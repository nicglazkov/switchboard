//! Replays a terminal capture (SB_TERM_REC=1, see sb-core term/rec.rs) through alacritty at the recorded sizes, the
//! way the desktop app parses it, and prints the screen it ends on.
//!
//!     cargo run -p sb-core --features grid --example term_replay -- <capture.rec> [--check] [--screens] [--until <ms>]
//!
//! --check   every snapshot the hub sent after output had started must draw exactly the screen the replay had at
//!           that point (cells, colors, cursor, modes). A difference means the stream and the hub's grid disagree:
//!           a resize parsed at the wrong place, lost bytes, or a snapshot that misses something. Exits 1 on any.
//! --screens print the screen after every size change.
//! --until   stop at this many milliseconds into the capture.

use alacritty_terminal::event::VoidListener;
use sb_core::term::rec::{self, Kind};
use sb_core::term::screen::{self, Screen};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.iter().find(|a| !a.starts_with("--") && !a.chars().all(|c| c.is_ascii_digit())) else {
        eprintln!("usage: term_replay <capture.rec> [--check] [--screens] [--until <ms>]");
        std::process::exit(2);
    };
    let check = args.iter().any(|a| a == "--check");
    let screens = args.iter().any(|a| a == "--screens");
    let until: Option<u64> =
        args.iter().position(|a| a == "--until").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok());
    let data = std::fs::read(path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(2);
    });
    let frames = rec::read(&data).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(2);
    });
    let r = replay(&frames, until, screens);
    println!("{}", r.screen.join("\n"));
    println!("---");
    println!(
        "{} frames, {} KB out, {} KB in, sizes {:?}, {} snapshots ({} checked), {} mismatches",
        frames.len(),
        r.out_bytes / 1024,
        r.in_bytes / 1024,
        r.sizes,
        r.snapshots,
        r.checked,
        r.mismatches.len()
    );
    for m in &r.mismatches {
        println!("{m}");
    }
    if check && !r.mismatches.is_empty() {
        std::process::exit(1);
    }
}

struct Replay {
    screen: Vec<String>,
    out_bytes: usize,
    in_bytes: usize,
    sizes: Vec<(u16, u16)>,
    snapshots: usize,
    checked: usize,
    mismatches: Vec<String>,
}

fn initial_size(frames: &[rec::Frame]) -> (u16, u16) {
    let note = frames
        .iter()
        .find(|f| f.kind == Kind::Note)
        .map(|f| String::from_utf8_lossy(&f.bytes).to_string())
        .unwrap_or_default();
    let get = |k: &str, d: u16| {
        note.split(['&', ' ']).find_map(|p| p.strip_prefix(k)).and_then(|v| v.parse().ok()).unwrap_or(d)
    };
    (get("cols=", 120), get("rows=", 36))
}

fn replay(frames: &[rec::Frame], until: Option<u64>, screens: bool) -> Replay {
    let (c, r) = initial_size(frames);
    let mut term: Screen<VoidListener> = Screen::new(c, r, 5000, VoidListener);
    let mut out = Replay {
        screen: vec![],
        out_bytes: 0,
        in_bytes: 0,
        sizes: vec![(c, r)],
        snapshots: 0,
        checked: 0,
        mismatches: vec![],
    };
    let mut snap: Option<Vec<u8>> = None;
    let mut had_output = false;
    for f in frames {
        if until.is_some_and(|u| f.ms > u) {
            break;
        }
        match f.kind {
            Kind::Out => {
                out.out_bytes += f.bytes.len();
                match snap.as_mut() {
                    Some(s) => s.extend_from_slice(&f.bytes),
                    None => {
                        term.feed(&f.bytes);
                        had_output = true;
                    }
                }
            }
            Kind::Input => out.in_bytes += f.bytes.len(),
            Kind::TextIn => {
                let v: serde_json::Value = serde_json::from_slice(&f.bytes).unwrap_or_default();
                match v["type"].as_str().unwrap_or("") {
                    "size" => {
                        let (c, r) = (v["cols"].as_u64().unwrap_or(0) as u16, v["rows"].as_u64().unwrap_or(0) as u16);
                        if c > 0 && r > 0 && (c, r) != term.size() {
                            term.resize(c, r);
                            out.sizes.push((c, r));
                            if screens {
                                println!("== {} ms: {c}x{r}", f.ms);
                                println!("{}", screen::dump(&term.term, 0).join("\n"));
                            }
                        }
                    }
                    "snapshot_begin" => snap = Some(Vec::new()),
                    "snapshot_end" => {
                        let Some(bytes) = snap.take() else { continue };
                        out.snapshots += 1;
                        let (c, r) = term.size();
                        let mut fresh: Screen<VoidListener> = Screen::new(c, r, 5000, VoidListener);
                        let mut alone: Screen<VoidListener> = Screen::new(c, r, 5000, VoidListener);
                        alone.feed(&bytes);
                        if had_output {
                            out.checked += 1;
                            let (want, got) = (screen::dump(&term.term, 0), screen::dump(&alone.term, 0));
                            let (ws, gs) = (screen::state(&term.term), screen::state(&alone.term));
                            if want != got || ws != gs {
                                let line = want.iter().zip(&got).position(|(a, b)| a != b);
                                let mut m =
                                    format!("snapshot at {} ms ({c}x{r}) differs from the replayed screen", f.ms);
                                if let Some(l) = line {
                                    m.push_str(&format!(
                                        "\n  line {l}\n  replay:   {}\n  snapshot: {}",
                                        want[l], got[l]
                                    ));
                                }
                                if ws != gs {
                                    m.push_str(&format!("\n  replay:   {ws}\n  snapshot: {gs}"));
                                }
                                out.mismatches.push(m);
                            }
                        }
                        // as the app does: its own scrollback carried over, then the screen from the snapshot
                        screen::carry_history(&mut term.term, &mut fresh.term);
                        fresh.feed(&bytes);
                        term = fresh;
                        had_output = true;
                    }
                    _ => {}
                }
            }
            Kind::TextOut | Kind::Note => {}
        }
    }
    out.screen = screen::dump(&term.term, 0);
    out
}
