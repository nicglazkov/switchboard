//! Jobs: Start fresh, New session, Resume, Continue and Handoff are taken by the server at once and finished there.
//! The app follows them with GET /api/job/server/<id> while they run. Same words and steps as the web app.

use serde::{Deserialize, Serialize};

use crate::api::ApiJob;
use crate::names;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobKind {
    Fresh,
    Start,
    Resume,
    Continue,
    Handoff,
    /// v2: a New session sent to "auto": Claude picks the machine when the job runs.
    Auto,
    /// v2: the same chat restarted with new launch flags (POST /api/relaunch); it carries on with its history.
    Relaunch,
    /// A History thread of several chats compacted into one fresh chat (POST /api/compact with confirm).
    Compact,
    /// A stop queued until the chat's own work ends (Stop on a working chat, or a closed tab).
    Stop,
}

impl JobKind {
    pub fn label(self) -> &'static str {
        match self {
            JobKind::Fresh => "Start fresh",
            JobKind::Start => "New chat",
            JobKind::Resume => "Resume",
            JobKind::Continue => "Continue elsewhere",
            JobKind::Handoff => "Write a handoff",
            JobKind::Auto => "New chat",
            JobKind::Relaunch => "Relaunch",
            JobKind::Compact => "Compact the thread",
            JobKind::Stop => "Stop",
        }
    }
    /// The headline while it waits for the chat's own work to end.
    pub fn queued(self) -> &'static str {
        match self {
            JobKind::Stop => "Stops when its work is done",
            JobKind::Fresh => "Starts fresh when its work is done",
            JobKind::Handoff => "Hands off when its work is done",
            JobKind::Continue => "Moves when its work is done",
            JobKind::Relaunch => "Relaunches when its work is done",
            _ => "Waiting for its work to finish",
        }
    }
    /// The card's state word while it waits.
    pub fn queued_short(self) -> &'static str {
        match self {
            JobKind::Stop => "Stops after this work",
            JobKind::Fresh => "Fresh start queued",
            JobKind::Handoff => "Handoff queued",
            JobKind::Continue => "Move queued",
            JobKind::Relaunch => "Relaunch queued",
            _ => "Queued",
        }
    }
    pub fn doing(self) -> &'static str {
        match self {
            JobKind::Fresh => "Starting fresh",
            JobKind::Start => "Starting",
            JobKind::Resume => "Resuming",
            JobKind::Continue => "Moving",
            JobKind::Handoff => "Handing off",
            JobKind::Auto => "Starting",
            JobKind::Relaunch => "Relaunching",
            JobKind::Compact => "Compacting the thread",
            JobKind::Stop => "Stopping",
        }
    }
    pub fn ready(self) -> &'static str {
        match self {
            JobKind::Fresh => "The new chat is ready.",
            JobKind::Start => "The session is ready.",
            JobKind::Resume => "It's open again.",
            JobKind::Continue => "It's running there.",
            JobKind::Handoff => "The handoff is saved.",
            JobKind::Auto => "The session is ready.",
            JobKind::Relaunch => "Relaunched with the new flags. The chat picked up where it left off.",
            JobKind::Compact => "The new chat is ready.",
            JobKind::Stop => "Stopped.",
        }
    }
    /// The steps shown while it runs. `label` is the machine doing the work.
    pub fn steps(self, label: &str) -> Vec<String> {
        match self {
            JobKind::Fresh => vec![
                format!("Sent to {label}"),
                "Writing the handoff (about a minute)".into(),
                "Starting the new chat".into(),
                "Ready".into(),
            ],
            JobKind::Start => vec![format!("Sent to {label}"), "Naming and starting it".into(), "Ready".into()],
            JobKind::Resume => vec![format!("Sent to {label}"), "Opening the chat".into(), "Ready".into()],
            JobKind::Continue => {
                vec!["Writing the handoff (about a minute)".into(), format!("Starting on {label}"), "Ready".into()]
            }
            JobKind::Handoff => {
                vec![format!("Sent to {label}"), "Writing the handoff (about a minute)".into(), "Saved".into()]
            }
            JobKind::Auto if label == AUTO => {
                vec![
                    "Claude picks the machine".into(),
                    "Sent to that machine".into(),
                    "Naming and starting it".into(),
                    "Ready".into(),
                ]
            }
            JobKind::Auto => vec![
                format!("Claude picked {label}"),
                format!("Sent to {label}"),
                "Naming and starting it".into(),
                "Ready".into(),
            ],
            JobKind::Relaunch => vec![
                format!("Sent to {label}"),
                "Restarting Claude Code with the new flags".into(),
                "Chat continued".into(),
            ],
            JobKind::Compact => vec![
                "Gathering the thread's chats".into(),
                "Writing one summary of the whole thread (a minute or two)".into(),
                format!("Starting the new chat on {label}"),
                "Ready".into(),
            ],
            JobKind::Stop => vec![format!("Sent to {label}"), "Stopping it".into(), "Stopped".into()],
        }
    }
}

