//! Compact long chats at: the one setting for how big a chat may grow before Claude Code compacts it (replaces the
//! older part with a summary). The server keeps a switch (`setup_compact_window`) and a value (`compact_at`); here
//! they are one choice on one slider, with the words that explain it. Everything is plain data, so it is tested
//! here and the desktop only draws it.
//!
//! What Claude Code does (code.claude.com/docs/en/model-config, "Set the auto-compact window"): the window is
//! 100k to 1M tokens, it is capped at the model's own context window, and with nothing set today's models (1M
//! windows) compact at about 967k.

use crate::api::{ApiCompactAt, ApiCompactSeen};

/// Tokens in short: "300k", "1M", "1.9M".
pub fn k(n: u64) -> String {
    let s = crate::fmt::k(n);
    s.strip_suffix(".0M").map_or(s.clone(), |whole| format!("{whole}M"))
}

/// The range Claude Code takes.
pub const MIN: u64 = 100_000;
pub const MAX: u64 = 1_000_000;
/// The value recommended, and what the switch meant before it had a value.
pub const RECOMMENDED: u64 = 300_000;
/// The slider's stops with a value; one more stop after them is Claude Code's default.
pub const STOPS: [u64; 4] = [150_000, 200_000, 300_000, 500_000];
/// Where Claude Code compacts a model with a 1M window when nothing is set.
pub const DEFAULT_ABOUT: u64 = 967_000;
/// The window of the models that do not have 1M (Opus 4.6 and Sonnet 4.6 without `[1m]`, and older ones).
pub const SMALL_WINDOW: u64 = 200_000;
/// A chat with no known limit is measured against this, as every chat was before the value existed.
pub const FALLBACK: u64 = 300_000;
/// The helper feature that says a computer's helper writes any value (an older one writes 300000 only).
pub const FEATURE: &str = "compact_at";

/// The cost weights the app uses everywhere (docs/chats.md): writing the cache again on a cold resume.
const WRITE_1H: f64 = 2.0;
const WRITE_5M: f64 = 1.25;

/// Where the slider is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// Switchboard sets this many tokens on every computer.
    At(u64),
    /// Switchboard sets nothing: Claude Code compacts where it does by default.
    Default,
}

/// A value held to what Claude Code takes.
pub fn clamp(v: u64) -> u64 {
    v.clamp(MIN, MAX)
}

/// Past this a chat is close to being compacted anyway, so Start fresh is the better pick: 90% of its limit.
pub fn near(limit: u64) -> u64 {
    limit / 10 * 9
}

impl Choice {
    /// From the server: the switch and its value. A server from before the value existed sends only the switch,
    /// which meant 300k.
    pub fn from_server(on: bool, at: Option<u64>) -> Choice {
        if on { Choice::At(clamp(at.unwrap_or(RECOMMENDED))) } else { Choice::Default }
    }

    pub fn of(c: &ApiCompactAt) -> Choice {
        Choice::from_server(c.on, Some(c.at))
    }

    /// The slider has this many positions: each stop, then the default.
    pub fn positions() -> usize {
        STOPS.len() + 1
    }

    /// The choice at one position of the slider (anything past the last stop is the default).
    pub fn at_position(i: usize) -> Choice {
        STOPS.get(i).map_or(Choice::Default, |v| Choice::At(*v))
    }

    /// Where the thumb sits, from 0 to `positions() - 1`. A value that is not a stop (saved another way) sits
    /// between its neighbours, in proportion.
    pub fn position(self) -> f32 {
        let last = (STOPS.len() - 1) as f32;
        match self {
            Choice::Default => last + 1.,
            Choice::At(v) if v <= STOPS[0] => 0.,
            Choice::At(v) if v >= STOPS[STOPS.len() - 1] => {
                // past the last stop: toward the default, which stands for about 967k
                let top = STOPS[STOPS.len() - 1];
                last + ((v - top) as f32 / (DEFAULT_ABOUT - top) as f32).min(0.9)
            }
            Choice::At(v) => {
                let i = STOPS.iter().rposition(|s| *s <= v).unwrap_or(0);
                i as f32 + (v - STOPS[i]) as f32 / (STOPS[i + 1] - STOPS[i]) as f32
            }
        }
    }

