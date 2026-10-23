//! The board as the UI sees it: owned, plain data with everything worked out once per refresh.
//! Time-dependent answers (lamp, cache left, phrases) take `now` so a once-a-second tick re-reads them for free.

use serde::{Deserialize, Serialize};

use crate::api::{ApiCost, ApiFlags, ApiMachine, ApiMem, ApiReaped, ApiReplaced, ApiSession, ApiSessions};
use crate::fmt;
use crate::names::{self, TagKind};

pub const GUARD: u64 = 150_000;
/// Under this a chat is small: going back is cheap even with a cold cache (agent/sb.py SMALL_CTX).
pub const SMALL: u64 = 60_000;
/// Plan numbers younger than this read as now; older ones say how old they are.
pub const LIMITS_FRESH: f64 = 600.0;
/// The five-hour window: weekly numbers older than this read "at least".
pub const FIVE_WINDOW: f64 = 5.0 * 3600.0;
pub const COMPACT: u64 = 300_000;
pub const TTL: u64 = 3600;
pub const STALE_CLOSED: f64 = 2.0 * 86400.0;
pub const STALE_OPEN: f64 = 7.0 * 86400.0;
/// Waiting longer than this, a chat is parked: a muted "parked" pill, and it no longer counts as needing you
/// (no badge, no notification). Replying or opening it clears it as usual.
pub const PARKED: f64 = 24.0 * 3600.0;
/// The most of a chat's need a card shows; the details sheet shows all of it.
pub const NEEDS_CARD: usize = 120;
pub const STALLED: f64 = 6.0 * 3600.0;
/// MB: above this the machine header says how much memory Claude is using (spec v2 section 2).
pub const MEM_WARN: f64 = 2000.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Lamp {
    /// A message is held by the context guard.
    Held,
    /// Claude asked something and is waiting.
    Wait,
    /// Says busy, nothing for hours.
    Stall,
    /// Working now.
    Run,
    /// Cache still warm: cheap to reply.
    Warm,
    /// Big and expired: replying re-reads everything.
    Cold,
    Idle,
}

impl Lamp {
    pub fn word(self) -> &'static str {
        match self {
            Lamp::Run => "Working",
            Lamp::Warm => "Warm",
            Lamp::Held => "Held",
            Lamp::Wait => "Waiting",
            Lamp::Stall => "Stalled",
            Lamp::Cold => "Cold",
            Lamp::Idle => "Idle",
        }
    }
    /// Sort rank, most urgent first.
    pub fn rank(self) -> u8 {
        match self {
            Lamp::Held => 0,
            Lamp::Wait => 1,
            Lamp::Run => 2,
            Lamp::Stall => 3,
            Lamp::Warm => 4,
            Lamp::Cold => 5,
            Lamp::Idle => 6,
        }
    }
    pub fn needs_you(self) -> bool {
        matches!(self, Lamp::Held | Lamp::Wait)
    }
}

/// What a chat is doing, as the server's derive() decided (docs/specs/2026-10-14-live-state.md), most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Activity {
    /// A message is held for it by the guard.
    Held,
    /// Claude waits on a permission prompt or a question.
    Needs,
    /// Needs, for more than a day: muted, out of the badges.
    Parked,
    /// A turn is open.
    Working,
    /// Working with no progress for more than 6 hours.
    Stalled,
    /// The turn is done.
    Quiet,
}

impl Activity {
    pub fn parse(s: &str) -> Option<Activity> {
        Some(match s {
            "held" => Activity::Held,
            "needs" => Activity::Needs,
            "parked" => Activity::Parked,
            "working" => Activity::Working,
            "stalled" => Activity::Stalled,
            "quiet" => Activity::Quiet,
            _ => return None,
        })
    }
}

/// The prompt cache, as the server knows it: warm until `cold_at`, cold after, or unknown (never a guess).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Cache {
    Warm,
    Cold,
    Unknown,
}

/// Machine data older than this (or a dropped link to the server) greys every view: "as of 2m ago".
pub const LIVE_STALE: f64 = 45.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Closed,
    Terminal,
    Background,
}

/// A chat's size, in three bands: what going back costs once the cache is cold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Band {
    /// Under 60k: going back is cheap.
    Small,
    /// 60k to 150k: resuming re-reads it all; Start fresh is cheaper if the history is not needed.
    Medium,
    /// 150k and up: the guard holds messages once it is cold.
    Big,
}

impl Band {
    pub fn of(ctx: u64) -> Band {
        if ctx < SMALL {
            Band::Small
        } else if ctx < GUARD {
            Band::Medium
        } else {
            Band::Big
        }
    }
    /// Always with the actual size: "145k, resuming re-reads 145k; Start fresh is cheaper if you do not need the
    /// history". A warm chat is cheap to reply to at any size. The same words as agent/sb.py band_words.
    pub fn words(ctx: u64, warm: bool) -> String {
        let k = fmt::k(ctx);
        if warm {
            return format!("{k}, reply while warm");
        }
        match Band::of(ctx) {
            Band::Small => format!("{k}, small, so going back is cheap"),
            Band::Medium => {
                format!("{k}, resuming re-reads {k}, so Start fresh is cheaper if you don't need the history")
            }
            Band::Big => format!("{k}, big: resuming re-reads all {k} at full price"),
        }
    }
}

/// Online state of a machine: on, offline, or answering with an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MachineLamp {
    On,
    Off,
    Warn,
}

/// The flags a session was started with, normalized from either wire form.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Flags {
    /// ultracode | max | xhigh | high | medium | low
    pub effort: Option<String>,
    pub chrome: bool,
    /// opus | sonnet | haiku | fable, when it differs from the machine default
    pub model: Option<String>,
    pub fast: bool,
    /// plan | acceptEdits | bypassPermissions | dontAsk | default | auto
    pub perm: Option<String>,
    pub agent: Option<String>,
    pub remote: bool,
}

impl Flags {
    /// How many launch flags it runs with, counting the model (always set), as the board's flag button shows it.
    pub fn count(&self) -> usize {
        1 + [self.effort.is_some(), self.chrome, self.fast, self.perm.is_some(), self.agent.is_some(), self.remote]
            .iter()
            .filter(|b| **b)
            .count()
    }

    pub fn from_api(f: Option<&ApiFlags>) -> Flags {
        match f {
            Some(ApiFlags::Object(o)) => Flags {
                effort: o.effort.clone().filter(|s| !s.is_empty()),
                chrome: o.chrome.unwrap_or(false),
                model: o.model.clone().filter(|s| !s.is_empty()),
                fast: o.fast.unwrap_or(false),
                perm: o.perm.clone().filter(|s| !s.is_empty()),
                agent: o.agent.clone().filter(|s| !s.is_empty()),
                remote: o.remote.unwrap_or(false),
            },
            Some(ApiFlags::List(ids)) => {
                let mut f = Flags::default();
                for id in ids {
                    match id.as_str() {
                        "ultracode" | "max" | "xhigh" | "high" | "medium" | "low" => f.effort = Some(id.clone()),
                        "plan" => f.perm = Some("plan".into()),
                        "bypass" | "bypassPermissions" => f.perm = Some("bypassPermissions".into()),
                        "chrome" => f.chrome = true,
                        "remote" => f.remote = true,
                        "fast" => f.fast = true,
                        _ => {}
                    }
                }
                f
            }
            _ => Flags::default(),
        }
    }
}

