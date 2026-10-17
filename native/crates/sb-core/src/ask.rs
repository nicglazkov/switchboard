//! What a waiting chat needs from you, shaped by kind (docs/specs/2026-10-13-needs-shaped-replies.md), and the one
//! message each answer becomes. The agent builds the `ask` object; the server passes it through on session rows.
//!
//! - `Ask::from_api` cleans the wire object: unknown kinds, a fix the app cannot do, or a kind missing what it needs
//!   become `open` (a free reply), and every list and string is capped so a bad agent never breaks a card.
//! - The composers turn the controls into the message typed into the chat's terminal, with the same words as iPhone.

use serde::{Deserialize, Serialize};

use crate::api::{ApiAsk, ApiAskQuestion};

/// The fix actions the app can really run (v1). Any other fix becomes `open`.
pub const FIXES: [&str; 1] = ["relaunch_chrome"];
const SUMMARY_MAX: usize = 300;
const LABEL_MAX: usize = 80;
const REPLY_MAX: usize = 300;
const QUESTION_MAX: usize = 160;
const STEP_MAX: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AskKind {
    Choose,
    Questions,
    Task,
    Fix,
    Open,
}

impl AskKind {
    /// The tag on the card and in the details sheet.
    pub fn label(self) -> &'static str {
        match self {
            AskKind::Choose => "Choose",
            AskKind::Questions => "Questions",
            AskKind::Task => "Do this, then confirm",
            AskKind::Fix => "Fix setup",
            AskKind::Open => "Open question",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskOption {
    pub label: String,
    /// What picking it sends (the label when the agent sent none).
    pub reply: String,
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskQuestion {
    pub q: String,
    pub options: Vec<String>,
    pub multi: bool,
    /// About you: nothing is pre-selected and nothing is guessed.
    pub about_you: bool,
    /// The option Claude itself suggested (never when `about_you`).
    pub recommended: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AskFix {
    /// One of `FIXES`.
    pub action: String,
    pub label: String,
    /// The second button: what to send instead ("Chrome is open now. Try again.").
    pub alt_reply: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ask {
    pub kind: AskKind,
    pub summary: String,
    pub options: Vec<AskOption>,
    pub assumptions: Vec<String>,
    pub questions: Vec<AskQuestion>,
    pub steps: Vec<String>,
    pub done_reply: String,
    pub fix: Option<AskFix>,
    /// question_tool | haiku | claude_code, when the agent said.
    pub source: Option<String>,
    /// What the ask was built from (the transcript's mtime), so a new ask resets the controls.
    pub at: String,
}

/// What the board card's one button does.
#[derive(Debug, Clone, PartialEq)]
pub enum Quick {
    /// Types this into the chat.
    Send(String),
    /// Runs the fix (relaunch with Chrome, after the cost confirm).
    Fix,
    /// Selects the chat: the details sheet opens on the card.
    Answer,
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn cap(s: &str, max: usize) -> String {
    let t = one_line(s);
    if t.chars().count() <= max { t } else { format!("{}...", t.chars().take(max.saturating_sub(3)).collect::<String>().trim_end()) }
}

fn text(v: Option<&str>, max: usize) -> Option<String> {
    v.map(|s| cap(s, max)).filter(|s| !s.is_empty())
}

fn texts(v: &[String], max: usize, n: usize) -> Vec<String> {
    v.iter().filter_map(|s| text(Some(s), max)).take(n).collect()
}

fn question(q: &ApiAskQuestion) -> Option<AskQuestion> {
    let text_q = text(q.q.as_deref(), QUESTION_MAX)?;
    let options = texts(&q.options, LABEL_MAX, 5);
    let about_you = q.about_you.unwrap_or(false);
    let recommended = if about_you {
        None
    } else {
        match &q.recommended {
            Some(serde_json::Value::Number(n)) => n.as_u64().map(|i| i as usize).filter(|i| *i < options.len()),
            Some(serde_json::Value::String(s)) => options.iter().position(|o| o.eq_ignore_ascii_case(s.trim())),
            _ => None,
        }
    };
    Some(AskQuestion { q: text_q, options, multi: q.multi.unwrap_or(false), about_you, recommended })
}

impl Ask {
    /// A free reply about `summary` (a need from an older server, or nothing known at all).
    pub fn open(summary: Option<&str>) -> Ask {
        Ask {
            kind: AskKind::Open,
            summary: text(summary, SUMMARY_MAX).unwrap_or_else(|| "Claude is waiting for your answer.".into()),
            options: Vec::new(),
            assumptions: Vec::new(),
            questions: Vec::new(),
            steps: Vec::new(),
            done_reply: String::new(),
            fix: None,
            source: None,
            at: String::new(),
        }
    }

    /// The wire object cleaned up. `need` stands in for a missing summary.
    pub fn from_api(a: &ApiAsk, need: Option<&str>) -> Ask {
        let mut k = Ask::open(a.summary.as_deref().filter(|s| !s.trim().is_empty()).or(need));
        k.source = text(a.source.as_deref(), 40);
        k.at = match &a.at {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => String::new(),
        };
        match a.kind.as_deref().map(str::trim) {
            Some("choose") => {
                let options: Vec<AskOption> = a
                    .options
                    .iter()
                    .filter_map(|o| {
                        let label = text(o.label.as_deref(), LABEL_MAX)?;
                        let reply = text(o.reply.as_deref(), REPLY_MAX).unwrap_or_else(|| label.clone());
                        Some(AskOption { label, reply, recommended: o.recommended.unwrap_or(false) })
                    })
                    .take(4)
                    .collect();
                if !options.is_empty() {
                    k.kind = AskKind::Choose;
                    k.options = options;
                    k.assumptions = texts(&a.assumptions, QUESTION_MAX, 6);
                }
            }
            Some("questions") => {
                let qs: Vec<AskQuestion> = a.questions.iter().filter_map(question).take(5).collect();
                if !qs.is_empty() {
                    k.kind = AskKind::Questions;
                    k.questions = qs;
                }
            }
            Some("task") => {
                let steps = texts(&a.steps, STEP_MAX, 8);
                if !steps.is_empty() {
                    k.kind = AskKind::Task;
                    k.steps = steps;
                    k.done_reply = text(a.done_reply.as_deref(), REPLY_MAX).unwrap_or_else(|| "Done.".into());
                }
            }
            Some("fix") => {
                if let Some(f) = a.fix.as_ref().filter(|f| f.action.as_deref().is_some_and(|x| FIXES.contains(&x.trim()))) {
                    k.kind = AskKind::Fix;
                    k.fix = Some(AskFix {
                        action: f.action.clone().unwrap_or_default().trim().to_string(),
                        label: text(f.label.as_deref(), LABEL_MAX).unwrap_or_else(|| "Restart this chat with Chrome".into()),
                        alt_reply: text(f.alt_reply.as_deref(), REPLY_MAX),
                    });
                }
            }
            _ => {}
        }
        k
    }

    /// The option picked at first: the recommended one, else the first.
    pub fn default_pick(&self) -> usize {
        self.options.iter().position(|o| o.recommended).unwrap_or(0)
    }

    /// Each question's answers at first: Claude's suggestion where it has one, nothing about you.
    pub fn default_answers(&self) -> Vec<Vec<usize>> {
        self.questions.iter().map(|q| q.recommended.into_iter().collect()).collect()
    }

    /// The line under the tag: "1 choice + 4 assumptions", "Claude asked 3 things", "5 steps".
    pub fn caption(&self) -> Option<String> {
        let n = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        match self.kind {
            AskKind::Choose if self.assumptions.is_empty() => Some(n(self.options.len(), "option", "options")),
            AskKind::Choose => Some(format!("{} + {}", n(self.options.len(), "option", "options"), n(self.assumptions.len(), "assumption", "assumptions"))),
            AskKind::Questions => Some(format!("Claude asked {}", n(self.questions.len(), "thing", "things"))),
            AskKind::Task => Some(n(self.steps.len(), "step", "steps")),
            AskKind::Fix | AskKind::Open => None,
        }
    }

    /// The board card's one button: its label and what it does. A choice with assumptions to check, questions and
    /// open replies need the full card, so the button only selects the chat.
    pub fn quick(&self) -> (String, Quick) {
        match self.kind {
            AskKind::Choose if self.assumptions.is_empty() => match self.options.iter().position(|o| o.recommended) {
                Some(i) => (self.options[i].label.clone(), Quick::Send(compose_choose(self, i, &[], "").unwrap_or_default())),
                None => ("Answer...".into(), Quick::Answer),
            },
            AskKind::Task => ("Done".into(), Quick::Send(compose_task(self))),
            AskKind::Fix => (self.fix.as_ref().map(|f| f.label.clone()).unwrap_or_default(), Quick::Fix),
            _ => ("Answer...".into(), Quick::Answer),
        }
    }
}

fn no_dot(s: &str) -> &str {
    s.trim().trim_end_matches(['.', ';'])
}

/// choose: the option's reply, then what you said about its assumptions (`ticked[i]` false: wrong; missing: right),
/// then your note.
pub fn compose_choose(a: &Ask, pick: usize, ticked: &[bool], note: &str) -> Option<String> {
    let o = a.options.get(pick)?;
    let mut m = o.reply.clone();
    if !a.assumptions.is_empty() {
        let wrong: Vec<&str> = a.assumptions.iter().enumerate().filter(|(i, _)| !ticked.get(*i).copied().unwrap_or(true)).map(|(_, s)| no_dot(s)).collect();
        if wrong.is_empty() {
            m.push_str(" Your assumptions are right.");
        } else {
            m.push_str(&format!(" These assumptions are wrong: {}.", wrong.join("; ")));
        }
    }
    let note = one_line(note);
    if !note.is_empty() {
        m.push(' ');
        m.push_str(&note);
    }
    Some(m)
}

/// questions: each question's answers (`picks[i]`: option indexes, then what was typed), or None while one is still
/// unanswered.
fn answers(a: &Ask, picks: &[Vec<usize>], typed: &[String]) -> Vec<Option<String>> {
    a.questions
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let mut v: Vec<String> = Vec::new();
            if let Some(p) = picks.get(i) {
                for (j, o) in q.options.iter().enumerate() {
                    if p.contains(&j) {
                        v.push(o.clone());
                    }
                }
            }
            if let Some(t) = typed.get(i).map(|t| one_line(t)).filter(|t| !t.is_empty()) {
                v.push(t);
            }
            if v.is_empty() { None } else { Some(v.join(", ")) }
        })
        .collect()
}

/// Every question has an answer, by chip or typed: Send is on.
pub fn questions_ready(a: &Ask, picks: &[Vec<usize>], typed: &[String]) -> bool {
    !a.questions.is_empty() && answers(a, picks, typed).iter().all(Option::is_some)
}

/// "1) No  2) Dutch oven, Cast iron  3) Yes, that's fine". None until every question is answered.
pub fn compose_questions(a: &Ask, picks: &[Vec<usize>], typed: &[String]) -> Option<String> {
    let parts: Option<Vec<String>> = answers(a, picks, typed).into_iter().enumerate().map(|(i, x)| x.map(|x| format!("{}) {x}", i + 1))).collect();
    parts.filter(|p| !p.is_empty()).map(|p| p.join("  "))
}

/// task: Done sends the done reply.
pub fn compose_task(a: &Ask) -> String {
    if a.done_reply.is_empty() { "Done.".into() } else { a.done_reply.clone() }
}

/// fix: the second button's message.
pub fn compose_fix_alt(a: &Ask) -> Option<String> {
    a.fix.as_ref()?.alt_reply.clone()
}

/// open, and "Hit a problem...": what you typed, on one line (Enter in the terminal sends it). None when empty.
pub fn compose_free(text: &str) -> Option<String> {
    Some(one_line(text)).filter(|t| !t.is_empty())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn api(json: &str) -> ApiAsk {
        serde_json::from_str(json).unwrap()
    }

    fn choose() -> Ask {
        Ask::from_api(
            &api(
                r#"{"kind": "choose", "summary": "Pick option 1 or 2", "options": [
                    {"label": "Option 1: new Builder window", "reply": "Option 1.", "recommended": true},
                    {"label": "Option 2: grow the Vibes editor", "reply": "Option 2."}],
                  "assumptions": ["OR within a field, AND across fields", "Songs added by search have no tags.", "Saving again only adds songs"],
                  "source": "haiku", "for": 1791782309.5}"#,
            ),
            None,
        )
    }

    fn questions() -> Ask {
        Ask::from_api(
            &api(
                r#"{"kind": "questions", "summary": "Answer 3 questions", "questions": [
                    {"q": "Are you a Cook Well member?", "options": ["Yes", "No"], "about_you": true, "recommended": 0},
                    {"q": "What pots and pans do you have?", "options": ["Dutch oven", "Big stockpot", "Sheet pans", "Cast iron"], "multi": true, "about_you": true},
                    {"q": "OK to braise raw pork or beef the day you buy it?", "options": ["Yes, that's fine", "No, buy it a day ahead"], "recommended": 0}]}"#,
            ),
            None,
        )
    }