    /// The nearest position to where the thumb was let go.
    pub fn snap(pos: f32) -> Choice {
        let max = (Choice::positions() - 1) as f32;
        Choice::at_position(if pos.is_finite() { pos.clamp(0., max).round() as usize } else { 2 })
    }

    /// One step along the slider: the next stop up or down from here (from between two stops, the nearer side).
    pub fn step(self, up: bool) -> Choice {
        let p = self.position();
        let max = Choice::positions() - 1;
        let i = if up { (p.floor() as usize + 1).min(max) } else { (p.ceil() as usize).saturating_sub(1) };
        Choice::at_position(i)
    }

    pub fn first() -> Choice {
        Choice::at_position(0)
    }

    pub fn last() -> Choice {
        Choice::Default
    }

    pub fn is_recommended(self) -> bool {
        self == Choice::At(RECOMMENDED)
    }

    /// What the server is sent: the switch, and the value when there is one (the default stop keeps the last value).
    pub fn body(self) -> serde_json::Value {
        match self {
            Choice::At(v) => serde_json::json!({"setup_compact_window": true, "compact_at": clamp(v)}),
            Choice::Default => serde_json::json!({"setup_compact_window": false}),
        }
    }

    /// The value beside the slider: "300k", or "Not set" on the default stop (its name says whose default).
    pub fn value_label(self) -> String {
        match self {
            Choice::At(v) => k(v),
            Choice::Default => "Not set".into(),
        }
    }

    /// The short label under a stop.
    pub fn stop_label(self) -> String {
        match self {
            Choice::At(v) => k(v),
            Choice::Default => "Default".into(),
        }
    }

    /// What the choice is called: the presets are the stops' names, not a second control.
    pub fn name(self) -> &'static str {
        match self {
            Choice::At(v) if v < SMALL_WINDOW => "Lean",
            Choice::At(v) if v < RECOMMENDED => "Standard",
            Choice::At(v) if v < STOPS[STOPS.len() - 1] => "Balanced",
            Choice::At(_) => "Long memory",
            Choice::Default => "Claude Code's default",
        }
    }

    /// What the choice means in practice, in one short paragraph that changes with the value: how much stays word
    /// for word, what coming back to a cold chat that size costs by the app's own weights, and what a model with a
    /// smaller window does.
    pub fn sentence(self) -> String {
        let cost = |n: u64| {
            format!(
                "about {} of usage on the 1-hour cache, {} on the 5-minute one",
                k((n as f64 * WRITE_1H) as u64),
                k((n as f64 * WRITE_5M) as u64)
            )
        };
        match self {
            Choice::At(v) => {
                let mut s = format!(
                    "A chat keeps everything word for word up to {}. Past that, Claude Code replaces the older part with a summary. Coming back to a cold chat that size re-reads all of it: {}.",
                    k(v),
                    cost(v)
                );
                if v > SMALL_WINDOW {
                    s.push_str(&format!(
                        " A model with a {} window still compacts at {}.",
                        k(SMALL_WINDOW),
                        k(SMALL_WINDOW)
                    ));
                }
                s
            }
            Choice::Default => format!(
                "Switchboard sets nothing. Today's models then compact at about {}, so a chat can keep nearly a million tokens word for word, and coming back to a cold one that size costs {}. Pick this to set a window per model yourself, with /autocompact in Claude Code.",
                k(DEFAULT_ABOUT),
                cost(DEFAULT_ABOUT)
            ),
        }
    }

    /// What else moves with the choice, as label and value: nothing here is a setting of its own.
    pub fn follows(self) -> [(&'static str, String); 2] {
        let (fill, fresh) = match self {
            Choice::At(v) if v > SMALL_WINDOW => {
                (format!("{}, or the model's window when that is smaller", k(v)), k(near(v)))
            }
            Choice::At(v) => (k(v), k(near(v))),
            Choice::Default => ("The model's window".into(), "Nine tenths of the model's window".into()),
        };
        [("Context meters fill at", fill), ("Start fresh is suggested from", fresh)]
    }

    /// Exactly what the choice does on each computer.
    pub fn writes(self) -> String {
        match self {
            Choice::At(v) => format!(
                "Writes CLAUDE_CODE_AUTO_COMPACT_WINDOW={} to the Claude Code settings on each computer. A chat picks it up the next time it starts.",
                clamp(v)
            ),
            Choice::Default => "Removes the value Switchboard wrote from the Claude Code settings on each computer, and nothing else. A chat picks it up the next time it starts.".into(),
        }
    }
}