/// One chip on a row or in the inspector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chip {
    pub id: String,
    pub short: String,
    pub long: String,
    /// Icon name from the shared stroke set (spark, gauge, cube, plan, unlock, globe, remote, bg, term, bolt, check, user).
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub machine: String,
    pub machine_label: String,
    pub sid: String,
    /// "<machine>:<sid>", unique across the fleet.
    pub key: String,
    /// The raw title as the server sent it.
    pub raw_title: String,
    /// Title without the stamp, first line only.
    pub title: String,
    pub topic: String,
    pub tag: Option<String>,
    pub tag_kind: Option<TagKind>,
    /// Folder relative to the machine's Claude folder, or a label for it, as people read it ("~/Documents/site",
    /// "iCloud Drive/Notes", "Home folder").
    pub folder: String,
    /// The folder's address as the server sent it ("" is the Claude folder, `~/...` and `@...` outside it), for
    /// matching and for sending back. Empty from an older server that sent no folder.
    pub folder_addr: String,
    pub folder_short: String,
    pub project: String,
    pub active: f64,
    pub ctx: u64,
    /// "Opus 5.5"
    pub model: String,
    pub kind: Kind,
    pub status: Option<String>,
    pub state: Option<String>,
    pub alive: bool,
    pub held: bool,
    /// The machine this chat's Chrome work goes to (Auto, Start fresh, Continue); None = its own machine.
    pub chrome_machine: Option<String>,
    pub url: Option<String>,
    pub bg_id: Option<String>,
    pub handoff: bool,
    pub repeats: u32,
    pub stalled: bool,
    pub waiting: bool,
    pub stale: bool,
    /// What it is doing, from the server (or worked out from an older server's status).
    pub activity: Activity,
    /// The cache as the server last knew it. Warm turns cold here once `cache_until` passes.
    pub cache: Cache,
    /// When the prompt cache goes cold, on this computer's clock (the server's `cold_at` corrected for clock skew);
    /// 0 when not known.
    pub cache_until: f64,
    /// When its machine last answered the server, on this computer's clock (0: not known). Older than LIVE_STALE,
    /// every view greys its state.
    #[serde(default)]
    pub as_of: f64,
    pub flags: Flags,
    /// v2: marked Keep running (the reaper leaves it alone).
    pub keep: bool,
    /// Starred: its tab sits at the left and is restored on launch even when the chat has stopped.
    pub star: bool,
    /// Asked for Claude in Chrome, but Claude Code started the chat without it.
    #[serde(default)]
    pub chrome_missing: bool,
    /// The tab group its thread is in (an id of `Snapshot::groups`). Kept in step with `groups` by `Snapshot::regroup`.
    #[serde(default)]
    pub group: Option<String>,
    /// v2: the reaper stopped it.
    pub reaped: Option<Reaped>,
    /// v2: the root chat of its thread and how many older chats the thread has before this one.
    pub thread: Option<String>,
    pub history: u32,
    /// A current summary is written: Start fresh costs no summary run.
    pub handoff_fresh: bool,
    /// Resume vs Start fresh as the agent worked it out (agent/sb.py cost_model), when the agent is new enough.
    pub cost: Option<ApiCost>,
    /// Library tools this chat runs with (empty on older servers).
    #[serde(default)]
    pub tools: Vec<String>,
    /// "auto" or "pick": how its tools were chosen (None on older servers).
    #[serde(default)]
    pub tools_mode: Option<String>,
    /// What a waiting chat wants from you ("confirm option 1 or 2"), from its background job. None on older servers.
    #[serde(default)]
    pub needs: Option<String>,
    /// The answer the job suggests. Kept for older clients; this app answers through `ask` now.
    #[serde(default)]
    pub suggested_reply: Option<String>,
    /// What it needs, shaped by kind, as the agent built it (None on older servers: `ask_now` falls back to `needs`).
    #[serde(default)]
    pub ask: Option<crate::ask::Ask>,
}

/// Why and when the reaper stopped a chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reaped {
    pub at: f64,
    /// idle | stalled | replaced
    pub reason: String,
    pub idle_h: f64,
}

impl Reaped {
    fn from_api(r: Option<&ApiReaped>) -> Option<Reaped> {
        let r = r?;
        Some(Reaped {
            at: r.at.unwrap_or(0.0),
            reason: r.reason.clone().unwrap_or_else(|| "idle".into()),
            idle_h: r.idle_h.unwrap_or(0.0),
        })
    }
    /// "Stopped after 26 h idle to free memory. Resume continues it."
    pub fn text(&self) -> String {
        let h = self.idle_h;
        let span = if h >= 48.0 {
            format!("{} days", (h / 24.0).round() as u64)
        } else {
            format!("{} h", h.round().max(1.0) as u64)
        };
        match self.reason.as_str() {
            "stalled" => format!("Stopped after {span} stuck on one step, to free memory. Resume continues it."),
            "replaced" => "Stopped to free memory once a newer chat took over. Resume continues it.".into(),
            _ => format!("Stopped after {span} idle to free memory. Resume continues it."),
        }
    }
}

