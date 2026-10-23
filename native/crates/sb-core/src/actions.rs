//! What you can do with a chat, in the same order and words as the web app. The primary one comes first.

use serde::{Deserialize, Serialize};

use crate::model::{Band, Lamp, Replaced, Session, Snapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionId {
    /// Open the chat's claude.ai link.
    Open,
    /// Open a running background chat in a terminal tab (desktop). Disabled, with the reason, for a chat that is
    /// open in a terminal on its machine.
    Terminal,
    /// Resume a stopped chat in the background, then open its terminal (desktop).
    ResumeOpen,
    /// Copy `claude attach <bg_id>` (waiting, background, no link).
    Attach,
    Fresh,
    /// Send the held message anyway.
    Pass,
    Resume,
    Continue,
    Handoff,
    Stop,
    Dismiss,
    /// Replaced rows: jump to the chat that replaced it.
    Newer,
    /// Replaced rows: open the old chat in Claude to archive it there.
    ArchiveInClaude,
    /// v2: Keep running (toggles): the reaper leaves this background chat alone.
    Keep,
    /// v2: "This continues...": pick the older chat this one carries on from.
    Link,
    /// v2: "Not the same topic": split this chat from the earlier ones in its history.
    Unlink,
    /// v2, replaced rows: "Not the same topic": put it back on the list as a chat of its own.
    Split,
    /// Give the chat a name of its own (the window opens a field for it; POST /api/action verb rename).
    Rename,
}

impl ActionId {
    /// The verb the server takes on POST /api/action, if this is one.
    pub fn verb(self) -> Option<&'static str> {
        match self {
            ActionId::Fresh => Some("fresh"),
            ActionId::Pass => Some("pass"),
            ActionId::Resume => Some("resume"),
            ActionId::Handoff => Some("handoff"),
            ActionId::Stop => Some("stop"),
            ActionId::Dismiss => Some("dismiss"),
            ActionId::Keep => Some("keep"),
            ActionId::Link => Some("link"),
            ActionId::Unlink | ActionId::Split => Some("unlink"),
            _ => None,
        }
    }
    /// v2: the thread actions, which the menu sets apart under a rule.
    pub fn is_thread(self) -> bool {
        matches!(self, ActionId::Link | ActionId::Unlink | ActionId::Split)
    }
    /// Verbs the server runs as a job (followed with GET /api/job/server/<id>).
    pub fn is_job(self) -> bool {
        matches!(self, ActionId::Fresh | ActionId::Resume | ActionId::Handoff)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub title: String,
    pub note: String,
    pub primary: bool,
    pub danger: bool,
    pub disabled: bool,
    /// Icon name from the shared stroke set.
    pub icon: String,
    /// The key that runs it from the board ("Enter", "F", "R", "C"), if any.
    pub key: Option<String>,
    pub href: Option<String>,
}

fn a(id: ActionId, title: &str, note: &str, icon: &str) -> Action {
    Action {
        id,
        title: title.into(),
        note: note.into(),
        primary: false,
        danger: false,
        disabled: false,
        icon: icon.into(),
        key: None,
        href: None,
    }
}

pub fn for_session(s: &Session, snap: &Snapshot, now: f64) -> Vec<Action> {
    let l = s.lamp(now);
    let bg = s.background();
    let alive = s.alive;
    let label = snap.label_of(&s.machine).unwrap_or_else(|| s.machine_label.clone());
    let recommend_fresh = s.recommend_fresh(now);
    let mut items = Vec::new();
    // A running background chat opens in a terminal tab: that is Enter and the primary button. Open in Claude stays,
    // as a secondary.
    let term = bg && alive;
    if term {
        let mut x = a(ActionId::Terminal, "Open terminal", "Type into it here. Closing the tab only detaches.", "term");
        x.primary = !recommend_fresh;
        x.key = Some("Enter".into());
        items.push(x);
    }
    if let Some(url) = &s.url {
        let mut x = a(ActionId::Open, "Open in Claude", "Pick up this session on this device", "out");
        x.primary = !recommend_fresh && alive && !term;
        x.key = (!term).then(|| "Enter".into());
        x.href = Some(url.clone());
        items.push(x);
    } else if let (Lamp::Wait, true, Some(id)) = (l, bg, &s.bg_id) {
        items.push(a(
            ActionId::Attach,
            "Copy the attach command",
            &format!("claude attach {id}, then run it on {label} to answer"),
            "copy",
        ));
    }
    if alive && !bg {
        let mut x = a(
            ActionId::Terminal,
            "Open terminal",
            &format!("It's open in a terminal on {label}. Only background chats open here."),
            "term",
        );
        x.disabled = true;
        items.push(x);
    }
    // The menu says what each choice costs, live: the agent's numbers for the cache as it is this second.
    let warm = s.cache_left(now) > 0;
    let kc = |n: f64| crate::fmt::k(n.max(0.0).round() as u64);
    let fresh_cost = s.cost.as_ref().map(|c| {
        if c.summary_ready {
            format!(", about {} (summary already written)", kc(c.fresh.total))
        } else {
            format!(", about {} (summary {} + new chat {})", kc(c.fresh.total), kc(c.fresh.summary), kc(c.fresh.first))
        }
    });
    let fresh_note = format!(
        "{}{}",
        if s.held {
            "New chat with your notes and the held message"
        } else {
            "New chat that starts from a summary of this one"
        },
        fresh_cost.unwrap_or_default()
    );
    let mut fresh = a(ActionId::Fresh, "Start fresh", &fresh_note, "fresh");
    fresh.primary = recommend_fresh;
    fresh.key = Some("F".into());
    items.push(fresh);
    if s.held {
        items.push(a(
            ActionId::Pass,
            "Send anyway",
            &format!("Re-reads {} once, then the guard leaves it alone", s.ctx_label()),
            "send",
        ));
    }
    let resume_note = if alive && !bg {
        format!("Already open in a terminal on {label}")
    } else {
        match s.cost.as_ref() {
            Some(c) if warm => format!("Re-opens the full chat, about {} while the cache is warm", kc(c.resume.warm)),
            Some(c) => format!("Re-opens the full chat, about {} because the cache is cold", kc(c.resume.cold)),
            None => format!(
                "Re-opens the full chat{}",
                if l == Lamp::Cold { format!(", re-reading {}", s.ctx_label()) } else { String::new() }
            ),
        }
    };
    if !alive {
        let mut x =
            a(ActionId::ResumeOpen, "Resume and open", "Resumes it in the background, then opens its terminal", "term");
        x.primary = !recommend_fresh;
        items.push(x);
    }
    let mut resume = a(ActionId::Resume, "Resume as is", &resume_note, "resume");
    resume.disabled = alive;
    resume.key = Some("R".into());
    items.push(resume);
    let mut cont = a(ActionId::Continue, "Continue on another machine", "Writes a handoff and starts there", "cont");
    cont.key = Some("C".into());
    cont.disabled = snap.other_machines(&s.machine).is_empty();
    items.push(cont);
    items.push(a(
        ActionId::Handoff,
        if bg && alive { "Hand off and stop" } else { "Write a handoff" },
        if s.handoff {
            "A handoff exists; this writes a newer one if needed"
        } else {
            "Saves notes you can start from later"
        },
        "handoff",
    ));
    if s.can_keep() {
        let (title, note) = if s.keep {
            ("Kept running", "Switchboard leaves it running. Click to let Switchboard stop it when idle.")
        } else {
            ("Keep running", "Switchboard stops idle background chats to free memory. Not this one.")
        };
        items.push(a(ActionId::Keep, title, note, "pin"));
    }
    if bg && alive {
        let mut stop = a(ActionId::Stop, "Stop", "Ends the background session; you can resume it later", "stop");
        stop.danger = true;
        items.push(stop);
    }
    items.push(a(
        ActionId::Dismiss,
        "Dismiss",
        if bg && alive {
            "Stops it and hides it from this list"
        } else if alive {
            "Hides it here. The terminal stays open."
        } else {
            "Hides it until it's used again"
        },
        "dismiss",
    ));
    // An older agent cannot keep a name (Snapshot::can_rename).
    let can_rename = snap.can_rename(&s.machine);
    let mut rename = a(
        ActionId::Rename,
        "Rename chat",
        if can_rename {
            "Shows on every device, and in Claude Code's own list"
        } else {
            "Update Switchboard on that machine to rename its chats"
        },
        "pencil",
    );
    rename.disabled = !can_rename;
    items.push(rename);
    items.push(a(ActionId::Link, "This continues...", "Pick the older chat it carries on from", "link"));
    if s.history > 0 {
        let what =
            if s.history == 1 { "the earlier chat".to_string() } else { format!("the {} earlier chats", s.history) };
        items.push(a(
            ActionId::Unlink,
            "Not the same topic",
            &format!("Splits it from {what} in its history"),
            "unlink",
        ));
    }
    if let Some(pi) = items.iter().position(|x| x.primary)
        && pi > 0
    {
        let p = items.remove(pi);
        items.insert(0, p);
    }
    items
}

pub fn for_replaced(r: &Replaced, snap: &Snapshot) -> Vec<Action> {
    let mut out = Vec::new();
    if let Some(newer) = r.newer_key().and_then(|k| snap.session(&k)) {
        let mut x = a(ActionId::Newer, "Open the newer session", &newer.title, "chev");
        x.primary = true;
        x.key = Some("Enter".into());
        out.push(x);
    }
    if let Some(url) = &r.url {
        let mut x = a(
            ActionId::ArchiveInClaude,
            "Archive it in Claude",
            "Opens the old chat in Claude; archive it there so only the new one shows",
            "out",
        );
        x.href = Some(url.clone());
        out.push(x);
    }
    let mut resume = a(
        ActionId::Resume,
        "Resume as is",
        if r.alive { "It's still open on that machine" } else { "Re-opens this older chat, as it was" },
        "resume",
    );
    resume.disabled = r.alive;
    out.push(resume);
    out.push(a(
        ActionId::Dismiss,
        "Dismiss",
        if r.background() && r.alive { "Stops it and hides it from this list" } else { "Hides it from this list" },
        "dismiss",
    ));
    if r.can_split() {
        out.push(a(ActionId::Split, "Not the same topic", "Puts it back on the list as a chat of its own", "unlink"));
    }
    out
}

/// The confirm dialog before Start fresh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Confirm {
    pub title: String,
    pub body: String,
    pub ok: String,
    pub cancel: String,
}

