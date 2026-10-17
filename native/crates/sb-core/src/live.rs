//! A chat's live state in words, the one display mapping every view uses: the board's card pill and second line,
//! the inspector, the palette, the archive and browser dots, the usage list, and a terminal tab's dot and strip
//! (docs/specs/2026-10-14-live-state.md). The server decides (its `activity`, `cache`, `cold_at`, `heavy`); this only
//! says it, with the cache counting down on this computer's clock, greyed when the data is old.
//!
//! A terminal's own output adds one thing: while the server says the chat is quiet, output in the last few seconds
//! reads "Working" (a hint), without touching needs, held, parked, stalled or the cache.

use crate::fmt;
use crate::model::{Activity, Cache, GUARD, Session};

/// Terminal output this recent is a "working" hint (Claude Code's spinner redraws every second while it works).
pub const QUIET: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiveKind {
    Held,
    Wait,
    Parked,
    Run,
    Stall,
    Warm,
    Cold,
    /// Quiet, with no cache known (or nothing to re-read yet).
    Idle,
}

impl LiveKind {
    /// The fixture's name for it (tests/fixtures/live-state.json).
    pub fn name(self) -> &'static str {
        match self {
            LiveKind::Held => "held",
            LiveKind::Wait => "needs",
            LiveKind::Parked => "parked",
            LiveKind::Run => "working",
            LiveKind::Stall => "stalled",
            LiveKind::Warm => "warm",
            LiveKind::Cold => "cold",
            LiveKind::Idle => "idle",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    pub kind: LiveKind,
    /// "Working", "Warm", "Cold", "Needs you", "Parked", "Held", "Stalled", "Quiet". "Working" too while a quiet
    /// chat's terminal shows output (`hint`).
    pub word: &'static str,
    /// What goes with the word: "cache good for 4:12", "next message re-reads 41k", what Claude asked; "as of 2m
    /// ago" when the data is old.
    pub detail: String,
    /// The card's pill: what going back costs ("warm", "cold", "cache ?"), or what it is doing when that says more
    /// ("working", "stalled", "parked"). None for a chat with nothing to re-read.
    pub pill: Option<&'static str>,
    /// The pill's color.
    pub pill_kind: LiveKind,
    /// Seconds of warm cache left (0 when not warm, or when the data is too old to count down).
    pub left: u64,
    /// The last minute of a warm cache.
    pub urgent: bool,
    /// 150k or more: "expensive", or "re-reads 312k" once it is cold.
    pub expensive: Option<String>,
    /// "as of 2m ago": the data is older than LIVE_STALE or the link to the server is down. Every view greys.
    pub stale: Option<String>,
    /// A quiet chat's terminal shows output right now.
    pub hint: bool,
}

impl Live {
    /// "Warm, cache good for 4:12": the whole state as one line.
    pub fn text(&self) -> String {
        if self.detail.is_empty() { self.word.to_string() } else { format!("{}, {}", self.word, self.detail) }
    }
    /// The dot breathes while it works.
    pub fn working(&self) -> bool {
        self.kind == LiveKind::Run || self.hint
    }
}

/// "4:12", "0:07", "1:02:05"
pub fn clock(s: u64) -> String {
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

/// "as of 45s ago", "as of 2m ago", "as of 3h ago", "as of 2d ago"
pub fn as_of_text(age: f64) -> String {
    let a = age.max(0.0) as u64;
    let n = if a < 60 {
        format!("{a}s")
    } else if a < 3600 {
        format!("{}m", a / 60)
    } else if a < 86400 {
        format!("{}h", a / 3600)
    } else {
        format!("{}d", a / 86400)
    };
    format!("as of {n} ago")
}

/// The live state of `s` at `now` (this computer's clock). `link`: the app's link to the server is up.
/// `last_out`: when a terminal showing the chat last printed Claude's own output (None: no terminal, or nothing yet).
pub fn live(s: &Session, now: f64, link: bool, last_out: Option<f64>) -> Live {
    // A chat with no replies has nothing cached and nothing to re-read, whatever the cache says.
    let cache = if s.ctx == 0 { Cache::Unknown } else { s.cache_now(now) };
    let k = fmt::k(s.ctx);
    let cache_kind = match cache {
        Cache::Warm => LiveKind::Warm,
        Cache::Cold => LiveKind::Cold,
        Cache::Unknown => LiveKind::Idle,
    };
    let kind = match s.activity {
        Activity::Held => LiveKind::Held,
        Activity::Needs => LiveKind::Wait,
        Activity::Parked => LiveKind::Parked,
        Activity::Working => LiveKind::Run,
        Activity::Stalled => LiveKind::Stall,
        Activity::Quiet => cache_kind,
    };
    let stale = s
        .stale_data(now, link)
        .then(|| if s.as_of > 0.0 { as_of_text(now - s.as_of) } else { "as of earlier".to_string() });
    let left = if cache == Cache::Warm && stale.is_none() { (s.cache_until - now).ceil().max(0.0) as u64 } else { 0 };
    let hint = s.activity == Activity::Quiet && last_out.is_some_and(|t| now - t < QUIET && now >= t - 1.0);
    let cache_detail = match cache_kind {
        LiveKind::Warm => format!("cache good for {}", clock(left)),
        LiveKind::Cold => format!("next message re-reads {k}"),
        _ if s.ctx > 0 => "cache ?".to_string(),
        _ => String::new(),
    };
    let word = if hint {
        "Working"
    } else {
        match kind {
            LiveKind::Held => "Held",
            LiveKind::Wait => "Needs you",
            LiveKind::Parked => "Parked",
            LiveKind::Run => "Working",
            LiveKind::Stall => "Stalled",
            LiveKind::Warm => "Warm",
            LiveKind::Cold => "Cold",
            LiveKind::Idle => "Quiet",
        }
    };
    let detail = match (&stale, kind) {
        (Some(t), _) => t.clone(),
        (None, LiveKind::Held) => "your message waits to send".into(),
        (None, LiveKind::Wait) => s
            .ask
            .as_ref()
            .map(|a| a.summary.clone())
            .filter(|t| !t.trim().is_empty())
            .or_else(|| s.needs.clone())
            .unwrap_or_else(|| "waiting for your answer".into()),
        (None, LiveKind::Parked) => format!("waiting {}", fmt::sago(s.active, now)),
        (None, LiveKind::Run) => String::new(),
        (None, LiveKind::Stall) => format!("no word for {}", fmt::sago(s.active, now)),
        (None, _) => cache_detail,
    };
    let (pill, pill_kind) = match s.activity {
        Activity::Parked => (Some("parked"), LiveKind::Parked),
        Activity::Working => (Some("working"), LiveKind::Run),
        Activity::Stalled => (Some("stalled"), LiveKind::Stall),
        _ => match cache {
            Cache::Warm => (Some("warm"), LiveKind::Warm),
            Cache::Cold => (Some("cold"), LiveKind::Cold),
            Cache::Unknown if s.ctx > 0 => (Some("cache ?"), LiveKind::Idle),
            Cache::Unknown => (None, LiveKind::Idle),
        },
    };
    let busy = matches!(s.activity, Activity::Working | Activity::Stalled);
    let expensive = (s.ctx >= GUARD)
        .then(|| if cache == Cache::Cold && !busy { format!("re-reads {k}") } else { "expensive".to_string() });
    Live { kind, word, detail, pill, pill_kind, left, urgent: left > 0 && left <= 60, expensive, stale, hint }
}

/// "45s", "2m 10s", "1h 05m"
pub fn span(s: u64) -> String {
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{}h {:02}m", s / 3600, (s / 60) % 60)
    }
}

/// The context label beside the strip's meter: "98k of 300k context", or "98k / 300k" when room is short.
pub fn ctx_label(ctx: u64, short: bool) -> String {
    let max = crate::model::COMPACT;
    match (short, ctx > max) {
        (true, _) => format!("{} / {}", fmt::k(ctx), fmt::k(max)),
        (false, false) => format!("{} of {} context", fmt::k(ctx), fmt::k(max)),
        (false, true) => format!("{} context, past {}", fmt::k(ctx), fmt::k(max)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ApiSessions;
    use crate::model::{Lamp, normalize};
    use serde_json::{Value, json};

    const FIXTURE: &str = include_str!("../../../../tests/fixtures/live-state.json");

    /// One fixture case: the reply as the server sent it, read at `received`, shown at `now` (both this computer's clock).
    fn shown(c: &Value) -> (Session, f64, bool, Option<f64>) {
        let raw: ApiSessions = serde_json::from_value(json!({
            "now": c["server_now"],
            "machines": [dict_with(&c["machine"], "sessions", json!([c["session"]]))],
        }))
        .unwrap();
        let snap = normalize(&raw, c["received"].as_f64().unwrap());
        let now = c["now"].as_f64().unwrap();
        let out = c["out_ago"].as_f64().map(|a| now - a);
        (snap.machines[0].sessions[0].clone(), now, c["link"].as_bool().unwrap(), out)
    }

    fn dict_with(v: &Value, k: &str, x: Value) -> Value {
        let mut v = v.clone();
        v[k] = x;
        v
    }

    #[test]
    fn every_fixture_case_reads_the_same_as_on_the_iphone() {
        let f: Value = serde_json::from_str(FIXTURE).unwrap();
        let cases = f["cases"].as_array().unwrap();
        assert!(cases.len() > 20);
        for c in cases {
            let (s, now, link, out) = shown(c);
            let l = live(&s, now, link, out);
            let e = &c["expect"];
            let name = c["name"].as_str().unwrap();
            assert_eq!(l.kind.name(), e["kind"].as_str().unwrap(), "{name}: kind");
            assert_eq!(l.word, e["word"].as_str().unwrap(), "{name}: word");
            assert_eq!(l.detail, e["detail"].as_str().unwrap(), "{name}: detail");
            assert_eq!(l.pill, e["pill"].as_str(), "{name}: pill");
            assert_eq!(l.left, e["left"].as_u64().unwrap(), "{name}: left");
            assert_eq!(l.urgent, e["urgent"].as_bool().unwrap(), "{name}: urgent");
            assert_eq!(l.expensive.as_deref(), e["expensive"].as_str(), "{name}: expensive");
            assert_eq!(l.stale.as_deref(), e["stale"].as_str(), "{name}: stale");
            assert_eq!(l.hint, e["hint"].as_bool().unwrap(), "{name}: hint");
        }
    }

    fn one(session: Value, server_now: f64, received: f64) -> Session {
        let raw: ApiSessions = serde_json::from_value(json!({
            "now": server_now,
            "machines": [{"name": "studio", "as_of": server_now - 2.0, "sessions": [session]}],
        }))
        .unwrap();
        normalize(&raw, received).machines[0].sessions[0].clone()
    }

    const N: f64 = 1_791_782_309.0;

    #[test]
    fn skew_correction_puts_cold_at_on_this_clock() {
        let w = json!({"sid": "a", "activity": "quiet", "cache": "warm", "cold_at": N + 100.0, "ctx": 9000});
        // This computer's clock is 25 s behind the server's: cold_at lands 25 s earlier here, still 100 s away.
        let s = one(w.clone(), N, N - 25.0);
        assert_eq!(s.cache_until, N + 75.0);
        assert_eq!(live(&s, N - 25.0, true, None).left, 100);
        // Ahead by an hour: the same 100 s.
        let s = one(w, N, N + 3600.0);
        assert_eq!(live(&s, N + 3600.0, true, None).left, 100);
        assert_eq!(live(&s, N + 3700.0, true, None).kind, LiveKind::Cold, "warm flips to cold on its own");
    }

    #[test]
    fn staleness_greys_at_45_seconds() {
        let s = one(json!({"sid": "a", "activity": "quiet", "cache": "warm", "cold_at": N + 900.0, "ctx": 9000}), N, N);
        assert!(live(&s, N + 40.0, true, None).stale.is_none());
        let l = live(&s, N + 50.0, true, None);
        assert_eq!((l.stale.as_deref(), l.left, l.detail.as_str()), (Some("as of 52s ago"), 0, "as of 52s ago"));
        assert_eq!(live(&s, N + 1.0, false, None).stale.as_deref(), Some("as of 3s ago"), "the link is down");
        assert_eq!(as_of_text(130.0), "as of 2m ago");
        assert_eq!(as_of_text(3.0 * 86400.0), "as of 3d ago");
    }

    #[test]
    fn the_board_and_the_terminal_agree() {
        // Cold needs 150k for the board's Cold column, but every view calls a small cold chat cold.
        let small =
            one(json!({"sid": "a", "activity": "quiet", "cache": "cold", "cold_at": N - 60.0, "ctx": 41000}), N, N);
        assert_eq!(
            (small.lamp(N), live(&small, N, true, None).kind, live(&small, N, true, None).pill),
            (Lamp::Idle, LiveKind::Cold, Some("cold"))
        );
        let big =
            one(json!({"sid": "a", "activity": "quiet", "cache": "cold", "cold_at": N - 60.0, "ctx": 300000}), N, N);
        assert_eq!(big.lamp(N), Lamp::Cold);
        // Stalled says stalled on the card, not working.
        let st =
            one(json!({"sid": "a", "activity": "stalled", "status": "busy", "cache": "unknown", "ctx": 1000}), N, N);
        assert_eq!((st.lamp(N), live(&st, N, true, None).pill), (Lamp::Stall, Some("stalled")));
        // Output never overrides a need.
        let w =
            one(json!({"sid": "a", "activity": "needs", "status": "waiting", "cache": "unknown", "ctx": 1000}), N, N);
        assert_eq!(live(&w, N, true, Some(N - 0.5)).word, "Needs you");
    }

    #[test]
    fn unknown_cache_is_never_a_guess() {
        let s = one(
            json!({"sid": "a", "activity": "quiet", "cache": "unknown", "active": N - 7200.0, "ctx": 200000}),
            N,
            N,
        );
        assert_eq!(s.cache_left(N), 0);
        assert!(!s.is_cold(N) && !s.needs_fresh(N));
        let l = live(&s, N, true, None);
        assert_eq!((l.word, l.pill, l.expensive.as_deref()), ("Quiet", Some("cache ?"), Some("expensive")));
    }

    #[test]
    fn small_helpers() {
        assert_eq!(clock(3725), "1:02:05");
        assert_eq!(span(45), "45s");
        assert_eq!(span(3900), "1h 05m");
        assert_eq!(ctx_label(98_000, false), "98k of 300k context");
        assert_eq!(ctx_label(98_000, true), "98k / 300k");
        assert_eq!(ctx_label(550_000, false), "550k context, past 300k");
    }
}