impl Session {
    pub fn background(&self) -> bool {
        self.kind == Kind::Background
    }
    fn busy(&self) -> bool {
        self.activity == Activity::Working
    }
    pub fn heavy(&self) -> bool {
        self.ctx >= GUARD
    }
    pub fn band(&self) -> Band {
        Band::of(self.ctx)
    }
    /// The size and what going back costs, for the cache as it is now.
    pub fn band_words(&self, now: f64) -> String {
        Band::words(self.ctx, self.cache_left(now) > 0 || self.lamp(now) == Lamp::Run)
    }
    /// "Start fresh: about 91k (summary 79k + new chat 12k). Resume: about 290k, because the cache is cold..." for
    /// the cache as it is this second, so it changes the moment the cache cools.
    pub fn cost_line(&self, now: f64) -> Option<String> {
        let c = self.cost.as_ref()?;
        let t = if self.cache_left(now) > 0 { &c.text.warm } else { &c.text.cold };
        Some(t.trim().to_string()).filter(|t| !t.is_empty())
    }
    /// What "How is this estimated?" opens: the inputs, the ratios, and that plan weighting is an estimate.
    pub fn cost_how(&self) -> Option<String> {
        self.cost.as_ref().map(|c| c.how.trim().to_string()).filter(|t| !t.is_empty())
    }
    /// The cheaper choice by the cost model, for the cache as it is now.
    pub fn cost_pick(&self, now: f64) -> Option<crate::actions::Pick> {
        let c = self.cost.as_ref()?;
        let p = if self.cache_left(now) > 0 { &c.pick.warm } else { &c.pick.cold };
        match p.as_str() {
            "fresh" => Some(crate::actions::Pick::Fresh),
            "resume" => Some(crate::actions::Pick::Resume),
            _ => None,
        }
    }
    pub fn over(&self) -> bool {
        self.ctx >= COMPACT
    }
    /// The details sheet's "Tools: Auto (5)" or "Tools: 3 picked"; None when the server does not say how.
    pub fn tools_word(&self) -> Option<String> {
        match self.tools_mode.as_deref() {
            Some("auto") => Some(format!("Auto ({})", self.tools.len())),
            Some("pick") => Some(format!("{} picked", self.tools.len())),
            _ => None,
        }
    }
    pub fn ctx_label(&self) -> String {
        fmt::k(self.ctx)
    }
    pub fn ctx_frac(&self) -> f64 {
        (self.ctx as f64 / COMPACT as f64).min(1.0)
    }
    pub fn cache_left(&self, now: f64) -> u64 {
        if self.cache_until > 0.0 { (self.cache_until - now).round().max(0.0) as u64 } else { 0 }
    }
    pub fn cache_frac(&self, now: f64) -> f64 {
        (self.cache_left(now) as f64 / TTL as f64).min(1.0)
    }
    /// The cache this second: warm turns cold the moment `cache_until` passes, on every view at once.
    pub fn cache_now(&self, now: f64) -> Cache {
        match self.cache {
            Cache::Warm | Cache::Cold if self.cache_until > 0.0 => {
                if self.cache_until > now {
                    Cache::Warm
                } else {
                    Cache::Cold
                }
            }
            c => c,
        }
    }
    /// Known to be cold: the next message re-reads the whole context.
    pub fn is_cold(&self, now: f64) -> bool {
        self.cache_now(now) == Cache::Cold
    }
    /// The board's lamp: the server's activity, and for a quiet chat its cache. Cold is the column for big chats
    /// gone cold (going back is costly); a small cold chat sits with the quiet ones, its pill still says cold.
    /// The words every view shows come from `live::live`, which reads the same fields.
    pub fn lamp(&self, now: f64) -> Lamp {
        match self.activity {
            Activity::Held => Lamp::Held,
            Activity::Needs | Activity::Parked => Lamp::Wait,
            Activity::Stalled => Lamp::Stall,
            Activity::Working => Lamp::Run,
            Activity::Quiet => match self.cache_now(now) {
                Cache::Warm => Lamp::Warm,
                Cache::Cold if self.ctx >= GUARD => Lamp::Cold,
                _ => Lamp::Idle,
            },
        }
    }
    /// Its machine's data is older than LIVE_STALE (`link`: the app's own link to the server is up).
    pub fn stale_data(&self, now: f64, link: bool) -> bool {
        !link || (self.as_of > 0.0 && now - self.as_of > LIVE_STALE)
    }
    pub fn word(&self, now: f64) -> &'static str {
        if self.reaped_note() && self.lamp(now) == Lamp::Idle { "Stopped" } else { self.lamp(now).word() }
    }
    /// The reaper stopped it and nothing has used it since: say so instead of the usual words.
    pub fn reaped_note(&self) -> bool {
        self.reaped.is_some() && !self.alive && !self.held
    }
    /// v2: can it be marked Keep running? Only a running background chat can be stopped by the reaper.
    pub fn can_keep(&self) -> bool {
        self.background() && self.alive
    }
    pub fn needs_fresh(&self, now: f64) -> bool {
        !self.held && !self.busy() && self.is_cold(now) && self.ctx >= GUARD
    }
    /// What to recommend first: a fresh start instead of going back.
    pub fn recommend_fresh(&self, now: f64) -> bool {
        self.held || self.lamp(now) == Lamp::Cold || self.needs_fresh(now)
    }

    /// Waiting for an answer for more than a day (PARKED): still in the Needs you column, but muted, and it no
    /// longer counts toward the badge or notifications. A held message is never parked.
    pub fn parked(&self, _now: f64) -> bool {
        self.activity == Activity::Parked
    }

    /// Needs you now: held, or waiting and not parked. What the badge, the counts and notifications go by.
    pub fn needs_you(&self, now: f64) -> bool {
        self.lamp(now).needs_you() && !self.parked(now)
    }

    /// Whole days it has been waiting (at least 1).
    pub fn wait_days(&self, now: f64) -> u64 {
        (((now - self.active) / 86400.0).floor() as u64).max(1)
    }

    /// What a waiting chat says about itself: "Needs: <need>" while it is fresh, "Waiting 2 days: <need>" once it is
    /// parked ("Waiting 2 days" with no need known), or None when the server sent no need and it is not parked
    /// (then the usual words stand). `max` clips the need, for cards (None = all of it).
    pub fn wait_line(&self, now: f64, max: Option<usize>) -> Option<String> {
        if self.held || self.lamp(now) != Lamp::Wait {
            return None;
        }
        let need = self.needs.as_deref().map(|n| match max {
            Some(m) => clip(n, m),
            None => n.to_string(),
        });
        if self.parked(now) {
            let d = self.wait_days(now);
            let head = format!("Waiting {d} {}", if d == 1 { "day" } else { "days" });
            return Some(match need {
                Some(n) => format!("{head}: {n}"),
                None => head,
            });
        }
        need.map(|n| format!("Needs: {n}"))
    }

    /// What to answer while it waits (not held): the agent's ask, else an open reply about its need. None otherwise.
    pub fn ask_now(&self, now: f64) -> Option<crate::ask::Ask> {
        if self.held || self.lamp(now) != Lamp::Wait {
            return None;
        }
        Some(self.ask.clone().unwrap_or_else(|| crate::ask::Ask::open(self.needs.as_deref())))
    }

    /// The one short phrase a row says about itself.
    pub fn phrase(&self, now: f64) -> String {
        let l = self.lamp(now);
        match l {
            Lamp::Warm => format!("Warm, {} left", fmt::mins(self.cache_left(now))),
            Lamp::Cold => {
                if self.handoff {
                    "Cold, handoff ready".into()
                } else {
                    "Cold, start fresh".into()
                }
            }
            Lamp::Stall => format!("Stalled {}", fmt::sago(self.active, now)),
            Lamp::Idle if self.reaped_note() => "Stopped to free memory".into(),
            Lamp::Idle => {
                if self.alive && !self.background() {
                    "Open, idle".into()
                } else {
                    "Idle".into()
                }
            }
            Lamp::Held => "Message held".into(),
            Lamp::Wait => self.wait_line(now, Some(NEEDS_CARD)).unwrap_or_else(|| "Waiting for you".into()),
            Lamp::Run => "Working".into(),
        }
    }

    /// One plain sentence about what to do, as runs of text where `bold` marks the key words.
    pub fn explain(&self, now: f64) -> Vec<TextRun> {
        let l = self.lamp(now);
        let kk = self.ctx_label();
        let mut r = Runs::default();
        if let (true, Some(rp)) = (self.reaped_note(), &self.reaped) {
            r.t(&rp.text());
        } else if self.ctx == 0 && matches!(l, Lamp::Run | Lamp::Warm | Lamp::Idle) {
            r.t(if l == Lamp::Run {
                "Starting. Claude is working on the first reply."
            } else {
                "No replies yet, so there's nothing to re-read."
            });
        } else if self.held {
            r.t("Your last message is held because going back would re-read ")
                .b(&kk)
                .t(" tokens. ")
                .b("Start fresh")
                .t(" carries your notes and that message into a new chat.");
        } else if l == Lamp::Wait {
            r.t("Claude asked something and is waiting for your answer");
            if self.url.is_some() {
                r.t(". ").b("Open in Claude").t(" to reply.");
            } else if self.background() {
                r.t(&format!(", on {}.", self.machine_label));
            } else {
                r.t(&format!(", in the terminal on {}.", self.machine_label));
            }
        } else if l == Lamp::Stall {
            let quiet = fmt::ago(self.active, now);
            let quiet = quiet.strip_suffix(" ago").unwrap_or(&quiet).to_string();
            r.t(&format!(
                "It says it's working, but nothing's happened for {quiet}. It might be stuck, so open it to check{}.",
                if self.background() && self.alive { ", or stop it" } else { "" }
            ));
        } else if l == Lamp::Cold {
            r.t("The cache expired, so replying here re-reads all ")
                .b(&kk)
                .t(" tokens. ")
                .b("Start fresh")
                .t(" writes a summary and opens a new chat with just your notes.");
        } else if l == Lamp::Run {
            r.t("Working now. The cache stays warm while it runs.");
        } else if l == Lamp::Warm && self.heavy() {
            r.t("Replies stay cheap for ")
                .b(&fmt::mins(self.cache_left(now)))
                .t(". After that, start fresh instead of going back.");
        } else if l == Lamp::Warm && self.band() == Band::Medium {
            r.t("Replies stay cheap for ")
                .b(&fmt::mins(self.cache_left(now)))
                .t(". After that, resuming re-reads all ")
                .b(&kk)
                .t(", so Start fresh is cheaper if you don't need the history.");
        } else if l == Lamp::Warm {
            r.t("Replies stay cheap for ")
                .b(&fmt::mins(self.cache_left(now)))
                .t(&format!(", and at {kk} it stays cheap to come back to later."));
        } else if self.cache_now(now) == Cache::Unknown && self.band() == Band::Medium {
            r.t("Not sure yet how long its cache stays warm. Once it's cold, resuming re-reads all ").b(&kk).t(".");
        } else if self.cache_now(now) == Cache::Unknown && self.band() == Band::Big {
            r.t("Not sure yet how long its cache stays warm. Once it's cold, going back re-reads all ")
                .b(&kk)
                .t(" at full price.");
        } else if self.cache_now(now) == Cache::Unknown {
            r.t(&format!("Not sure yet how long its cache stays warm, but at {kk} going back is cheap."));
        } else if self.band() == Band::Medium {
            r.t("The cache expired, so resuming re-reads all ")
                .b(&kk)
                .t(". ")
                .b("Start fresh")
                .t(" is cheaper if you don't need the history.");
        } else {
            r.t(&format!("The cache expired, but at {kk} going back is cheap."));
        }
        r.0
    }

    pub fn cache_text(&self, now: f64) -> String {
        if self.lamp(now) == Lamp::Run {
            "Warm while it works".into()
        } else if self.cache_left(now) > 0 {
            format!(
                "Warm for {}, until {}",
                fmt::mins(self.cache_left(now)),
                fmt::when(now + self.cache_left(now) as f64, now)
            )
        } else if self.cache_now(now) == Cache::Unknown {
            "Not known yet".into()
        } else if self.cache_until > 0.0 {
            format!("Expired {}", fmt::ago(self.cache_until, now))
        } else {
            "Expired".into()
        }
    }

    pub fn kind_text(&self) -> String {
        let k = match self.kind {
            Kind::Background => "Background",
            Kind::Terminal => "Terminal",
            Kind::Closed => "Closed",
        };
        if self.stalled { format!("{k}, stalled") } else { k.into() }
    }

    /// Chips: what it was started with and what is on. `full` adds remote control and the kind (inspector).
    /// Fixed order so a row always reads the same way: effort, model, speed, permissions, agent, tools, kind.
    pub fn chips(&self, full: bool) -> Vec<Chip> {
        let mut out = Vec::new();
        let f = &self.flags;
        let chip = |id: &str, short: &str, long: &str, icon: &str| Chip {
            id: id.into(),
            short: short.into(),
            long: long.into(),
            icon: icon.into(),
        };
        if let Some(e) = &f.effort {
            let said = (self.tag_kind == Some(TagKind::Ultra) && e == "ultracode")
                || (self.tag_kind == Some(TagKind::Max) && e == "max");
            if !said {
                out.push(match e.as_str() {
                    "ultracode" => chip("ultracode", "UltraCode", "UltraCode", "spark"),
                    "max" => chip("max", "Max", "Max effort", "gauge"),
                    "xhigh" => chip("xhigh", "xhigh", "xhigh effort", "gauge"),
                    "high" => chip("effort", "High", "High effort", "gauge"),
                    "medium" => chip("effort", "Medium", "Medium effort", "gauge"),
                    "low" => chip("effort", "Low", "Low effort", "gauge"),
                    other => chip("effort", other, other, "gauge"),
                });
            }
        }
        let fam = ["Sonnet", "Haiku", "Fable"].into_iter().find(|p| self.model.starts_with(p));
        if let Some(m) = &f.model {
            let long = if self.model.to_lowercase().starts_with(m.as_str()) { self.model.clone() } else { cap(m) };
            out.push(chip("model", &cap(m), &long, "cube"));
        } else if let Some(fam) = fam {
            out.push(chip("model", fam, &self.model, "cube"));
        }
        if f.fast {
            out.push(chip("fast", "Fast", "Fast mode", "bolt"));
        }
        if let Some(p) = &f.perm {
            out.push(match p.as_str() {
                "plan" => chip("plan", "Plan", "Plan mode", "plan"),
                "bypassPermissions" => chip("bypass", "Bypass", "Skips permissions", "unlock"),
                "acceptEdits" => chip("edits", "Edits", "Accept edits", "check"),
                "dontAsk" => chip("dontask", "Never asks", "Never asks", "unlock"),
                "default" => chip("asks", "Asks", "Asks first", "lock"),
                "auto" => chip("auto", "Auto", "Auto mode", "spark"),
                other => chip("perm", other, other, "lock"),
            });
        }
        if let Some(a) = &f.agent {
            out.push(chip("agent", a, &format!("Agent {a}"), "user"));
        }
        if f.chrome {
            out.push(chip("chrome", "Chrome", "Claude in Chrome", "globe"));
        }
        if full && f.remote {
            out.push(chip("remote", "Remote", "Remote control", "remote"));
        }
        if full {
            match self.kind {
                Kind::Background => out.push(chip("bg", "Background", "Background", "bg")),
                Kind::Terminal => out.push(chip("term", "Terminal", "Terminal", "term")),
                Kind::Closed => {}
            }
        }
        out
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// A run of text inside a sentence; `bold` marks the key words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextRun {
    pub text: String,
    pub bold: bool,
}