/// How one computer's line reads on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// As picked.
    Ok,
    /// On its way: the computer has not answered since the change, or is offline.
    Wait,
    /// The user's own value there: left alone, whatever is picked.
    Theirs,
    /// Its helper is older and only knows 300k, which is not what was picked.
    Older,
    /// Its Claude Code settings could not be read, so nothing was changed.
    Unread,
}

impl Tone {
    /// The short tag beside a computer that is not as picked and will stay so.
    pub fn tag(self) -> Option<&'static str> {
        match self {
            Tone::Ok | Tone::Wait => None,
            Tone::Theirs => Some("Left alone"),
            Tone::Older => Some("Older helper"),
            Tone::Unread => Some("Not changed"),
        }
    }
}

/// One computer's line: what it has now, against what was picked. `seen` is what its helper read from its Claude
/// Code settings (None from an older helper, or when the file could not be read), `knows` whether its helper
/// writes any value (the `compact_at` feature), `online` as the server last saw it.
pub fn machine_line(
    choice: Choice,
    seen: Option<&ApiCompactSeen>,
    knows: bool,
    online: Option<bool>,
) -> (String, Tone) {
    let off = online == Some(false);
    if !knows {
        // an older helper: on means 300000 there, and it does not say what the computer has
        return match choice {
            Choice::At(v) if v == RECOMMENDED => {
                ("300k. Its helper is older and does not report back.".into(), Tone::Ok)
            }
            Choice::At(v) => (
                format!("300k, not {}: its helper is older and only knows 300k. Update Switchboard there.", k(v)),
                Tone::Older,
            ),
            Choice::Default => {
                ("Nothing set by Switchboard. Its helper is older and does not report back.".into(), Tone::Ok)
            }
        };
    }
    let Some(seen) = seen else {
        return ("Its Claude Code settings could not be read, so nothing was changed there.".into(), Tone::Unread);
    };
    let theirs = |what: String| (format!("Set on this computer: {what}, left alone."), Tone::Theirs);
    if seen.set && !seen.own {
        return theirs(match (seen.value, seen.raw.as_deref()) {
            // the words as written only when they do not read as the value ("200", held to 100k)
            (Some(v), Some(raw)) if raw.trim() != v.to_string() && !raw.trim().eq_ignore_ascii_case(&k(v)) => {
                format!("{} (written as {})", k(v), raw.trim())
            }
            (Some(v), _) => k(v),
            (None, Some(raw)) => format!("\"{}\", which Claude Code ignores", raw.trim()),
            (None, None) => "a value of its own".into(),
        });
    }
    if !seen.set {
        match seen.other.as_deref() {
            Some("setting") => return theirs("a window in Claude Code's settings (autoCompactWindow)".into()),
            Some("models") => return theirs("its own window per model (/autocompact)".into()),
            _ => {}
        }
    }
    let wait =
        |s: &str| (if off { format!("Offline. {s} when it is back.") } else { format!("{s}\u{2026}") }, Tone::Wait);
    match (choice, seen.set, seen.value) {
        (Choice::At(v), true, Some(have)) if have == v => (format!("{}, set by Switchboard.", k(v)), Tone::Ok),
        (Choice::At(v), true, _) => wait(&format!("Changing to {}", k(v))),
        (Choice::At(v), false, _) => wait(&format!("Setting {}", k(v))),
        (Choice::Default, true, _) => wait("Removing Switchboard's value"),
        (Choice::Default, false, _) => ("Nothing set, so Claude Code's default applies.".into(), Tone::Ok),
    }
}