/// The label an Auto job has until the server says which machine it picked.
pub const AUTO: &str = "Auto";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Error,
}

impl JobState {
    pub fn parse(s: &str) -> JobState {
        match s {
            "running" => JobState::Running,
            "done" => JobState::Done,
            "error" => JobState::Error,
            _ => JobState::Queued,
        }
    }
    pub fn finished(self) -> bool {
        matches!(self, JobState::Done | JobState::Error)
    }
}

/// One job this app started, as it is followed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    /// The chat it came from, if any ("<machine>:<sid>").
    pub key: Option<String>,
    /// The machine doing the work.
    pub label: String,
    /// The source machine (continue).
    pub src: String,
    /// The title of the chat it came from.
    pub from: Option<String>,
    /// The name the new chat will have, best guess until the server says.
    pub name: Option<String>,
    pub started: f64,
    pub state: JobState,
    pub step: Option<String>,
    pub error: Option<String>,
    pub url: Option<String>,
    /// The new chat, when the server reports it.
    pub new_sid: Option<String>,
    pub new_machine: Option<String>,
    /// The background id of the chat it started, when the server reports one (opens its terminal).
    #[serde(default)]
    pub new_bg: Option<String>,
    pub ended: Option<f64>,
    /// v2, Auto: why that machine ("Has Xcode 26 and your iPhone connected."), and who picked it ("claude" | "rules").
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default)]
    pub by: Option<String>,
    /// Queued until the chat's own work ends (a fresh start or a move of a busy chat): the server's words for
    /// what it waits on. None once it runs.
    #[serde(default)]
    pub waiting: Option<String>,
    /// Called off while it waited (Cancel, Keep running): nothing was changed, so the card is not an error.
    #[serde(default)]
    pub cancelled: bool,
}

impl Job {
    #[allow(clippy::too_many_arguments)] // One argument per field a new job starts with; a struct would only repeat them.
    pub fn new(
        id: &str,
        kind: JobKind,
        key: Option<String>,
        label: &str,
        src: &str,
        from: Option<String>,
        name: Option<String>,
        now: f64,
    ) -> Job {
        Job {
            id: id.into(),
            kind,
            key,
            label: label.into(),
            src: src.into(),
            from,
            name,
            started: now,
            state: JobState::Queued,
            step: Some("sent".into()),
            error: None,
            url: None,
            new_sid: None,
            new_machine: None,
            new_bg: None,
            ended: None,
            why: None,
            by: None,
            waiting: None,
            cancelled: false,
        }
    }

    /// Takes one poll reply.
    pub fn update(&mut self, j: &ApiJob, now: f64) {
        if let Some(s) = &j.state {
            self.state = JobState::parse(s);
        }
        if j.step.is_some() {
            self.step = j.step.clone();
        }
        let waited = self.waiting.is_some();
        self.waiting = (self.state == JobState::Queued && self.step.as_deref() == Some("waiting")).then(|| {
            j.note.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Waiting for its work to finish".into())
        });
        if waited && self.waiting.is_none() {
            self.started = now; // its own steps start now: the wait does not count toward them
        }
        if let Some(n) = &j.name {
            self.name = Some(n.clone());
        }
        if j.url.is_some() {
            self.url = j.url.clone();
        }
        if j.sid.is_some() {
            self.new_sid = j.sid.clone();
        }
        if j.machine.is_some() {
            self.new_machine = j.machine.clone();
        }
        if j.bg_id.is_some() {
            self.new_bg = j.bg_id.clone();
        }
        // Auto: once the server has picked, the job is that machine's ("Auto picked Laptop. Has Xcode 26.").
        if self.kind == JobKind::Auto {
            if let (Some(m), Some(l)) = (&j.machine, &j.label)
                && m != "auto"
                && !l.is_empty()
            {
                self.label = l.clone();
                self.src = l.clone();
            }
            if let Some(p) = &j.picked {
                let why = match p.strip_prefix("Auto picked ") {
                    Some(rest) => rest.split_once(". ").map(|(_, w)| w.to_string()).unwrap_or_default(),
                    None => p.clone(),
                };
                self.why = Some(why).filter(|w| !w.is_empty());
            }
            if j.by.is_some() {
                self.by = j.by.clone();
            }
        }
        if self.state == JobState::Error {
            self.error = Some(j.error.clone().unwrap_or_else(|| "Something went wrong.".into()));
            if self.error.as_deref().is_some_and(|e| e.starts_with("Cancelled")) {
                self.cancelled = true;
            }
        }
        if self.state.finished() && self.ended.is_none() {
            self.ended = Some(now);
        }
    }