    #[test]
    fn choose_with_every_assumption_ticked() {
        let a = choose();
        assert_eq!(a.kind, AskKind::Choose);
        assert_eq!(a.at, "1791782309.5");
        assert_eq!(a.default_pick(), 0);
        assert_eq!(compose_choose(&a, 0, &[true, true, true], "").unwrap(), "Option 1. Your assumptions are right.");
        // missing ticks count as right
        assert_eq!(compose_choose(&a, 1, &[], "  ").unwrap(), "Option 2. Your assumptions are right.");
        assert_eq!(a.caption().unwrap(), "2 options + 3 assumptions");
    }

    #[test]
    fn choose_with_some_assumptions_unticked_and_a_note() {
        let a = choose();
        assert_eq!(
            compose_choose(&a, 0, &[true, false, false], "Also show\n the total time.").unwrap(),
            "Option 1. These assumptions are wrong: Songs added by search have no tags; Saving again only adds songs. Also show the total time."
        );
        assert_eq!(compose_choose(&a, 7, &[], ""), None);
    }

    #[test]
    fn choose_without_assumptions_says_nothing_about_them() {
        let a = Ask::from_api(&api(r#"{"kind": "choose", "options": [{"label": "Ship it"}, {"label": "Wait", "recommended": true}]}"#), Some("Ship or wait?"));
        assert_eq!(a.summary, "Ship or wait?");
        assert_eq!(compose_choose(&a, 0, &[], "now").unwrap(), "Ship it now");
        assert_eq!(a.quick(), ("Wait".to_string(), Quick::Send("Wait".into())));
        // with assumptions to check, the card's button only opens the sheet
        assert_eq!(choose().quick().1, Quick::Answer);
    }

    #[test]
    fn questions_need_every_answer() {
        let a = questions();
        assert_eq!(a.kind, AskKind::Questions);
        // about you: never pre-selected, even when the agent named one
        assert_eq!(a.questions[0].recommended, None);
        assert_eq!(a.default_answers(), vec![vec![], vec![], vec![0]]);
        let none = vec![String::new(); 3];
        assert!(!questions_ready(&a, &a.default_answers(), &none));
        assert_eq!(compose_questions(&a, &a.default_answers(), &none), None);
        let picks = vec![vec![1], vec![3, 0], vec![0]];
        assert!(questions_ready(&a, &picks, &none));
        assert_eq!(compose_questions(&a, &picks, &none).unwrap(), "1) No  2) Dutch oven, Cast iron  3) Yes, that's fine");
        // typed answers count, and join the chips
        let typed = vec!["Not yet".to_string(), " a wok ".to_string(), String::new()];
        assert_eq!(compose_questions(&a, &[vec![], vec![2], vec![1]], &typed).unwrap(), "1) Not yet  2) Sheet pans, a wok  3) No, buy it a day ahead");
        assert_eq!(a.caption().unwrap(), "Claude asked 3 things");
        assert_eq!(a.quick().1, Quick::Answer);
    }

    #[test]
    fn task_done_and_fix_alt() {
        let a = Ask::from_api(&api(r#"{"kind": "task", "steps": ["Plug it in", " "], "done_reply": "root.pem is on the Kitchen remodel."}"#), Some("Load root.pem"));
        assert_eq!(a.steps, vec!["Plug it in"]);
        assert_eq!(compose_task(&a), "root.pem is on the Kitchen remodel.");
        assert_eq!(a.quick(), ("Done".to_string(), Quick::Send("root.pem is on the Kitchen remodel.".into())));
        let a = Ask::from_api(&api(r#"{"kind": "task", "steps": ["x"]}"#), None);
        assert_eq!(compose_task(&a), "Done.");

        let f = Ask::from_api(&api(r#"{"kind": "fix", "fix": {"action": "relaunch_chrome", "label": "Restart this chat with Chrome", "alt_reply": "Chrome is open now. Try again."}}"#), None);
        assert_eq!(f.kind, AskKind::Fix);
        assert_eq!(compose_fix_alt(&f).unwrap(), "Chrome is open now. Try again.");
        assert_eq!(f.quick(), ("Restart this chat with Chrome".to_string(), Quick::Fix));
    }

    #[test]
    fn anything_the_app_cannot_do_becomes_open() {
        for j in [
            r#"{"kind": "fix", "fix": {"action": "reboot_machine", "label": "Reboot"}}"#,
            r#"{"kind": "choose", "options": []}"#,
            r#"{"kind": "questions", "questions": [{"options": ["a"]}]}"#,
            r#"{"kind": "task", "steps": []}"#,
            r#"{"kind": "dance"}"#,
            r#"{}"#,
        ] {
            let a = Ask::from_api(&api(j), Some("the need"));
            assert_eq!(a.kind, AskKind::Open, "{j}");
            assert_eq!(a.summary, "the need");
        }
        assert_eq!(compose_free("  line one\nline two  ").unwrap(), "line one line two");
        assert_eq!(compose_free(" \n "), None);
        assert_eq!(Ask::open(None).summary, "Claude is waiting for your answer.");
    }

    #[test]
    fn odd_shapes_and_long_strings_are_capped() {
        let long = "x".repeat(400);
        let j = serde_json::json!({
            "kind": "questions", "summary": long, "options": 5, "fix": "nope",
            "questions": (0..7).map(|i| serde_json::json!({"q": format!("{i} {long}"), "options": (0..9).map(|_| long.clone()).collect::<Vec<_>>(), "multi": "yes"})).collect::<Vec<_>>()
        });
        let a = Ask::from_api(&serde_json::from_value(j).unwrap(), None);
        assert_eq!(a.questions.len(), 5);
        assert_eq!(a.questions[0].options.len(), 5);
        assert!(a.questions[0].q.chars().count() <= QUESTION_MAX);
        assert!(a.questions[0].options[0].chars().count() <= LABEL_MAX);
        assert!(a.summary.chars().count() <= SUMMARY_MAX);
        assert!(!a.questions[0].multi);
    }
}
