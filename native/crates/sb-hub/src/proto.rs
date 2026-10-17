//! The terminal WebSocket protocol (spec section 1).
//!
//! client -> hub: binary = raw input bytes; text = {"type":"resize","cols":N,"rows":M} (the size this viewer would
//! like), {"type":"focus"} (it has focus: it sets the size), {"type":"redraw"} (send the screen again) or
//! {"type":"ping"}.
//! hub -> client: binary = raw terminal output; text = status, size (the terminal's size from this point in the
//! stream), owner (whether this viewer sets it), snapshot_begin and snapshot_end (the screen as output, for a fresh
//! grid), exit, error, pong. Clients that ignore text they do not know keep working: they get a reset in front of
//! a later snapshot instead (spec 2026-10-14, terminal pipeline).
//! Close codes: 1000 after exit, 4401 bad or used ticket, 4403 bad origin, 4429 at the terminal limit.

use serde::Deserialize;
use serde_json::json;

pub const CLOSE_NORMAL: u16 = 1000;
pub const CLOSE_INTERNAL: u16 = 1011;
pub const CLOSE_TICKET: u16 = 4401;
pub const CLOSE_ORIGIN: u16 = 4403;
pub const CLOSE_LIMIT: u16 = 4429;

pub const DEFAULT_COLS: u16 = 120;
pub const DEFAULT_ROWS: u16 = 36;

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Control {
    Resize { cols: u32, rows: u32 },
    Focus,
    Redraw,
    Ping,
}

/// A control frame, or None for anything not understood (ignored, so newer clients keep working).
pub fn parse_control(text: &str) -> Option<Control> {
    serde_json::from_str(text).ok()
}

/// Sizes a PTY can take: at least 2x1, at most 500x300.
pub fn clamp_size(cols: u32, rows: u32) -> (u16, u16) {
    (cols.clamp(2, 500) as u16, rows.clamp(1, 300) as u16)
}

pub fn status(state: &str) -> String {
    json!({"type": "status", "state": state}).to_string()
}

pub fn exit(code: Option<i64>, note: &str) -> String {
    json!({"type": "exit", "code": code, "note": note}).to_string()
}

pub fn error(msg: &str) -> String {
    json!({"type": "error", "error": msg}).to_string()
}

pub fn pong() -> String {
    json!({"type": "pong"}).to_string()
}

pub fn size(cols: u16, rows: u16) -> String {
    json!({"type": "size", "cols": cols, "rows": rows}).to_string()
}

pub fn owner(owner: bool) -> String {
    json!({"type": "owner", "owner": owner}).to_string()
}

pub fn snapshot_begin() -> String {
    json!({"type": "snapshot_begin"}).to_string()
}

pub fn snapshot_end() -> String {
    json!({"type": "snapshot_end"}).to_string()
}

/// What a finished `ssh ... term <id>` means for the person. The session itself keeps running in every case.
pub fn exit_note(code: Option<i64>, label: &str) -> String {
    match code {
        Some(0) => "Detached. The session keeps running.".into(),
        Some(255) => format!("Could not reach {label}, or the connection dropped. The session keeps running there."),
        Some(c) => format!("The terminal on {label} ended (code {c}). The session keeps running."),
        None => "The terminal ended. The session keeps running.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls() {
        assert_eq!(parse_control(r#"{"type":"resize","cols":80,"rows":24}"#), Some(Control::Resize { cols: 80, rows: 24 }));
        assert_eq!(parse_control(r#"{"type":"ping"}"#), Some(Control::Ping));
        assert_eq!(parse_control(r#"{"type":"focus"}"#), Some(Control::Focus));
        assert_eq!(parse_control(r#"{"type":"redraw"}"#), Some(Control::Redraw));
        assert_eq!(parse_control(r#"{"type":"ping","extra":1}"#), Some(Control::Ping));
        for bad in [r#"{"type":"resize","cols":-1,"rows":24}"#, r#"{"type":"resize"}"#, r#"{"type":"nope"}"#, "", "[]", "hi"] {
            assert_eq!(parse_control(bad), None, "{bad}");
        }
    }

    #[test]
    fn sizes_are_clamped() {
        assert_eq!(clamp_size(80, 24), (80, 24));
        assert_eq!(clamp_size(0, 0), (2, 1));
        assert_eq!(clamp_size(100_000, 100_000), (500, 300));
    }

    #[test]
    fn outgoing_frames_match_the_spec() {
        let v: serde_json::Value = serde_json::from_str(&status("connecting")).unwrap();
        assert_eq!(v, json!({"type": "status", "state": "connecting"}));
        let v: serde_json::Value = serde_json::from_str(&exit(Some(0), "Detached. The session keeps running.")).unwrap();
        assert_eq!(v, json!({"type": "exit", "code": 0, "note": "Detached. The session keeps running."}));
        let v: serde_json::Value = serde_json::from_str(&error("x")).unwrap();
        assert_eq!(v, json!({"type": "error", "error": "x"}));
        let v: serde_json::Value = serde_json::from_str(&pong()).unwrap();
        assert_eq!(v, json!({"type": "pong"}));
        let v: serde_json::Value = serde_json::from_str(&size(100, 30)).unwrap();
        assert_eq!(v, json!({"type": "size", "cols": 100, "rows": 30}));
        let v: serde_json::Value = serde_json::from_str(&owner(true)).unwrap();
        assert_eq!(v, json!({"type": "owner", "owner": true}));
        assert_eq!(snapshot_begin(), r#"{"type":"snapshot_begin"}"#);
        assert_eq!(snapshot_end(), r#"{"type":"snapshot_end"}"#);
    }

    #[test]
    fn exit_notes() {
        assert_eq!(exit_note(Some(0), "Desk"), "Detached. The session keeps running.");
        assert!(exit_note(Some(255), "Desk").starts_with("Could not reach Desk"));
        assert!(exit_note(Some(1), "Tower").contains("code 1"));
    }
}
