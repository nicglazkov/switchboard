//! What the helper program's control API says, as data: the server and the command line read these.
//!
//! The API itself lives in the helper (sb-node control.rs) and listens on this computer only. These are its
//! answers, kept here so that everything that reads them agrees on the names.

use crate::identity::DeviceId;
use crate::paired::{Device, Kind, Via};
use serde::{Deserialize, Serialize};

/// The control API's port on 127.0.0.1.
pub const PORT: u16 = 8137;

/// A paired device, as `GET /devices` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRow {
    pub id: DeviceId,
    #[serde(default)]
    pub name: String,
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    #[serde(default)]
    pub paired: u64,
    pub via: Via,
    /// When it last had a connection, in seconds since 1970. None: not since the helper started.
    pub last_seen: Option<u64>,
    /// "direct" or "relay" while it is connected, else None.
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtt_ms: Option<u64>,
    /// A computer's helper version, as the hub last heard it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ver: Option<String>,
    /// A computer: whether its helper took the signed list of devices, which is what lets it answer for its own
    /// chats while the home computer is away. False: its helper is too old. None: not asked yet, or not a
    /// computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers_away: Option<bool>,
}

impl DeviceRow {
    pub fn of(d: Device) -> DeviceRow {
        DeviceRow {
            id: d.id,
            name: d.name,
            kind: d.kind,
            machine: d.machine,
            paired: d.paired,
            via: d.via,
            last_seen: None,
            path: None,
            rtt_ms: None,
            ver: None,
            answers_away: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelayRow {
    pub url: Option<String>,
    /// Where the address came from: "environment", "settings", "home", "built in" or "none".
    pub source: String,
    pub connected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The relay says another computer is using this computer's key.
    #[serde(default)]
    pub same_id: bool,
}

/// The home computer, as another computer sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeRow {
    pub id: DeviceId,
    #[serde(default)]
    pub name: String,
    pub connected: bool,
    /// "direct" or "relay" while connected.
    pub path: Option<String>,
    pub rtt_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Home said this computer is not paired with it (any more).
    #[serde(default)]
    pub refused: bool,
}

/// A loopback port on this computer that leads to the home computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardRow {
    /// "api" or "hub".
    pub target: String,
    pub addr: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The home computer's signed list of devices, as this computer holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterRow {
    pub epoch: u64,
    pub seq: u64,
    /// When the home computer signed it, and when it stops being honoured, in seconds since 1970.
    pub at: u64,
    pub expires: u64,
}

/// A computer the hub lists that was asked to switch to keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateRow {
    pub machine: String,
    /// "paired" or "needs update".
    pub state: String,
}

/// `GET /status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub ok: bool,
    pub version: String,
    /// "home" or "member".
    pub role: String,
    pub id: DeviceId,
    #[serde(default)]
    pub name: String,
    /// This computer's name at the hub, when it has one.
    pub machine: Option<String>,
    pub relay: RelayRow,
    /// On a computer that is not home.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<HomeRow>,
    #[serde(default)]
    pub forwarders: Vec<ForwardRow>,
    /// How many devices are paired (home).
    #[serde(default)]
    pub devices: usize,
    #[serde(default)]
    pub updates: Vec<UpdateRow>,
    /// Where other computers can reach this one without a relay ("192.0.2.20:41641"), on a home computer: a
    /// link that names no relay needs one of these typed on the computer that joins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addrs: Vec<String>,
    /// The home computer's signed list of devices. None: none is held yet.
    #[serde(default)]
    pub roster: Option<RosterRow>,
    /// On a computer that is not home, while the home computer is out of reach: what its two ports answer
    /// from (the `away` object of sb-link standin.rs). Null while home answers.
    #[serde(default)]
    pub away: Option<serde_json::Value>,
    /// What is wrong, in plain words. Empty when nothing is.
    #[serde(default)]
    pub problems: Vec<String>,
}

/// `POST /pair/new`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairNew {
    pub ok: bool,
    /// None when home has no relay: a short code can only find home through one.
    pub code: Option<String>,
    pub link: String,
    /// When both stop working, in seconds since 1970.
    pub expires: u64,
}

/// Which relay to use, and where that came from: the environment first, then the settings file, then (on a
/// computer that is not home) the one pairing recorded for home, then the one this build was made with.
pub fn resolve_relay(
    env: Option<&str>,
    settings: Option<&str>,
    home: Option<&str>,
    built_in: &str,
) -> (Option<String>, &'static str) {
    let pick = |v: Option<&str>| v.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    if let Some(v) = pick(env) {
        return (Some(v), "environment");
    }
    if let Some(v) = pick(settings) {
        return (Some(v), "settings");
    }
    if let Some(v) = pick(home) {
        return (Some(v), "home");
    }
    match pick(Some(built_in)) {
        Some(v) => (Some(v), "built in"),
        None => (None, "none"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_relay_is_the_first_one_that_is_set() {
        let r = resolve_relay;
        assert_eq!(
            r(Some("https://e"), Some("https://s"), Some("https://h"), "https://b"),
            (Some("https://e".into()), "environment")
        );
        assert_eq!(
            r(Some("  "), Some("https://s"), Some("https://h"), "https://b"),
            (Some("https://s".into()), "settings")
        );
        assert_eq!(r(None, None, Some("https://h"), "https://b"), (Some("https://h".into()), "home"));
        assert_eq!(r(None, Some(""), None, "https://b"), (Some("https://b".into()), "built in"));
        assert_eq!(r(None, None, None, ""), (None, "none"));
    }

    #[test]
    fn a_device_row_reads_as_the_server_expects() {
        let id: DeviceId = "07ef0adb411452d0243de7f4b9c7c14cfaf89b4538f63aed4420538d1f1ee9f9".parse().unwrap();
        let row = DeviceRow {
            last_seen: Some(7),
            path: Some("direct".into()),
            ..DeviceRow::of(Device {
                id,
                name: "Laptop".into(),
                kind: Kind::Computer,
                machine: Some("laptop".into()),
                paired: 5,
                via: Via::Code,
            })
        };
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"id": id.to_string(), "name": "Laptop", "kind": "computer", "machine": "laptop",
                               "paired": 5, "via": "code", "last_seen": 7, "path": "direct"})
        );
        let phone =
            DeviceRow::of(Device { id, name: "p".into(), kind: Kind::Phone, machine: None, paired: 1, via: Via::Link });
        let v = serde_json::to_value(&phone).unwrap();
        assert_eq!(v["last_seen"], serde_json::Value::Null);
        assert_eq!(v["path"], serde_json::Value::Null);
        assert!(v.get("machine").is_none());
    }
}
