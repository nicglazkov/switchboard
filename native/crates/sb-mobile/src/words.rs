//! What the phone says about its connection to the home computer, and about a pairing that did not work. The
//! sentences are the desktop app's (sb-core keys.rs) wherever the same thing is being said, so a person reads the
//! same words on every device.

use crate::{LinkError, LinkPath, LinkState};
use sb_core::keys::{InputProblem, JoinProblem, read_input};
use sb_link::pairing::PairError;

pub const DIRECT: &str = "Direct";
pub const RELAY: &str = "Through the relay";
pub const CONNECTING: &str = "Connecting to home";
pub const REMOVED: &str = "Can't reach home: it no longer lists this phone.";
pub const REMOVED_FIX: &str = "It was removed there. On your home computer, open Settings, then Devices, choose Add a \
                               device, and join again here with the new code.";
/// Said while away when the stand-in has no line of its own.
pub const AWAY: &str = "Home is away.";
pub const NO_ANSWER_FIX: &str = "Check that your home computer is on and online. This phone keeps trying.";
pub const NEEDS_LINK: &str = "A short code finds your home computer through a relay, and this phone has none set. \
                              Scan the QR code or paste the link instead.";
pub const NEEDS_ADDRESS: &str = "That link has no relay in it, so it can't find your home computer from here. Set a \
                                 relay on the home computer and get a new link, or connect by address.";

/// The one line the app shows where it shows its connection, and what to do about it when there is something
/// to do.
pub fn line(state: LinkState, path: Option<LinkPath>, why: Option<&str>, relay_down: bool) -> (String, Option<String>) {
    match state {
        LinkState::Stopped => (String::new(), None),
        LinkState::Connected => match path {
            Some(LinkPath::Direct) => (DIRECT.into(), None),
            Some(LinkPath::Relay) => (RELAY.into(), None),
            None => (CONNECTING.into(), None),
        },
        LinkState::Removed => (REMOVED.into(), Some(REMOVED_FIX.into())),
        LinkState::Connecting => match why {
            None => (CONNECTING.into(), None),
            Some(_) if relay_down => {
                ("Can't reach home: the relay isn't answering.".into(), Some(NO_ANSWER_FIX.into()))
            }
            Some(w) if w.contains("no relay is set") => (
                "Can't reach home: no relay is set and it isn't on this network.".into(),
                Some("Join again with a link from your home computer once it has a relay set.".into()),
            ),
            Some(_) => ("Can't reach home: it isn't answering.".into(), Some(NO_ANSWER_FIX.into())),
        },
    }
}

/// The line while the home computer is away: the stand-in's own (sb-link standin.rs), which names the computers
/// whose chats are showing. None when home is not away.
pub fn away_line(away: Option<&serde_json::Value>) -> Option<String> {
    let a = away?;
    Some(a["line"].as_str().filter(|l| !l.is_empty()).unwrap_or(AWAY).to_string())
}

/// What was typed or scanned, as it is sent on: a code as `ABCD-EFGH`, or the link. Err says what is wrong with
/// it, before anything is sent.
pub fn input(typed: &str) -> Result<String, LinkError> {
    read_input(typed).map_err(|p: InputProblem| LinkError::Input { text: p.says() })
}

pub fn unreachable() -> LinkError {
    LinkError::Unreachable { text: JoinProblem::HomeUnreachable.says() }
}