/// The context label beside a meter: "98k of 300k", against the chat's own limit.
pub fn of_limit(ctx: u64, limit: u64) -> String {
    format!("{} of {}", crate::fmt::k(ctx), k(limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(set: bool, own: bool, value: Option<u64>, raw: Option<&str>, other: Option<&str>) -> ApiCompactSeen {
        ApiCompactSeen { set, own, value, raw: raw.map(String::from), other: other.map(String::from) }
    }

    #[test]
    fn the_old_switch_means_300k_and_off_means_the_default() {
        assert_eq!(Choice::from_server(true, None), Choice::At(300_000), "a server from before the value existed");
        assert_eq!(Choice::from_server(true, Some(500_000)), Choice::At(500_000));
        assert_eq!(Choice::from_server(false, Some(500_000)), Choice::Default);
        assert_eq!(Choice::from_server(false, None), Choice::Default);
    }

    #[test]
    fn a_value_out_of_range_is_held_to_what_claude_code_takes() {
        assert_eq!(Choice::from_server(true, Some(5)), Choice::At(MIN));
        assert_eq!(Choice::from_server(true, Some(u64::MAX)), Choice::At(MAX));
        assert_eq!((clamp(0), clamp(100_000), clamp(1_000_000), clamp(1_000_001)), (MIN, MIN, MAX, MAX));
    }

    #[test]
    fn the_server_view_reads_and_an_older_server_has_none() {
        let p: crate::api::ApiPrefs = serde_json::from_str(
            r#"{"ok": true, "compact": {"at": 500000, "on": true, "min": 100000, "max": 1000000, "recommended": 300000, "stops": [150000, 200000, 300000, 500000]}}"#,
        )
        .unwrap();
        assert_eq!(Choice::of(p.compact.as_ref().unwrap()), Choice::At(500_000));
        let old: crate::api::ApiPrefs = serde_json::from_str(r#"{"ok": true, "options": []}"#).unwrap();
        assert!(old.compact.is_none());
        let odd: crate::api::ApiPrefs = serde_json::from_str(r#"{"ok": true, "compact": "yes"}"#).unwrap();
        assert!(odd.compact.is_none(), "an answer of another shape reads as none, never as an error");
    }

    #[test]
    fn every_position_is_a_stop_and_every_stop_a_position() {
        assert_eq!(Choice::positions(), 5);
        for i in 0..Choice::positions() {
            let c = Choice::at_position(i);
            assert_eq!(c.position(), i as f32);
            assert_eq!(Choice::snap(i as f32), c);
        }
        assert_eq!(Choice::at_position(4), Choice::Default);
        assert_eq!(Choice::at_position(99), Choice::Default);
        assert!(Choice::at_position(2).is_recommended());
        assert_eq!((0..5).filter(|i| Choice::at_position(*i).is_recommended()).count(), 1);
    }

    #[test]
    fn a_value_between_stops_sits_between_them() {
        assert_eq!(Choice::At(250_000).position(), 1.5);
        assert_eq!(Choice::At(400_000).position(), 2.5);
        assert_eq!(Choice::At(100_000).position(), 0.0);
        let high = Choice::At(1_000_000).position();
        assert!(high > 3.0 && high < 4.0, "the top of the range is never drawn on the default stop: {high}");
        let p750 = Choice::At(750_000).position();
        assert!(p750 > 3.0 && p750 < high);
    }

    #[test]
    fn letting_go_lands_on_the_nearest_stop() {
        assert_eq!(Choice::snap(0.4), Choice::At(150_000));
        assert_eq!(Choice::snap(0.6), Choice::At(200_000));
        assert_eq!(Choice::snap(3.6), Choice::Default);
        assert_eq!(Choice::snap(-3.0), Choice::At(150_000));
        assert_eq!(Choice::snap(99.0), Choice::Default);
        assert_eq!(Choice::snap(f32::NAN), Choice::At(300_000));
    }

    #[test]
    fn the_arrow_keys_walk_the_stops_and_stop_at_the_ends() {
        let mut c = Choice::first();
        let mut seen = vec![c];
        for _ in 0..6 {
            c = c.step(true);
            seen.push(c);
        }
        assert_eq!(
            seen,
            vec![
                Choice::At(150_000),
                Choice::At(200_000),
                Choice::At(300_000),
                Choice::At(500_000),
                Choice::Default,
                Choice::Default,
                Choice::Default
            ]
        );
        assert_eq!(Choice::first().step(false), Choice::first());
        assert_eq!(Choice::Default.step(false), Choice::At(500_000));
        assert_eq!(Choice::last(), Choice::Default);
        // from between two stops, each arrow goes to the stop on its side
        assert_eq!(Choice::At(250_000).step(true), Choice::At(300_000));
        assert_eq!(Choice::At(250_000).step(false), Choice::At(200_000));
        assert_eq!(Choice::At(750_000).step(true), Choice::Default);
        assert_eq!(Choice::At(750_000).step(false), Choice::At(500_000));
        assert_eq!(Choice::At(100_000).step(false), Choice::At(150_000));
    }

    #[test]
    fn what_is_sent_is_the_switch_and_the_value() {
        assert_eq!(
            Choice::At(500_000).body(),
            serde_json::json!({"setup_compact_window": true, "compact_at": 500_000})
        );
        assert_eq!(Choice::Default.body(), serde_json::json!({"setup_compact_window": false}));
        assert_eq!(Choice::At(7).body()["compact_at"], 100_000, "never a value the server would refuse");
    }

    #[test]
    fn each_stop_has_its_name_and_exactly_one_is_recommended() {
        let names: Vec<_> = (0..5).map(|i| Choice::at_position(i).name()).collect();
        assert_eq!(names, ["Lean", "Standard", "Balanced", "Long memory", "Claude Code's default"]);
        let labels: Vec<_> = (0..5).map(|i| Choice::at_position(i).stop_label()).collect();
        assert_eq!(labels, ["150k", "200k", "300k", "500k", "Default"]);
        assert_eq!(Choice::At(300_000).value_label(), "300k");
        assert_eq!(Choice::Default.value_label(), "Not set");
        assert_eq!(Choice::At(250_000).name(), "Standard");
        assert_eq!(Choice::At(750_000).name(), "Long memory");
    }

    #[test]
    fn the_sentence_uses_the_apps_cost_weights() {
        let s = Choice::At(300_000).sentence();
        assert!(s.contains("word for word up to 300k"), "{s}");
        assert!(s.contains("about 600k of usage on the 1-hour cache, 375k on the 5-minute one"), "{s}");
        assert!(s.contains("A model with a 200k window still compacts at 200k."), "{s}");
        let s = Choice::At(150_000).sentence();
        assert!(s.contains("about 300k of usage on the 1-hour cache, 188k on the 5-minute one"), "{s}");
        assert!(!s.contains("still compacts"), "nothing to say about smaller windows at or under 200k: {s}");
        assert!(!Choice::At(200_000).sentence().contains("still compacts"));
        let s = Choice::At(500_000).sentence();
        assert!(s.contains("about 1M of usage on the 1-hour cache, 625k on the 5-minute one"), "{s}");
        let s = Choice::Default.sentence();
        assert!(s.starts_with("Switchboard sets nothing."), "{s}");
        assert!(s.contains("about 967k") && s.contains("/autocompact"), "{s}");
        assert!(s.contains("about 1.9M of usage on the 1-hour cache, 1.2M on the 5-minute one"), "{s}");
    }

    #[test]
    fn every_sentence_is_different_and_plain() {
        let all: Vec<String> = (0..5).map(|i| Choice::at_position(i).sentence()).collect();
        for (i, s) in all.iter().enumerate() {
            assert!(!s.contains('\u{2014}') && !s.contains("  "), "{s}");
            assert!(s.ends_with('.'), "{s}");
            assert!(all.iter().skip(i + 1).all(|o| o != s));
        }
    }

    #[test]
    fn start_fresh_is_suggested_at_nine_tenths_of_the_limit() {
        assert_eq!(near(300_000), 270_000, "as before the value existed");
        assert_eq!(near(500_000), 450_000);
        assert_eq!(near(200_000), 180_000);
        assert_eq!(near(0), 0);
    }

    #[test]
    fn a_computer_as_picked_says_so() {
        let ok = seen(true, true, Some(500_000), Some("500000"), None);
        assert_eq!(
            machine_line(Choice::At(500_000), Some(&ok), true, Some(true)),
            ("500k, set by Switchboard.".into(), Tone::Ok)
        );
        let none = seen(false, false, None, None, None);
        assert_eq!(
            machine_line(Choice::Default, Some(&none), true, Some(true)),
            ("Nothing set, so Claude Code's default applies.".into(), Tone::Ok)
        );
    }

    #[test]
    fn a_value_the_user_set_is_named_and_left_alone() {
        let theirs = seen(true, false, Some(500_000), Some("500000"), None);
        for c in [Choice::At(300_000), Choice::At(500_000), Choice::Default] {
            assert_eq!(
                machine_line(c, Some(&theirs), true, Some(true)),
                ("Set on this computer: 500k, left alone.".into(), Tone::Theirs)
            );
        }
        let k = seen(true, false, Some(200_000), Some("200k"), None);
        assert_eq!(
            machine_line(Choice::At(300_000), Some(&k), true, None).0,
            "Set on this computer: 200k, left alone."
        );
        let low = seen(true, false, Some(100_000), Some("200"), None);
        assert_eq!(
            machine_line(Choice::At(300_000), Some(&low), true, None).0,
            "Set on this computer: 100k (written as 200), left alone."
        );
        let odd = seen(true, false, None, Some("lots"), None);
        assert_eq!(
            machine_line(Choice::At(300_000), Some(&odd), true, None).0,
            "Set on this computer: \"lots\", which Claude Code ignores, left alone."
        );
        let setting = seen(false, false, None, None, Some("setting"));
        assert_eq!(
            machine_line(Choice::At(300_000), Some(&setting), true, None),
            (
                "Set on this computer: a window in Claude Code's settings (autoCompactWindow), left alone.".into(),
                Tone::Theirs
            )
        );
        let models = seen(false, false, None, None, Some("models"));
        assert_eq!(
            machine_line(Choice::Default, Some(&models), true, None).0,
            "Set on this computer: its own window per model (/autocompact), left alone."
        );
    }

    #[test]
    fn a_change_on_its_way_says_so_and_an_offline_computer_says_when() {
        let old = seen(true, true, Some(300_000), Some("300000"), None);
        assert_eq!(
            machine_line(Choice::At(500_000), Some(&old), true, Some(true)),
            ("Changing to 500k\u{2026}".into(), Tone::Wait)
        );
        assert_eq!(
            machine_line(Choice::At(500_000), Some(&old), true, Some(false)),
            ("Offline. Changing to 500k when it is back.".into(), Tone::Wait)
        );
        assert_eq!(
            machine_line(Choice::Default, Some(&old), true, Some(true)).0,
            "Removing Switchboard's value\u{2026}"
        );
        let none = seen(false, false, None, None, None);
        assert_eq!(machine_line(Choice::At(150_000), Some(&none), true, Some(true)).0, "Setting 150k\u{2026}");
        // offline but already as picked: nothing is pending
        let ok = seen(true, true, Some(500_000), Some("500000"), None);
        assert_eq!(
            machine_line(Choice::At(500_000), Some(&ok), true, Some(false)),
            ("500k, set by Switchboard.".into(), Tone::Ok)
        );
    }

    #[test]
    fn an_older_helper_only_knows_300k() {
        assert_eq!(
            machine_line(Choice::At(500_000), None, false, Some(true)),
            ("300k, not 500k: its helper is older and only knows 300k. Update Switchboard there.".into(), Tone::Older)
        );
        assert_eq!(machine_line(Choice::At(300_000), None, false, Some(true)).1, Tone::Ok);
        assert_eq!(machine_line(Choice::Default, None, false, Some(true)).1, Tone::Ok);
        // a newer helper whose settings file could not be read
        assert_eq!(machine_line(Choice::At(300_000), None, true, Some(true)).1, Tone::Unread);
        assert_eq!((Tone::Ok.tag(), Tone::Wait.tag(), Tone::Theirs.tag()), (None, None, Some("Left alone")));
    }

    #[test]
    fn what_follows_the_setting_is_said_beside_it() {
        assert_eq!(
            Choice::At(300_000).follows(),
            [
                ("Context meters fill at", "300k, or the model's window when that is smaller".to_string()),
                ("Start fresh is suggested from", "270k".to_string())
            ]
        );
        assert_eq!(Choice::At(150_000).follows()[0].1, "150k");
        assert_eq!(Choice::At(500_000).follows()[1].1, "450k");
        assert_eq!(
            Choice::Default.follows().map(|f| f.1),
            ["The model's window".to_string(), "Nine tenths of the model's window".to_string()]
        );
    }

    #[test]
    fn what_is_written_is_said_in_full() {
        assert!(Choice::At(500_000).writes().starts_with("Writes CLAUDE_CODE_AUTO_COMPACT_WINDOW=500000 to"));
        assert!(Choice::Default.writes().starts_with("Removes the value Switchboard wrote"));
    }

    #[test]
    fn a_meter_reads_against_the_chats_own_limit() {
        assert_eq!(of_limit(98_000, 300_000), "98k of 300k");
        assert_eq!(of_limit(98_000, 1_000_000), "98k of 1M");
    }
}
