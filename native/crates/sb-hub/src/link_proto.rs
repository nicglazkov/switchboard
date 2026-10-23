//! The node link (spec section 6): one WebSocket from a machine's switchboard-node to the hub's `/node`.
//! Text frames carry the JSON messages below; binary frames carry terminal bytes behind a 4-byte channel id (big
//! endian). The hub and sb-node both use this module, so the two sides never drift.

use serde::{Deserialize, Serialize};

/// Hub to node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToNode {
    /// Run `sb.py rpc` with `req` on stdin and answer with `rpc.result`. `timeout` is in seconds.
    #[serde(rename = "rpc")]
    Rpc { id: u64, req: serde_json::Value, timeout: u64 },
    /// A PTY running `claude attach <bg>` (through sb.py, exactly as over ssh), sized cols x rows.
    #[serde(rename = "term.open")]
    TermOpen { ch: u32, bg: String, cols: u16, rows: u16 },
    #[serde(rename = "term.resize")]
    TermResize { ch: u32, cols: u16, rows: u16 },
    /// The viewer left: hang up. Claude Code detaches and the session keeps running.
    #[serde(rename = "term.close")]
    TermClose { ch: u32 },
}

/// Node to hub.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FromNode {
    /// The first message on every link.
    #[serde(rename = "hello")]
    Hello {
        version: String,
        os: String,
        #[serde(default)]
        label: String,
    },
    #[serde(rename = "rpc.result")]
    RpcResult { id: u64, res: serde_json::Value },
    /// The terminal's process ended; everything it wrote was sent before this.
    #[serde(rename = "term.exit")]
    TermExit { ch: u32, code: Option<i64> },
}

/// The hub closes a link with this code when the same machine links again (two nodes with one token).
pub const CLOSE_REPLACED: u16 = 4409;
/// ... and with this one when its token stopped working (unpaired, or paired again): the node waits for a new pairing.
pub const CLOSE_UNPAIRED: u16 = 4401;

pub fn encode<T: Serialize>(m: &T) -> String {
    serde_json::to_string(m).unwrap_or_default()
}

/// Terminal bytes for channel `ch`.
pub fn frame(ch: u32, data: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + data.len());
    v.extend_from_slice(&ch.to_be_bytes());
    v.extend_from_slice(data);
    v
}

pub fn unframe(b: &[u8]) -> Option<(u32, &[u8])> {
    let (head, rest) = b.split_at_checked(4)?;
    Some((u32::from_be_bytes(head.try_into().ok()?), rest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_match_the_spec() {
        let m = ToNode::TermOpen { ch: 3, bg: "c03ef0d9".into(), cols: 80, rows: 24 };
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json!({"type": "term.open", "ch": 3, "bg": "c03ef0d9", "cols": 80, "rows": 24})
        );
        let r: FromNode = serde_json::from_str(r#"{"type":"rpc.result","id":7,"res":{"ok":true}}"#).unwrap();
        assert_eq!(r, FromNode::RpcResult { id: 7, res: json!({"ok": true}) });
        let h: FromNode = serde_json::from_str(r#"{"type":"hello","version":"0.1.0","os":"linux"}"#).unwrap();
        assert_eq!(h, FromNode::Hello { version: "0.1.0".into(), os: "linux".into(), label: String::new() });
        assert!(serde_json::from_str::<ToNode>(r#"{"type":"later.thing"}"#).is_err());
    }

    #[test]
    fn frames() {
        let f = frame(258, b"hi");
        assert_eq!(f, vec![0, 0, 1, 2, b'h', b'i']);
        assert_eq!(unframe(&f), Some((258, &b"hi"[..])));
        assert_eq!(unframe(&[0, 0, 1]), None);
        assert_eq!(unframe(&[0, 0, 0, 9]), Some((9, &b""[..])));
    }
}