pub fn fresh_confirm(s: &Session) -> Confirm {
    let cost = s.cost_line(crate::fmt::now_epoch()).map(|c| format!(" {c}")).unwrap_or_default();
    Confirm {
        title: format!("Start {} fresh?", s.topic),
        body: format!(
            "Writes a summary on {} and opens a new chat with your notes{}, so the next message doesn't re-read {}. The old chat moves to Replaced.{cost}",
            s.machine_label,
            if s.held { " and the held message" } else { "" },
            s.ctx_label()
        ),
        ok: "Start fresh".into(),
        cancel: "Cancel".into(),
    }
}

/// New session form constants, the same as the web app.
pub const MODES: [(&str, &str, &str); 3] = [
    ("normal", "Normal", "Your usual effort"),
    ("max", "Max effort", "Deepest reasoning on every turn"),
    ("ultracode", "UltraCode", "Multi-agent workflows for big builds"),
];
/// The model picker when the server has not said (an older one, or offline): the aliases every Claude Code takes.
pub const MODELS: [(&str, &str); 5] =
    [("", "Default"), ("fable", "Fable"), ("opus", "Opus"), ("sonnet", "Sonnet"), ("haiku", "Haiku")];

/// The model picker from GET /api/models: Default, then what the machines' Claude Code takes, each shown with the
/// newest version the chats ran on ("Opus 5.5") when there is one. Nothing from the server: the built-in list.
pub fn model_choices(list: Option<&[crate::api::ApiModel]>) -> Vec<(String, String)> {
    match list {
        Some(l) if !l.is_empty() => std::iter::once((String::new(), "Default".to_string()))
            .chain(l.iter().map(|m| {
                let shown = if !m.detail.is_empty() {
                    m.detail.clone()
                } else if !m.label.is_empty() {
                    m.label.clone()
                } else {
                    model_word(&m.id)
                };
                (m.id.clone(), shown)
            }))
            .collect(),
        _ => MODELS.iter().map(|(v, l)| (v.to_string(), l.to_string())).collect(),
    }
}