#[derive(Default)]
struct Runs(Vec<TextRun>);
impl Runs {
    fn t(&mut self, s: &str) -> &mut Self {
        self.0.push(TextRun { text: s.into(), bold: false });
        self
    }
    fn b(&mut self, s: &str) -> &mut Self {
        self.0.push(TextRun { text: s.into(), bold: true });
        self
    }
}

pub fn plain_text(runs: &[TextRun]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Replaced {
    pub machine: String,
    pub machine_label: String,
    pub sid: String,
    pub key: String,
    pub title: String,
    pub topic: String,
    pub tag: Option<String>,
    pub tag_kind: Option<TagKind>,
    pub folder: String,
    pub active: f64,
    /// fresh | resumed | cleared | renamed | same-name | continued
    pub how: Option<String>,
    pub by_sid: Option<String>,
    pub by_machine: Option<String>,
    pub kind: Kind,
    pub alive: bool,
    pub url: Option<String>,
    pub bg_id: Option<String>,
    pub reaped: Option<Reaped>,
}

impl Replaced {
    pub fn background(&self) -> bool {
        self.kind == Kind::Background
    }
    /// The key of the chat that replaced it, if the server said.
    pub fn newer_key(&self) -> Option<String> {
        let sid = self.by_sid.as_ref()?;
        Some(format!("{}:{}", self.by_machine.as_deref().unwrap_or(&self.machine), sid))
    }
    /// "Started fresh", "Continued on Laptop", ...
    pub fn how_text(&self, label_of: impl Fn(&str) -> Option<String>) -> String {
        match self.how.as_deref() {
            Some("fresh") => "Started fresh".into(),
            Some("resumed") => "Resumed in a newer copy".into(),
            Some("cleared") => "Cleared, so it started over".into(),
            Some("renamed") => "Renamed".into(),
            Some("same-name") => "A newer chat has the same name".into(),
            Some("same-topic") => "A newer chat picked up the same topic".into(),
            Some("handoff") => "A newer chat started from its handoff".into(),
            Some("compacted") => "Compacted with its thread into a fresh chat".into(),
            Some("linked") => "You linked a newer chat to it".into(),
            Some("continued") => match &self.by_machine {
                Some(m) => format!("Continued on {}", label_of(m).unwrap_or_else(|| m.clone())),
                None => "Continued on another machine".into(),
            },
            _ => "Replaced by a newer session".into(),
        }
    }
    /// v2: a thread link (not a rename or a resume), so "Not the same topic" can split it off.
    pub fn can_split(&self) -> bool {
        matches!(self.how.as_deref(), Some("same-topic" | "linked" | "handoff"))
    }
    pub fn reaped_note(&self) -> bool {
        self.reaped.is_some() && !self.alive
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub name: String,
    pub label: String,
    pub os: Option<String>,
    pub online: Option<bool>,
    pub error: Option<String>,
    pub updated: Option<f64>,
    /// The Claude folder's absolute path, as the machine reports it.
    pub root: String,
    /// "Home folder" or the Claude folder's last segment.
    pub root_label: String,
    pub sessions: Vec<Session>,
    pub replaced: Vec<Replaced>,
    /// v2: Claude's memory on it.
    pub mem: Option<Mem>,
    /// v2: what it can do, one line ("iOS, Android SDK, CUDA (RTX GPU)"); None when its caps are off.
    pub can: Option<String>,
    /// v2: it shares what it can do, so Auto may pick it.
    pub has_caps: bool,
    /// v2: what its agent can do ("browse", "rename", "summary", "flags", ...).
    #[serde(default)]
    pub features: Vec<String>,
    /// v2: Claude in Chrome there.
    #[serde(default)]
    pub chrome: Option<crate::api::ApiChrome>,
    /// v2: the machine Chrome sessions go to first.
    #[serde(default)]
    pub chrome_pref: bool,
    /// Setup problems its agent found.
    #[serde(default)]
    pub setup: Vec<crate::api::ApiSetupIssue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mem {
    pub claude_mb: f64,
    pub procs: u32,
    /// Leftover claude processes nothing is using, and their memory.
    pub orphans: u32,
    pub orphan_mb: f64,
}

impl Mem {
    fn from_api(m: Option<&ApiMem>) -> Option<Mem> {
        let m = m?;
        Some(Mem {
            claude_mb: m.claude_mb.unwrap_or(0.0).max(0.0),
            procs: m.procs.unwrap_or(0.0).max(0.0) as u32,
            orphans: m.orphans.len() as u32,
            orphan_mb: m.orphans.iter().map(|o| o.mb.unwrap_or(0.0).max(0.0)).sum(),
        })
    }
}

impl Machine {
    /// "Claude is using 3.1 GB on Studio, 410 MB of it in a process nothing is using", only above 2 GB.
    pub fn mem_line(&self) -> Option<String> {
        let m = self.mem.as_ref().filter(|m| m.claude_mb > MEM_WARN)?;
        let mut out = format!("Claude is using {} on {}", fmt::mem(m.claude_mb), self.label);
        if m.orphans > 0 {
            let what = if m.orphans == 1 { "a process".to_string() } else { format!("{} processes", m.orphans) };
            out.push_str(&format!(", {} of it in {what} nothing is using", fmt::mem(m.orphan_mb)));
        }
        Some(out)
    }
    /// "3.1 GB" for the machine's row in a list, only above 2 GB.
    pub fn mem_short(&self) -> Option<String> {
        self.mem.as_ref().filter(|m| m.claude_mb > MEM_WARN).map(|m| fmt::mem(m.claude_mb))
    }
    pub fn lamp(&self) -> MachineLamp {
        match self.online {
            Some(false) => MachineLamp::Off,
            None => MachineLamp::Warn,
            Some(true) if self.error.is_some() => MachineLamp::Warn,
            Some(true) => MachineLamp::On,
        }
    }
    pub fn is_offline(&self) -> bool {
        self.online == Some(false)
    }
}

/// Plan usage: the weekly and five-hour windows.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub week: Option<f64>,
    pub week_projected: Option<f64>,
    pub week_reset: Option<f64>,
    pub five_hour: Option<f64>,
    pub five_hour_reset: Option<f64>,
    /// v2: the server's status note (server/notes.py). The apps never write advice of their own.
    #[serde(default)]
    pub note: Option<crate::api::ApiNote>,
    /// When a session last saw these numbers. Usage in the Claude apps does not reach Switchboard until a
    /// session replies, so old numbers say how old they are.
    #[serde(default)]
    pub updated: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tone {
    Ok,
    Warn,
    Bad,
}

impl Plan {
    pub fn week_tone(&self) -> Tone {
        let used = self.week.unwrap_or(0.0);
        match self.week_projected {
            None => {
                if used >= 80.0 {
                    Tone::Bad
                } else if used >= 50.0 {
                    Tone::Warn
                } else {
                    Tone::Ok
                }
            }
            Some(p) => {
                if p > 100.0 {
                    Tone::Bad
                } else if p > 85.0 {
                    Tone::Warn
                } else {
                    Tone::Ok
                }
            }
        }
    }
    pub fn five_tone(&self) -> Tone {
        let f = self.five_hour.unwrap_or(0.0);
        if f >= 80.0 {
            Tone::Bad
        } else if f >= 60.0 {
            Tone::Warn
        } else {
            Tone::Ok
        }
    }
    pub fn five_live(&self, now: f64) -> bool {
        self.five_hour_reset.map(|r| r > now).unwrap_or(false)
    }
    /// Seconds since a session last saw the numbers, when known.
    pub fn limits_age(&self, now: f64) -> Option<f64> {
        self.updated.filter(|u| *u > 0.0).map(|u| (now - u).max(0.0))
    }
    /// Older than ten minutes: shown with "as of" and at reduced emphasis.
    pub fn limits_old(&self, now: f64) -> bool {
        self.limits_age(now).is_some_and(|a| a >= LIMITS_FRESH)
    }
    /// "as of 40m ago", or "as of 3:10pm" past an hour; None while the numbers are fresh.
    pub fn as_of(&self, now: f64) -> Option<String> {
        let age = self.limits_age(now).filter(|a| *a >= LIMITS_FRESH)?;
        let u = self.updated?;
        Some(if age < 3600.0 {
            format!("as of {} ago", fmt::sago(u, now))
        } else {
            format!("as of {}", fmt::when(u, now))
        })
    }
    fn week_reset_passed(&self, now: f64) -> bool {
        self.week_reset.is_some_and(|r| r <= now)
    }
    /// The week as it may be shown: "81%"; "at least 81%" once the number is older than the five-hour window (use
    /// since then only adds); "--" when the week has reset since (the status line zeroes it there).
    pub fn week_label(&self, now: f64) -> String {
        match self.week {
            None => "--".into(),
            Some(_) if self.week_reset_passed(now) => "--".into(),
            Some(w) if self.limits_age(now).is_some_and(|a| a > FIVE_WINDOW) => {
                format!("at least {}%", w.round() as i64)
            }
            Some(w) => format!("{}%", w.round() as i64),
        }
    }
    /// The five-hour window: "12%", or "--" once its reset has passed.
    pub fn five_label(&self, now: f64) -> String {
        match self.five_hour {
            None => "--".into(),
            Some(_) if self.five_hour_reset.is_some_and(|r| r <= now) => "--".into(),
            Some(f) => format!("{}%", f.round() as i64),
        }
    }
    /// The server's status note, when it has something to say.
    pub fn note_text(&self) -> Option<&str> {
        self.note.as_ref().map(|n| n.text.trim()).filter(|t| !t.is_empty())
    }
    /// Fraction of the week that has gone by.
    pub fn week_gone(&self, now: f64) -> Option<f64> {
        self.week_reset.map(|r| (1.0 - (r - now) / (7.0 * 86400.0)).clamp(0.0, 1.0))
    }
    pub fn five_gone(&self, now: f64) -> Option<f64> {
        self.five_hour_reset.filter(|r| *r > now).map(|r| (1.0 - (r - now) / (5.0 * 3600.0)).clamp(0.0, 1.0))
    }
}

/// Everything one refresh brought, normalized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub machines: Vec<Machine>,
    pub plan: Plan,
    /// Local time this data was fetched (seconds since the epoch).
    pub fetched_at: f64,
    /// The server's clock when it answered.
    pub server_now: Option<f64>,
    /// Build id of the server's web app.
    pub web: Option<String>,
    /// v2: the live-update version this snapshot is at.
    #[serde(default)]
    pub v: Option<u64>,
    /// v2: the cleanup card.
    #[serde(default)]
    pub cleanup: Option<crate::api::ApiCleanup>,
    /// Tab groups, shared by every Mac, in the tab row's order.
    #[serde(default)]
    pub groups: Vec<crate::groups::Group>,
    /// The usage budget (None on older servers).
    #[serde(default)]
    pub budget: Option<crate::api::ApiBudget>,
    /// The server's own addresses, for the saved list (empty from older servers).
    #[serde(default)]
    pub addresses: Vec<crate::api::ApiAddress>,
}

fn root_label(root: &str) -> String {
    let r = root.replace('\\', "/");
    let r = r.trim_end_matches('/');
    let segs: Vec<&str> = r.split('/').collect();
    let n = segs.len();
    if n >= 3 && (segs[n - 2] == "Users" || segs[n - 2] == "home") && !segs[n - 1].is_empty() {
        return "Home folder".into();
    }
    match segs.last() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => "Home folder".into(),
    }
}

fn is_home(cwd: &str) -> bool {
    let segs: Vec<&str> = cwd.split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        ["Users", _] | ["home", _] => cwd.starts_with('/'),
        [drive, "Users", _] => drive.len() == 2 && drive.ends_with(':'),
        _ => false,
    }
}