/// Why the exchange itself did not work.
pub fn pair_error(e: &PairError) -> LinkError {
    match e {
        PairError::WrongCode => LinkError::Code { text: JoinProblem::WrongCode.says() },
        PairError::Gone => LinkError::Code { text: JoinProblem::Expired.says() },
        PairError::Busy => LinkError::Other { text: JoinProblem::Busy.says() },
        PairError::Timeout | PairError::Io(_) => unreachable(),
        other => LinkError::Other { text: other.message().replace("this device", "this phone") },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_connection_line() {
        let l = |s, p, w, r| line(s, p, w, r).0;
        assert_eq!(l(LinkState::Connected, Some(LinkPath::Direct), None, false), "Direct");
        assert_eq!(l(LinkState::Connected, Some(LinkPath::Relay), None, false), "Through the relay");
        assert_eq!(l(LinkState::Connected, None, None, false), "Connecting to home");
        assert_eq!(l(LinkState::Connecting, None, None, false), "Connecting to home");
        assert_eq!(l(LinkState::Stopped, None, None, false), "");
        assert_eq!(
            l(LinkState::Connecting, None, Some("the other computer did not answer"), false),
            "Can't reach home: it isn't answering."
        );
        assert_eq!(l(LinkState::Connecting, None, Some("x"), true), "Can't reach home: the relay isn't answering.");
        let (text, fix) = line(LinkState::Removed, None, Some("closed by peer: removed"), false);
        assert_eq!(text, "Can't reach home: it no longer lists this phone.");
        assert!(fix.unwrap().contains("Add a device"));
        assert!(line(LinkState::Connected, Some(LinkPath::Direct), None, false).1.is_none());
    }

    #[test]
    fn the_line_while_home_is_away_is_the_stand_ins() {
        use serde_json::json;
        assert_eq!(away_line(None), None);
        let a = json!({"since": 5, "line": "Home is away. Showing chats from Laptop and Tower."});
        assert_eq!(away_line(Some(&a)).as_deref(), Some("Home is away. Showing chats from Laptop and Tower."));
        assert_eq!(away_line(Some(&json!({"since": 5}))).as_deref(), Some("Home is away."));
        assert_eq!(away_line(Some(&json!({"line": ""}))).as_deref(), Some(AWAY));
    }

    #[test]
    fn no_line_has_a_dash_or_an_exclamation_mark() {
        let all = [DIRECT, RELAY, CONNECTING, REMOVED, REMOVED_FIX, NO_ANSWER_FIX, NEEDS_LINK, NEEDS_ADDRESS, AWAY];
        for s in all {
            assert!(!s.contains('\u{2014}') && !s.contains('!'), "{s}");
        }
    }

    #[test]
    fn what_was_typed_is_read_the_way_the_desktop_app_reads_it() {
        assert_eq!(input(" abcd efgh ").unwrap(), "ABCD-EFGH");
        assert_eq!(input("abcd-efgh").unwrap(), "ABCD-EFGH");
        let link = "switchboard://pair/1?h=07ef0adb411452d0243de7f4b9c7c14cfaf89b4538f63aed4420538d1f1ee9f9&s=000102030405060708090a0b0c0d0e0f";
        assert_eq!(input(link).unwrap(), link);
        let says = |t: &str| match input(t) {
            Err(LinkError::Input { text: message }) => message,
            other => panic!("{other:?}"),
        };
        assert_eq!(says(""), "Type the code from your home computer, or paste its link.");
        assert_eq!(says("ABCD"), "A code has 8 letters and digits. That was 4.");
        assert!(says("ABCD-EFG0").starts_with("Codes never use 0, O, 1, I or L."));
        assert!(says("https://example.com").starts_with("That isn't a Switchboard pairing link."));
    }

    #[test]
    fn a_pairing_that_failed_says_why() {
        let msg = |e: PairError| match pair_error(&e) {
            LinkError::Code { text: message }
            | LinkError::Other { text: message }
            | LinkError::Unreachable { text: message } => message,
            LinkError::Input { text: message } => panic!("{message}"),
        };
        assert!(matches!(pair_error(&PairError::WrongCode), LinkError::Code { .. }));
        assert!(matches!(pair_error(&PairError::Gone), LinkError::Code { .. }));
        assert!(matches!(pair_error(&PairError::Timeout), LinkError::Unreachable { .. }));
        assert!(msg(PairError::WrongCode).starts_with("That code didn't match."));
        assert!(msg(PairError::Gone).starts_with("That code has expired or was already used."));
        assert_eq!(msg(PairError::Refused(String::new())), "The other computer could not add this phone.");
    }
}