/// "Opus" for opus, "Opus 5.5" for claude-opus-5-5: a model as a chip says it.
pub fn model_word(v: &str) -> String {
    let cap = |s: &str| {
        let mut c = s.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    };
    let parts: Vec<&str> = v.trim_end_matches("[1m]").split('-').collect();
    if parts.len() >= 3
        && parts[0] == "claude"
        && let Ok(major) = parts[2].parse::<u32>()
    {
        let minor = parts.get(3).filter(|p| p.len() <= 2).and_then(|p| p.parse::<u32>().ok());
        return format!("{} {major}{}", cap(parts[1]), minor.map(|m| format!(".{m}")).unwrap_or_default());
    }
    cap(v)
}
pub const ULTRA_NOTE: &str =
    "Runs multi-agent workflows for big builds, on Opus at extra high effort. Uses the plan faster.";

/// A new folder name the server accepts: letters, numbers, spaces, dots, dashes, underscores, 1 to 64.
pub fn valid_folder_name(name: &str) -> bool {
    let n = name.chars().count();
    (1..=64).contains(&n) && name.chars().all(|c| c.is_ascii_alphanumeric() || "._ -".contains(c))
}

/// The warning under UltraCode when the five-hour window is already busy.
pub fn ultra_warn(plan: &crate::model::Plan, now: f64) -> Option<String> {
    let five = plan.five_hour.unwrap_or(0.0).round() as i64;
    if five < 60 {
        return None;
    }
    let mut w = format!("The 5-hour window is at {five}%. Workflows stop if it fills mid-run.");
    if plan.five_live(now)
        && let Some(r) = plan.five_hour_reset
    {
        w.push_str(&format!(" It resets at {}.", crate::fmt::when(r, now)));
    }
    Some(w)
}