/// `s` cut to at most `max` characters at a word break, with "..." when anything was cut.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(3)).collect();
    let cut = match cut.rfind(char::is_whitespace) {
        Some(i) if i > cut.len() / 2 => cut[..i].to_string(),
        _ => cut,
    };
    format!("{}...", cut.trim_end_matches(|c: char| c.is_whitespace() || c == ',' || c == '.' || c == ':'))
}

/// A server text field cleaned up: one line of trimmed text, None when empty, at most `max` characters.
fn text_field(v: Option<&str>, max: usize) -> Option<String> {
    let t = v?.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() { None } else { Some(t.chars().take(max).collect()) }
}

fn folder_of(folder: Option<&str>, cwd: Option<&str>, root_lbl: &str) -> String {
    match folder {
        Some("") => root_lbl.to_string(),
        Some("~") | Some("~/") => "Home folder".into(),
        Some(f) => crate::places::shown(f, root_lbl),
        None => {
            let cwd = cwd.unwrap_or("").replace('\\', "/");
            let cwd = cwd.trim_end_matches('/');
            if cwd.is_empty() || is_home(cwd) {
                return "Home folder".into();
            }
            let segs: Vec<&str> = cwd.split('/').filter(|s| !s.is_empty()).collect();
            segs[segs.len().saturating_sub(2)..].join("/")
        }
    }
}

/// The project a folder belongs to: worktrees and common parents fold away, so
/// "switchboard/.claude/worktrees/x" is "switchboard" and "Developer/notebook-cli" is "notebook-cli".
pub fn project_of(folder: &str) -> String {
    let mut f = folder.replace('\\', "/");
    // Outside the Claude folder: "~/Documents/site" is in home, "@Data/work" in a place, "iCloud Drive/x" in iCloud.
    for top in ["~/", "iCloud Drive/"] {
        if let Some(rest) = f.strip_prefix(top) {
            f = rest.to_string();
        }
    }
    if f.starts_with('@')
        && let Some((_, rest)) = f.split_once('/')
    {
        f = rest.to_string();
    }
    if let Some(i) = f.find("/.claude/worktrees/") {
        f.truncate(i);
    }
    for p in [
        "Developer/",
        "Documents/Code/",
        "Documents/code/",
        "Documents/",
        "Downloads/",
        "Desktop/Claude/",
        "Desktop/",
        "code/",
        "Code/",
    ] {
        if let Some(rest) = f.strip_prefix(p) {
            f = rest.to_string();
            break;
        }
    }
    f.split('/').find(|s| !s.is_empty()).unwrap_or("").to_string()
}

fn kind_of(k: Option<&str>) -> Kind {
    match k {
        Some("background") => Kind::Background,
        Some("interactive") => Kind::Terminal,
        Some(_) => Kind::Terminal,
        None => Kind::Closed,
    }
}

fn alive(kind: Option<&str>, status: Option<&str>) -> bool {
    kind.is_some() && status != Some("stopped") && status != Some("done")
}

/// `fetched_at`: this computer's clock when the reply arrived. `offset`: add it to a server timestamp to get this
/// computer's clock (`fetched_at` minus the reply's `now`), so a countdown never mixes the two clocks.
pub fn session(s: &ApiSession, m: &ApiMachine, label: &str, root_lbl: &str, fetched_at: f64) -> Session {
    session_at(s, m, label, root_lbl, fetched_at, 0.0)
}