    /// Called off by the user while it waited: nothing was changed.
    pub fn call_off(&mut self, now: f64) {
        self.fail("Cancelled. Nothing was changed.", now);
        self.cancelled = true;
        self.waiting = None;
    }

    /// A stop of chat `key` that has not started yet (it waits for the chat's reply to end): opening the chat again
    /// calls it off.
    pub fn stop_waits_for(&self, key: &str) -> bool {
        self.kind == JobKind::Stop
            && self.key.as_deref() == Some(key)
            && self.state == JobState::Queued
            && !self.cancelled
    }

    /// The topic of the chat it came from.
    fn source_topic(&self) -> Option<String> {
        self.from.as_deref().map(|f| names::split(f).0.to_string()).filter(|t| !t.is_empty())
    }

    /// The word on the chat's own card while it runs.
    pub fn state_word(&self) -> &'static str {
        if self.waiting.is_some() { self.kind.queued_short() } else { self.kind.doing() }
    }

    pub fn fail(&mut self, msg: &str, now: f64) {
        self.state = JobState::Error;
        self.error = Some(msg.into());
        self.ended = Some(now);
    }

    /// Index of the current step.
    pub fn at(&self, now: f64) -> usize {
        let steps = self.kind.steps(&self.label);
        let last = steps.len() - 1;
        if self.state == JobState::Done {
            return last;
        }
        let el = now - self.started;
        let mut at = if self.waiting.is_some() {
            0
        } else if self.kind == JobKind::Auto {
            let picked = self.label != AUTO && self.step.as_deref() != Some("choosing");
            if !picked {
                0
            } else if self.state == JobState::Running {
                2
            } else {
                1
            }
        } else if self.kind == JobKind::Compact {
            match self.step.as_deref() {
                None | Some("sent") | Some("gather") | Some("retrying") => 0,
                Some("summary") => 1,
                _ => 2,
            }
        } else if self.kind == JobKind::Continue {
            match self.step.as_deref() {
                None | Some("handoff") | Some("sent") => 0,
                _ => 1,
            }
        } else if self.kind == JobKind::Fresh && self.state == JobState::Running && el > 50.0 {
            2
        } else {
            1
        };
        if self.state == JobState::Error {
            at = at.min(last.saturating_sub(1));
        }
        at
    }

    /// v2, Auto: the pick for the job card, as (who, what): ("Claude picked", "Laptop. Has Xcode 26.").
    pub fn pick(&self) -> Option<(String, String)> {
        if self.kind != JobKind::Auto {
            return None;
        }
        if self.label == AUTO {
            return (!self.state.finished())
                .then(|| ("Auto".to_string(), "Claude is reading what each machine can do.".to_string()));
        }
        let who = if self.by.as_deref() == Some("rules") { "Picked by the rules" } else { "Claude picked" };
        let what = match &self.why {
            Some(w) => format!("{}. {w}", self.label),
            None => self.label.clone(),
        };
        Some((who.to_string(), what))
    }

    /// Bold headline for the progress strip.
    pub fn headline(&self) -> String {
        match self.state {
            JobState::Error if self.cancelled && self.kind == JobKind::Stop => "Kept running".into(),
            JobState::Error if self.cancelled => format!("{} called off", self.kind.label()),
            JobState::Done => self.kind.ready().trim_end_matches('.').to_string(),
            JobState::Error => format!("{} didn't finish", self.kind.label()),
            _ if self.waiting.is_some() => self.kind.queued().to_string(),
            _ => self.kind.doing().to_string(),
        }
    }

    /// The line under it.
    pub fn detail(&self, now: f64) -> String {
        match self.state {
            JobState::Error if self.cancelled => match self.source_topic() {
                Some(t) => format!("{t}. Nothing was changed."),
                None => "Nothing was changed.".into(),
            },
            JobState::Error => self.error.clone().unwrap_or_else(|| "Something went wrong.".into()),
            JobState::Done if self.kind == JobKind::Stop => match self.source_topic() {
                Some(t) => format!("{t}, on {}. Resume it any time.", self.label),
                None => format!("On {}. Resume it any time.", self.label),
            },
            JobState::Done => {
                let topic = self.name.as_deref().map(|n| names::split(n).0);
                match topic {
                    Some(t) => format!("{t}, on {}", self.label),
                    None => format!("on {}", self.label),
                }
            }
            _ => {
                if let Some(w) = &self.waiting {
                    let w = w.trim_end_matches('.');
                    return match self.source_topic() {
                        Some(t) => format!("{t}. {w}."),
                        None => format!("{w}."),
                    };
                }
                if self.step.as_deref() == Some("retrying") {
                    let who = if self.kind == JobKind::Continue { &self.src } else { &self.label };
                    return format!("{who} isn't answering yet. Still trying.");
                }
                let steps = self.kind.steps(&self.label);
                steps[self.at(now).min(steps.len() - 1)].clone()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_waiting_stop_of_that_chat_is_called_off_on_reopen() {
        let stop = |key: &str| Job::new("s-1", JobKind::Stop, Some(key.into()), "Studio", "Studio", None, None, 1000.0);
        assert!(stop("studio:a").stop_waits_for("studio:a"));
        assert!(!stop("studio:a").stop_waits_for("studio:b"), "another chat's stop stays");
        let mut running = stop("studio:a");
        running.update(&ApiJob { state: Some("running".into()), ..Default::default() }, 1001.0);
        assert!(!running.stop_waits_for("studio:a"), "a stop under way cannot be called off");
        let mut off = stop("studio:a");
        off.call_off(1001.0);
        assert!(!off.stop_waits_for("studio:a"));
        let fresh = Job::new("s-2", JobKind::Fresh, Some("studio:a".into()), "Studio", "Studio", None, None, 1000.0);
        assert!(!fresh.stop_waits_for("studio:a"), "only stops");
    }

    #[test]
    fn compact_job_follows_the_servers_steps() {
        let mut j = Job::new(
            "s-0123456789ab",
            JobKind::Compact,
            Some("studio:x".into()),
            "Studio",
            "Studio",
            Some("Topic".into()),
            None,
            1000.0,
        );
        assert_eq!(j.detail(1001.0), "Gathering the thread's chats");
        j.update(&ApiJob { state: Some("running".into()), step: Some("summary".into()), ..Default::default() }, 1010.0);
        assert_eq!(j.detail(1010.0), "Writing one summary of the whole thread (a minute or two)");
        j.update(&ApiJob { state: Some("running".into()), step: Some("running".into()), ..Default::default() }, 1080.0);
        assert_eq!(j.detail(1080.0), "Starting the new chat on Studio");
        j.update(
            &ApiJob { state: Some("done".into()), name: Some("Topic (compacted)".into()), ..Default::default() },
            1090.0,
        );
        assert_eq!(j.headline(), "The new chat is ready");
    }

    #[test]
    fn fresh_job_on_a_busy_chat_says_it_is_waiting() {
        let mut j = Job::new(
            "s-0123456789ab",
            JobKind::Fresh,
            Some("studio:x".into()),
            "Studio",
            "Studio",
            Some("Topic".into()),
            None,
            1000.0,
        );
        let wait = |note: Option<&str>| ApiJob {
            state: Some("queued".into()),
            step: Some("waiting".into()),
            note: note.map(Into::into),
            ..Default::default()
        };
        j.update(&wait(Some("Claude is replying")), 1100.0);
        assert_eq!(j.detail(1300.0), "Topic. Claude is replying.");
        assert_eq!(j.headline(), "Starts fresh when its work is done");
        assert_eq!(j.at(1300.0), 0);
        j.update(&wait(None), 1310.0);
        assert_eq!(j.detail(1310.0), "Topic. Waiting for its work to finish.");
        j.update(&ApiJob { state: Some("running".into()), step: Some("summary".into()), ..Default::default() }, 1320.0);
        assert_eq!(j.waiting, None);
        assert_eq!(j.detail(1320.0), "Writing the handoff (about a minute)");
        j.update(
            &ApiJob {
                state: Some("error".into()),
                error: Some("Cancelled. Nothing was changed.".into()),
                ..Default::default()
            },
            1330.0,
        );
        assert_eq!(j.waiting, None);
        assert_eq!(j.detail(1330.0), "Topic. Nothing was changed.");
        assert_eq!(j.headline(), "Start fresh called off");
    }

    #[test]
    fn queued_stop_waits_then_stops_or_keeps_running() {
        let mut j = Job::new(
            "s-0123456789ab",
            JobKind::Stop,
            Some("studio:x".into()),
            "Studio",
            "Studio",
            Some("Soap (2)".into()),
            None,
            1000.0,
        );
        j.update(
            &ApiJob {
                state: Some("queued".into()),
                step: Some("waiting".into()),
                note: Some("Claude is replying".into()),
                ..Default::default()
            },
            1001.0,
        );
        assert_eq!(j.headline(), "Stops when its work is done");
        assert_eq!(j.state_word(), "Stops after this work");
        let mut kept = j.clone();
        j.update(&ApiJob { state: Some("done".into()), ..Default::default() }, 1050.0);
        assert_eq!(j.headline(), "Stopped");
        assert_eq!(j.detail(1050.0), "Soap, on Studio. Resume it any time.");
        kept.call_off(1010.0);
        assert!(kept.cancelled);
        assert_eq!(kept.headline(), "Kept running");
    }

    #[test]
    fn fresh_job_walks_its_steps() {
        let mut j = Job::new(
            "s-0123456789ab",
            JobKind::Fresh,
            Some("studio:x".into()),
            "Studio",
            "Studio",
            Some("Topic".into()),
            Some("Topic (fresh)".into()),
            1000.0,
        );
        assert_eq!(j.detail(1001.0), "Writing the handoff (about a minute)");
        j.update(&ApiJob { state: Some("running".into()), ..Default::default() }, 1060.0);
        assert_eq!(j.detail(1060.0), "Starting the new chat");
        j.update(
            &ApiJob {
                state: Some("done".into()),
                name: Some("Oct 11 3:40p Topic (fresh)".into()),
                sid: Some("y".into()),
                machine: Some("studio".into()),
                ..Default::default()
            },
            1090.0,
        );
        assert_eq!(j.headline(), "The new chat is ready");
        assert_eq!(j.detail(1090.0), "Topic, on Studio");
        assert_eq!(j.ended, Some(1090.0));
    }

    #[test]
    fn continue_and_errors() {
        let mut j = Job::new("s-1", JobKind::Continue, None, "Laptop", "Studio", None, None, 0.0);
        assert_eq!(j.detail(1.0), "Writing the handoff (about a minute)");
        j.update(&ApiJob { state: Some("running".into()), step: Some("start".into()), ..Default::default() }, 2.0);
        assert_eq!(j.detail(2.0), "Starting on Laptop");
        j.update(&ApiJob { state: Some("running".into()), step: Some("retrying".into()), ..Default::default() }, 3.0);
        assert_eq!(j.detail(3.0), "Studio isn't answering yet. Still trying.");
        j.update(
            &ApiJob { state: Some("error".into()), error: Some("Laptop is offline".into()), ..Default::default() },
            4.0,
        );
        assert_eq!(j.headline(), "Continue elsewhere didn't finish");
        assert_eq!(j.detail(4.0), "Laptop is offline");
    }

    #[test]
    fn auto_job_follows_the_pick() {
        let mut j = Job::new("s-2", JobKind::Auto, None, AUTO, AUTO, None, Some("Topic".into()), 0.0);
        assert_eq!(j.detail(1.0), "Claude picks the machine");
        assert_eq!(j.pick(), Some(("Auto".into(), "Claude is reading what each machine can do.".into())));
        j.update(
            &ApiJob {
                state: Some("queued".into()),
                step: Some("choosing".into()),
                machine: Some("auto".into()),
                label: Some("auto".into()),
                ..Default::default()
            },
            2.0,
        );
        assert_eq!(j.label, AUTO);
        j.update(
            &ApiJob {
                state: Some("queued".into()),
                step: Some("sent".into()),
                machine: Some("laptop".into()),
                label: Some("Laptop".into()),
                picked: Some("Auto picked Laptop. Has Xcode 26 and your iPhone connected.".into()),
                by: Some("claude".into()),
                ..Default::default()
            },
            3.0,
        );
        assert_eq!(j.detail(3.0), "Sent to Laptop");
        assert_eq!(j.pick(), Some(("Claude picked".into(), "Laptop. Has Xcode 26 and your iPhone connected.".into())));
        j.update(&ApiJob { state: Some("running".into()), by: Some("rules".into()), ..Default::default() }, 4.0);
        assert_eq!(j.detail(4.0), "Naming and starting it");
        assert_eq!(j.pick().unwrap().0, "Picked by the rules");
        j.update(
            &ApiJob {
                state: Some("done".into()),
                name: Some("Topic".into()),
                sid: Some("y".into()),
                machine: Some("laptop".into()),
                label: Some("Laptop".into()),
                ..Default::default()
            },
            5.0,
        );
        assert_eq!(j.detail(5.0), "Topic, on Laptop");
    }
}