/// Resume or Start fresh: which one the inspector suggests now, and why, in plain words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pick {
    Resume,
    Fresh,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggest {
    /// None while it is working: open the terminal to watch it instead.
    pub pick: Option<Pick>,
    pub why: String,
    /// What resuming costs: "Pays only for your new message while warm (12:40 left)" or "Your next message re-reads 213k".
    pub resume_cost: String,
    /// Both choices in full from the agent's cost model, for the cache as it is now: "Start fresh: about 91k
    /// (summary 79k + new chat 12k). Resume: about 290k, because the cache is cold..." None from an older agent.
    pub cost: Option<String>,
    /// "How is this estimated?": its inputs and ratios.
    pub how: Option<String>,
}

/// Where Claude squeezes a chat down; past this a fresh start is the better pick even when it is small enough to read.
const NEAR_COMPACT: u64 = 270_000;

pub fn suggest(s: &Session, now: f64) -> Suggest {
    let left = s.cache_left(now);
    let kk = s.ctx_label();
    let resume_cost = if left > 0 {
        format!("Pays only for your new message while warm ({} left)", crate::fmt::clock(left))
    } else if s.ctx > 0 {
        format!("Your next message re-reads {kk}")
    } else {
        "Nothing to re-read yet".to_string()
    };
    let (pick, why) = match s.lamp(now) {
        Lamp::Run => (None, "It's working right now. Open the terminal to watch or steer it.".to_string()),
        _ if s.held => (
            Some(Pick::Fresh),
            format!("It went cold at {kk}. Sending your held message would re-read all of it first."),
        ),
        _ if left > 0 => (
            Some(Pick::Resume),
            format!("It's still warm for {}, so resuming only pays for your new message.", crate::fmt::mins(left)),
        ),
        _ if s.ctx >= NEAR_COMPACT => {
            (Some(Pick::Fresh), format!("At {kk} it's close to the 300k point where Claude squeezes the chat down."))
        }
        _ if s.ctx == 0 => (Some(Pick::Resume), "It has no history to re-read yet.".to_string()),
        _ => {
            // Cold: the cheaper one up front by the cost model, else by size band.
            // Since when: the server's cache end, never a guess. Not known: say it is quiet instead.
            let went = if s.cache_until > 0.0 && s.cache_until <= now {
                format!("It went cold {} ago", crate::fmt::sago(s.cache_until, now))
            } else {
                format!("It has been quiet {}", crate::fmt::sago(s.active, now))
            };
            let band = s.band();
            let pick = s.cost_pick(now).unwrap_or(if band == Band::Small { Pick::Resume } else { Pick::Fresh });
            let why = match (pick, band) {
                (_, Band::Small) => format!("It's small ({kk}), so going back is cheap even with the cache cold."),
                (Pick::Fresh, Band::Big) => format!("{went} and holds {kk}. Resuming re-reads all of it."),
                (Pick::Fresh, Band::Medium) => format!(
                    "{went}. Resuming re-reads all {kk}, so Start fresh is cheaper if you don't need the history."
                ),
                (Pick::Resume, _) => {
                    format!("{went}, but resuming its {kk} still costs less than writing a summary and starting over.")
                }
            };
            (Some(pick), why)
        }
    };
    Suggest { pick, why, resume_cost, cost: s.cost_line(now), how: s.cost_how() }
}