pub fn session_at(
    s: &ApiSession,
    m: &ApiMachine,
    label: &str,
    root_lbl: &str,
    fetched_at: f64,
    offset: f64,
) -> Session {
    let local = |t: Option<f64>| t.filter(|t| *t > 0.0).map(|t| t + offset);
    let ctx = s.ctx.unwrap_or(0.0).max(0.0) as u64;
    let mtime = local(s.mtime).unwrap_or(0.0);
    let active_raw = local(s.active).unwrap_or(0.0);
    let quiet = fetched_at - mtime.max(active_raw);
    let status = s.status.as_deref();
    let held = s.held.unwrap_or(false);
    // The server's activity; an older server sends only status and state, read the way it read them.
    let activity = match s.activity.as_deref().and_then(Activity::parse) {
        Some(a) => a,
        None => {
            let stalled = s.stalled.unwrap_or(status == Some("busy") && quiet > STALLED);
            let waiting = !stalled
                && (status == Some("waiting")
                    || (s.state.as_deref() == Some("blocked")
                        && status.is_some_and(|x| x != "busy" && x != "stopped")));
            let a = active_raw.max(if active_raw > 0.0 { 0.0 } else { mtime });
            if held {
                Activity::Held
            } else if waiting && a > 0.0 && fetched_at - a > PARKED {
                Activity::Parked
            } else if waiting {
                Activity::Needs
            } else if stalled {
                Activity::Stalled
            } else if status == Some("busy") || s.state.as_deref() == Some("running") {
                Activity::Working
            } else {
                Activity::Quiet
            }
        }
    };
    let (cache, cache_until) = match s.cache.as_deref() {
        Some("warm") | Some("cold") => {
            let until = local(s.cold_at).unwrap_or(0.0);
            (if s.cache.as_deref() == Some("warm") { Cache::Warm } else { Cache::Cold }, until)
        }
        Some(_) => (Cache::Unknown, 0.0),
        // An older server: seconds left as of the machine's answer; none left was cold (its agent always guessed).
        None => match s.cache_left.filter(|l| *l > 0.0) {
            Some(left) => (Cache::Warm, local(m.updated).unwrap_or(fetched_at) + left),
            None if s.cold == Some(false) => (Cache::Unknown, 0.0),
            None => (Cache::Cold, 0.0),
        },
    };
    let stalled = activity == Activity::Stalled;
    let open = s.kind.is_some() && status.is_some() && status != Some("stopped") && status != Some("done") && !stalled;
    let active = if active_raw > 0.0 {
        active_raw
    } else if mtime > 0.0 {
        mtime
    } else {
        fetched_at
    };
    let idle = fetched_at - active;
    let raw_title = s.title.clone().unwrap_or_default();
    let (topic, tag) = names::split(&raw_title);
    let folder = folder_of(s.folder.as_deref(), s.cwd.as_deref(), root_lbl);
    let project = {
        let p = project_of(if s.folder.as_deref() == Some("") { "" } else { &folder });
        if p.is_empty() { root_lbl.to_string() } else { p }
    };
    let waiting = matches!(activity, Activity::Needs | Activity::Parked);
    let repeats = match &s.repeats {
        Some(serde_json::Value::Number(n)) => n.as_f64().unwrap_or(0.0).max(0.0) as u32,
        Some(serde_json::Value::String(t)) => t.trim().parse::<u32>().unwrap_or(0),
        _ => 0,
    };
    let stale = !held && s.stale.unwrap_or(idle > if open { STALE_OPEN } else { STALE_CLOSED });
    Session {
        machine: m.name.clone(),
        machine_label: label.to_string(),
        sid: s.sid.clone(),
        key: format!("{}:{}", m.name, s.sid),
        title: names::plain(&raw_title),
        raw_title,
        tag_kind: names::tag_kind(tag.as_deref()),
        topic,
        tag,
        folder_short: if folder.starts_with("iCloud Drive") && !folder.contains('/') {
            folder.clone()
        } else {
            fmt::last_seg(&folder)
        },
        folder_addr: s.folder.clone().unwrap_or_default(),
        folder,
        project,
        active,
        ctx,
        model: fmt::model(s.model.as_deref().unwrap_or("")),
        kind: kind_of(s.kind.as_deref()),
        status: s.status.clone(),
        state: s.state.clone(),
        alive: alive(s.kind.as_deref(), status),
        held,
        chrome_machine: s.chrome_machine.clone().filter(|n| !n.is_empty()),
        url: s.url.clone().filter(|u| !u.is_empty()),
        bg_id: s.bg_id.clone().filter(|u| !u.is_empty()),
        handoff: s.handoff.unwrap_or(false),
        repeats,
        stalled,
        waiting,
        stale,
        activity,
        cache,
        cache_until,
        as_of: local(m.as_of.or(m.updated)).unwrap_or(0.0),
        flags: Flags::from_api(s.flags.as_ref()),
        keep: s.keep.unwrap_or(false),
        star: s.star.unwrap_or(false),
        chrome_missing: s.chrome_missing.unwrap_or(false),
        group: s.group.clone().filter(|g| !g.is_empty()),
        reaped: Reaped::from_api(s.reaped.as_ref()),
        thread: s.thread.clone().filter(|t| !t.is_empty()),
        history: s.history.unwrap_or(0.0).max(0.0) as u32,
        handoff_fresh: s.handoff_fresh.unwrap_or(false),
        cost: s.cost.clone().filter(|c| c.ctx > 0.0 || !c.text.cold.is_empty()),
        tools: s.tools.clone(),
        tools_mode: s.tools_mode.clone().filter(|m| m == "auto" || m == "pick"),
        needs: text_field(s.needs.as_deref(), 300),
        suggested_reply: text_field(s.suggested_reply.as_deref(), 200),
        ask: s.ask.as_ref().map(|a| crate::ask::Ask::from_api(a, s.needs.as_deref())),
    }
}

pub fn replaced(r: &ApiReplaced, m: &ApiMachine, label: &str, root_lbl: &str) -> Replaced {
    let raw = r.title.clone().unwrap_or_default();
    let (topic, tag) = names::split(&raw);
    Replaced {
        machine: m.name.clone(),
        machine_label: label.to_string(),
        sid: r.sid.clone(),
        key: format!("{}:{}", m.name, r.sid),
        title: names::plain(&raw),
        tag_kind: names::tag_kind(tag.as_deref()),
        topic,
        tag,
        folder: folder_of(r.folder.as_deref(), r.cwd.as_deref(), root_lbl),
        active: r.active.unwrap_or(0.0),
        how: r.how.clone(),
        by_sid: r.by.as_ref().and_then(|b| b.sid.clone()),
        by_machine: r.by.as_ref().and_then(|b| b.machine.clone()),
        kind: kind_of(r.kind.as_deref()),
        alive: alive(r.kind.as_deref(), r.status.as_deref()),
        url: r.url.clone().filter(|u| !u.is_empty()),
        bg_id: r.bg_id.clone().filter(|u| !u.is_empty()),
        reaped: Reaped::from_api(r.reaped.as_ref()),
    }
}

/// Turns one /api/sessions reply into the board. `fetched_at` is the local time it arrived.
pub fn normalize(raw: &ApiSessions, fetched_at: f64) -> Snapshot {
    // Server timestamps (cold_at, as_of, active) onto this computer's clock: local = server + (received - server now).
    // Whole seconds, so network delay jitter does not make every poll a new board.
    let offset = raw.now.filter(|n| *n > 0.0).map(|n| (fetched_at - n).round()).unwrap_or(0.0);
    let machines = raw
        .machines
        .iter()
        .map(|m| {
            let label = m.label.clone().filter(|l| !l.is_empty()).unwrap_or_else(|| m.name.clone());
            let root = m.root.clone().unwrap_or_default();
            let rl = root_label(&root);
            let mut sessions: Vec<Session> =
                m.sessions.iter().map(|s| session_at(s, m, &label, &rl, fetched_at, offset)).collect();
            sessions.sort_by(|a, b| b.active.total_cmp(&a.active));
            let mut rep: Vec<Replaced> = m.replaced.iter().map(|r| replaced(r, m, &label, &rl)).collect();
            rep.sort_by(|a, b| b.active.total_cmp(&a.active));
            Machine {
                name: m.name.clone(),
                label,
                os: m.os.clone(),
                online: m.online,
                error: m.error.clone().filter(|e| !e.is_empty()),
                updated: m.updated,
                root,
                root_label: rl,
                sessions,
                replaced: rep,
                mem: Mem::from_api(m.mem.as_ref()),
                can: m.can.clone().filter(|c| !c.is_empty()),
                has_caps: m.caps.as_ref().is_some_and(|c| c.is_object()),
                features: m.features.clone(),
                chrome: m.chrome.clone(),
                chrome_pref: m.chrome_pref == Some(true),
                setup: m.setup.clone(),
            }
        })
        .collect();
    let s = &raw.summary;
    Snapshot {
        machines,
        plan: Plan {
            week: s.seven_day,
            week_projected: s.week_projected,
            week_reset: s.seven_day_reset,
            five_hour: s.five_hour,
            five_hour_reset: s.five_hour_reset,
            note: s.note.clone().filter(|n| !n.text.trim().is_empty()),
            updated: s.limits_updated,
        },
        fetched_at,
        server_now: raw.now,
        web: raw.web.clone(),
        v: raw.v,
        cleanup: raw.cleanup.clone().filter(|c| !c.items.is_empty() || !c.stopped.is_empty()),
        groups: crate::groups::from_api(&raw.groups),
        budget: raw.budget.clone(),
        addresses: s.addresses.as_ref().map(|a| a.list.clone()).unwrap_or_default(),
    }
}

impl Snapshot {
    pub fn empty() -> Snapshot {
        Snapshot {
            machines: Vec::new(),
            plan: Plan::default(),
            fetched_at: 0.0,
            server_now: None,
            web: None,
            v: None,
            cleanup: None,
            groups: Vec::new(),
            budget: None,
            addresses: Vec::new(),
        }
    }
    /// The group a chat is in, by its thread (its own sid when it has none), from `groups`.
    pub fn group_of(&self, s: &Session) -> Option<&crate::groups::Group> {
        crate::groups::group_of(&self.groups, s.thread.as_deref().unwrap_or(&s.sid))
    }
    /// Puts `groups` in place (a change made here, or the server's answer) and marks each session's `group` again.
    pub fn regroup(&mut self, groups: Vec<crate::groups::Group>) {
        self.groups = groups;
        let gs = &self.groups;
        for m in &mut self.machines {
            for s in &mut m.sessions {
                s.group = crate::groups::group_of(gs, s.thread.as_deref().unwrap_or(&s.sid)).map(|g| g.id.clone());
            }
        }
    }
    /// True when `o` draws the same board: equal in everything but the fetch and scan times. A poll that
    /// brings nothing new then costs no redraw.
    pub fn same_board(&self, o: &Snapshot) -> bool {
        self.plan == o.plan
            && self.web == o.web
            && self.cleanup == o.cleanup
            && self.groups == o.groups
            && self.machines.len() == o.machines.len()
            && self.machines.iter().zip(&o.machines).all(|(a, b)| {
                a.name == b.name
                    && a.label == b.label
                    && a.os == b.os
                    && a.online == b.online
                    && a.error == b.error
                    && a.root == b.root
                    && a.root_label == b.root_label
                    && a.sessions == b.sessions
                    && a.replaced == b.replaced
                    && a.mem == b.mem
                    && a.can == b.can
                    && a.has_caps == b.has_caps
                    && a.features == b.features
                    && a.chrome == b.chrome
                    && a.chrome_pref == b.chrome_pref
            })
    }
    /// v2: the machine Chrome sessions go to first, if one is set.
    pub fn chrome_machine(&self) -> Option<&Machine> {
        self.machines.iter().find(|m| m.chrome_pref)
    }
    /// v2: Auto can pick a machine: at least one machine that is not offline shares what it can do.
    pub fn auto_ready(&self) -> bool {
        self.machines.iter().any(|m| m.has_caps && !m.is_offline())
    }
    pub fn machine(&self, name: &str) -> Option<&Machine> {
        self.machines.iter().find(|m| m.name == name)
    }
    pub fn label_of(&self, name: &str) -> Option<String> {
        self.machine(name).map(|m| m.label.clone())
    }
    /// Every session on every machine, newest activity first.
    pub fn sessions(&self) -> Vec<&Session> {
        let mut v: Vec<&Session> = self.machines.iter().flat_map(|m| m.sessions.iter()).collect();
        v.sort_by(|a, b| b.active.total_cmp(&a.active));
        v
    }
    pub fn replaced(&self) -> Vec<&Replaced> {
        let mut v: Vec<&Replaced> = self.machines.iter().flat_map(|m| m.replaced.iter()).collect();
        v.sort_by(|a, b| b.active.total_cmp(&a.active));
        v
    }
    /// Whether a machine's chats can be renamed: its agent says so. One not heard from yet (no feature list, not
    /// online) is given the benefit: the server keeps the name and tells the machine when it is back.
    pub fn can_rename(&self, machine: &str) -> bool {
        self.machine(machine).is_none_or(|m| {
            m.features.iter().any(|f| f == "rename") || (m.features.is_empty() && m.online != Some(true))
        })
    }
    /// A chat renamed here shows its new name at once, before the server's next snapshot says it.
    pub fn retitle(&mut self, key: &str, raw: &str) -> bool {
        let Some((m, sid)) = key.split_once(':') else { return false };
        let Some(s) =
            self.machines.iter_mut().find(|x| x.name == m).and_then(|x| x.sessions.iter_mut().find(|s| s.sid == sid))
        else {
            return false;
        };
        let (topic, tag) = names::split(raw);
        s.raw_title = raw.to_string();
        s.title = names::plain(raw);
        s.tag_kind = names::tag_kind(tag.as_deref());
        s.topic = topic;
        s.tag = tag;
        true
    }
    pub fn session(&self, key: &str) -> Option<&Session> {
        let (m, sid) = key.split_once(':')?;
        self.machine(m)?.sessions.iter().find(|s| s.sid == sid)
    }
    /// The board chat a background id belongs to on a machine (the newest one, should two ever share it).
    pub fn session_by_bg(&self, machine: &str, bg: &str) -> Option<&Session> {
        self.machine(machine)?
            .sessions
            .iter()
            .filter(|s| s.bg_id.as_deref() == Some(bg))
            .max_by(|a, b| a.active.total_cmp(&b.active))
    }
    /// The chat a terminal tab shows: by its background id (it outlives /clear and is known before a new chat's id is),
    /// else its key or the newer chat that replaced it. The tab's dot and its strip both ask this, so they never split.
    pub fn chat_for(&self, machine: &str, bg: Option<&str>, key: Option<&str>) -> Option<&Session> {
        bg.and_then(|b| self.session_by_bg(machine, b)).or_else(|| key.and_then(|k| self.session(&self.latest_key(k))))
    }
    /// The chat a saved key leads to now: itself while it is on the board, else the newer chat that replaced it
    /// (after /clear, Start fresh, Continue, ...), else the board chat of the same thread.
    pub fn latest_key(&self, key: &str) -> String {
        let mut k = key.to_string();
        for _ in 0..12 {
            if self.session(&k).is_some() {
                return k;
            }
            match self.replaced_row(&k).and_then(|r| r.newer_key()) {
                Some(n) if n != k => k = n,
                _ => break,
            }
        }
        let sid = key.split_once(':').map(|(_, s)| s).unwrap_or(key);
        self.machines
            .iter()
            .flat_map(|m| m.sessions.iter())
            .find(|s| s.thread.as_deref() == Some(sid))
            .map(|s| s.key.clone())
            .unwrap_or_else(|| key.to_string())
    }
    pub fn replaced_row(&self, key: &str) -> Option<&Replaced> {
        let (m, sid) = key.split_once(':')?;
        self.machine(m)?.replaced.iter().find(|s| s.sid == sid)
    }
    /// Machines that could take a session from `from`: every other machine that is not known to be offline.
    pub fn other_machines(&self, from: &str) -> Vec<&Machine> {
        self.machines.iter().filter(|m| m.name != from && !m.is_offline()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board;

    const NOW: f64 = 1_791_782_309.0;

    fn waiting(quiet: f64, needs: Option<&str>, reply: Option<&str>) -> Session {
        let a = ApiSession {
            sid: "s1".into(),
            title: Some("Fix the build".into()),
            kind: Some("background".into()),
            status: Some("waiting".into()),
            state: Some("blocked".into()),
            active: Some(NOW - quiet),
            mtime: Some(NOW - quiet),
            ctx: Some(40_000.0),
            needs: needs.map(str::to_string),
            suggested_reply: reply.map(str::to_string),
            ..Default::default()
        };
        let m = ApiMachine { name: "studio".into(), ..Default::default() };
        session(&a, &m, "Studio", "Claude", NOW)
    }

    #[test]
    fn folders_outside_the_claude_folder_read_nicely() {
        let at = |folder: &str| {
            let a = ApiSession { sid: "s1".into(), folder: Some(folder.into()), ..Default::default() };
            let m = ApiMachine { name: "studio".into(), ..Default::default() };
            session(&a, &m, "Studio", "Claude", NOW)
        };
        let s = at("~/Documents/site");
        assert_eq!((s.folder.as_str(), s.folder_addr.as_str()), ("~/Documents/site", "~/Documents/site"));
        assert_eq!((s.folder_short.as_str(), s.project.as_str()), ("site", "site"));
        let s = at("~/Library/Mobile Documents/com~apple~CloudDocs/Notes/week");
        assert_eq!(s.folder, "iCloud Drive/Notes/week");
        assert_eq!((s.folder_short.as_str(), s.project.as_str()), ("week", "Notes"));
        let s = at("~/Library/Mobile Documents/com~apple~CloudDocs");
        assert_eq!((s.folder.as_str(), s.folder_short.as_str()), ("iCloud Drive", "iCloud Drive"));
        let s = at("~");
        assert_eq!((s.folder.as_str(), s.folder_addr.as_str()), ("Home folder", "~"));
        let s = at("@Data/work/api");
        assert_eq!((s.folder.as_str(), s.project.as_str()), ("@Data/work/api", "work"));
        let s = at("");
        assert_eq!((s.folder.as_str(), s.folder_addr.as_str()), ("Claude", ""));
        let s = at("switchboard/.claude/worktrees/x");
        assert_eq!(s.project, "switchboard", "Claude folder addresses as before");
    }

    #[test]
    fn a_saved_tab_follows_its_chat_to_the_newest_part() {
        let raw: ApiSessions = serde_json::from_str(
            r#"{"machines":[{"name":"studio","online":true,
                "sessions":[{"sid":"s3","title":"Lights","thread":"s1"},{"sid":"s9","title":"Other","thread":"s9"}],
                "replaced":[{"sid":"s1","title":"Lights","how":"cleared","by":{"machine":"studio","sid":"s2"}},
                            {"sid":"s2","title":"Lights","how":"fresh","by":{"machine":"studio","sid":"s3"}}]}]}"#,
        )
        .unwrap();
        let snap = normalize(&raw, NOW);
        assert_eq!(snap.latest_key("studio:s1"), "studio:s3", "through /clear and Start fresh");
        assert_eq!(snap.latest_key("studio:s9"), "studio:s9", "on the board: itself");
        assert_eq!(snap.latest_key("studio:gone"), "studio:gone", "unknown: unchanged");
    }

    #[test]
    fn a_tab_finds_its_chat_by_background_id_after_a_clear() {
        let raw: ApiSessions = serde_json::from_str(
            r#"{"machines":[{"name":"laptop","online":true,
                "sessions":[{"sid":"new","title":"Canvas","bg_id":"c5ba2d9c","active":1791782300}]}]}"#,
        )
        .unwrap();
        let snap = normalize(&raw, NOW);
        assert_eq!(snap.session_by_bg("laptop", "c5ba2d9c").map(|s| s.sid.as_str()), Some("new"));
        assert!(snap.session_by_bg("studio", "c5ba2d9c").is_none());
    }

    #[test]
    fn a_fresh_wait_needs_you_and_says_its_need() {
        let s = waiting(600.0, Some("  confirm option 1\n or 2 "), Some("1"));
        assert_eq!(s.lamp(NOW), Lamp::Wait);
        assert!(!s.parked(NOW));
        assert!(s.needs_you(NOW));
        assert_eq!(s.needs.as_deref(), Some("confirm option 1 or 2"), "one tidy line");
        assert_eq!(s.suggested_reply.as_deref(), Some("1"));
        assert_eq!(s.wait_line(NOW, None).as_deref(), Some("Needs: confirm option 1 or 2"));
        assert_eq!(s.phrase(NOW), "Needs: confirm option 1 or 2");
    }

    #[test]
    fn without_a_need_the_usual_words_stand() {
        let s = waiting(600.0, Some("   "), None);
        assert_eq!(s.needs, None);
        assert_eq!(s.wait_line(NOW, None), None);
        assert_eq!(s.phrase(NOW), "Waiting for you");
    }

    #[test]
    fn a_day_later_it_is_parked_and_stops_counting() {
        let s = waiting(2.0 * 86400.0 + 3600.0, Some("pick a name"), None);
        assert!(s.parked(NOW));
        assert_eq!(s.lamp(NOW), Lamp::Wait, "still waiting, still in Needs you");
        assert!(!s.needs_you(NOW));
        assert_eq!(s.wait_line(NOW, None).as_deref(), Some("Waiting 2 days: pick a name"));
        let one = waiting(PARKED + 60.0, None, None);
        assert_eq!(one.wait_line(NOW, None).as_deref(), Some("Waiting 1 day"));
        // Just under a day: not yet.
        assert!(!waiting(PARKED - 60.0, None, None).parked(NOW));

        let fresh = waiting(600.0, None, None);
        let t = board::tally([&s, &fresh], NOW);
        assert_eq!((t.wait, t.parked, t.need), (2, 1, 1));
    }

    #[test]
    fn held_and_working_chats_are_never_parked() {
        // The server says held (it outranks parked); an older server's held flag reads the same.
        let mut s = waiting(3.0 * 86400.0, None, None);
        s.held = true;
        s.activity = Activity::Held;
        assert!(!s.parked(NOW));
        assert!(s.needs_you(NOW));
        assert_eq!(s.wait_line(NOW, None), None);
        let mut s = waiting(3.0 * 86400.0, None, None);
        s.waiting = false;
        s.activity = Activity::Working;
        assert!(!s.parked(NOW));
    }

    #[test]
    fn cards_clip_a_long_need() {
        let long = "confirm the migration plan for the database tables and then the cache layer and finally the search index rebuild that follows";
        let s = waiting(600.0, Some(long), None);
        let card = s.wait_line(NOW, Some(60)).unwrap();
        assert!(card.ends_with("..."));
        assert!(card.chars().count() <= "Needs: ".len() + 60);
        assert_eq!(s.wait_line(NOW, None).unwrap(), format!("Needs: {long}"));
        // The server's caps hold here too.
        let s = waiting(600.0, Some(&"x".repeat(400)), Some(&"y".repeat(300)));
        assert_eq!(s.needs.unwrap().len(), 300);
        assert_eq!(s.suggested_reply.unwrap().len(), 200);
    }

    #[test]
    fn a_waiting_chat_always_has_something_to_answer() {
        let s = waiting(600.0, Some("pick a name"), None);
        let a = s.ask_now(NOW).unwrap();
        assert_eq!((a.kind, a.summary.as_str()), (crate::ask::AskKind::Open, "pick a name"));
        let mut s = waiting(600.0, Some("pick a name"), None);
        s.held = true;
        assert_eq!(s.ask_now(NOW), None);

        let raw = r#"{"sid": "s1", "status": "waiting", "state": "blocked", "active": 1791781709, "needs": "the need",
            "ask": {"kind": "task", "steps": ["Plug it in"], "done_reply": "Done, plugged in."}}"#;
        let a: ApiSession = serde_json::from_str(raw).unwrap();
        let s = session(&a, &ApiMachine { name: "studio".into(), ..Default::default() }, "Studio", "Claude", NOW);
        let k = s.ask_now(NOW).unwrap();
        assert_eq!(k.kind, crate::ask::AskKind::Task);
        assert_eq!(k.summary, "the need", "the need stands in for a missing summary");
        // an odd ask never sinks the row
        let a: ApiSession = serde_json::from_str(r#"{"sid": "x", "status": "waiting", "ask": "oops"}"#).unwrap();
        assert_eq!(a.ask, None);
    }

    #[test]
    fn older_servers_parse_without_the_new_fields() {
        let a: ApiSession = serde_json::from_str(r#"{"sid": "x", "status": "waiting", "needs": 5}"#).unwrap();
        assert_eq!(a.needs, None);
        assert_eq!(a.suggested_reply, None);
    }

    #[test]
    fn a_star_is_read_and_defaults_off() {
        let m = ApiMachine { name: "studio".into(), ..Default::default() };
        let a: ApiSession = serde_json::from_str(r#"{"sid": "x", "status": "idle", "star": true}"#).unwrap();
        assert!(session(&a, &m, "Studio", "Claude", NOW).star);
        let a: ApiSession = serde_json::from_str(r#"{"sid": "x", "status": "idle"}"#).unwrap();
        assert!(!session(&a, &m, "Studio", "Claude", NOW).star);
    }

    #[test]
    fn groups_come_with_the_snapshot_and_follow_the_thread() {
        let raw: ApiSessions = serde_json::from_str(
            r#"{"machines": [{"name": "studio", "sessions": [
                {"sid": "s1", "thread": "t1", "status": "idle", "group": "g1"},
                {"sid": "s2", "status": "idle"}]}],
               "groups": [{"id": "g1", "name": "Server", "color": "cyan", "members": ["t1"]}, {"id": "", "members": []},
                          {"id": "g2", "name": 7, "color": "plaid", "members": ["s9", 3]}]}"#,
        )
        .unwrap();
        let mut snap = normalize(&raw, NOW);
        assert_eq!(snap.groups.len(), 2, "a group without an id is dropped");
        assert_eq!(
            (snap.groups[1].name.as_str(), snap.groups[1].color, snap.groups[1].members.clone()),
            ("", crate::groups::GroupColor::Grey, vec!["s9".to_string()])
        );
        let s1 = snap.session("studio:s1").unwrap().clone();
        assert_eq!(s1.group.as_deref(), Some("g1"));
        assert_eq!(snap.group_of(&s1).map(|g| g.name.as_str()), Some("Server"));
        // A change made here: s2 (no thread, so its own sid) joins g2, s1 leaves.
        let mut gs = snap.groups.clone();
        crate::groups::Op::Add { id: "g2".into(), thread: "s2".into(), index: None }.apply(&mut gs, "");
        crate::groups::Op::Remove { thread: "t1".into() }.apply(&mut gs, "");
        snap.regroup(gs);
        assert_eq!(snap.groups.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), vec!["g2"]);
        assert_eq!(snap.session("studio:s1").unwrap().group, None);
        assert_eq!(snap.session("studio:s2").unwrap().group.as_deref(), Some("g2"));
        // An older server: no groups at all.
        let old: ApiSessions = serde_json::from_str(r#"{"machines": [], "groups": "nope"}"#).unwrap();
        assert!(normalize(&old, NOW).groups.is_empty());
    }
}
