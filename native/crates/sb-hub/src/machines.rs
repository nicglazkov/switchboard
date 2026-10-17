//! The machine list the Python server writes at start (`~/.local/share/switchboard/machines.json`):
//! {"machines": [{"name", "label", "target", "os"}], "origins": ["http://..."]}. A bare list of machines works too.

use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Machine {
    pub name: String,
    #[serde(default)]
    pub label: String,
    pub target: String,
    #[serde(default)]
    pub os: String,
}

impl Machine {
    pub fn label(&self) -> &str {
        if self.label.is_empty() { &self.name } else { &self.label }
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct MachineFile {
    #[serde(default)]
    pub machines: Vec<Machine>,
    #[serde(default)]
    pub origins: Vec<String>,
}

pub fn parse(text: &str) -> Result<MachineFile, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut f: MachineFile = if v.is_array() {
        MachineFile { machines: serde_json::from_value(v).map_err(|e| e.to_string())?, origins: vec![] }
    } else {
        serde_json::from_value(v).map_err(|e| e.to_string())?
    };
    f.machines.retain(|m| crate::ticket::valid_machine(&m.name) && valid_target(&m.target));
    Ok(f)
}

pub fn load(path: &Path) -> Result<MachineFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse(&text)
}

/// An ssh destination: `user@host` or `host`, nothing that ssh could read as an option.
pub fn valid_target(t: &str) -> bool {
    let ok = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"._-:[]%".contains(&c));
    if t.is_empty() || t.len() > 255 || t.starts_with('-') {
        return false;
    }
    match t.split_once('@') {
        Some((u, h)) => ok(u) && ok(h) && !h.starts_with('-') && !u.starts_with('-'),
        None => ok(t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_server_file() {
        let f = parse(r#"{"machines":[{"name":"studio","label":"Studio","target":"me@10.0.0.11","os":"mac"},
            {"name":"tower","label":"Tower","target":"me@10.0.0.12","os":"windows"}],
            "origins":["http://10.0.0.5:8130"]}"#).unwrap();
        assert_eq!(f.machines.len(), 2);
        assert_eq!(f.machines[1].label(), "Tower");
        assert_eq!(f.origins, vec!["http://10.0.0.5:8130"]);
    }

    #[test]
    fn bare_list_and_bad_rows() {
        let f = parse(r#"[{"name":"studio","target":"me@10.0.0.11"},{"name":"x","target":"-oProxyCommand=sh"},
            {"name":"Bad","target":"a@b"},{"name":"lab","target":"lab"}]"#).unwrap();
        let names: Vec<_> = f.machines.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["studio", "lab"]);
        assert_eq!(f.machines[0].label(), "studio");
    }

    #[test]
    fn targets() {
        for ok in ["me@10.0.0.11", "me@10.0.0.12", "hub-box", "me@host.tail1234.ts.net", "u@[fe80::1%en0]"] {
            assert!(valid_target(ok), "{ok}");
        }
        for bad in ["", "-oX", "a@-oX", "a b", "a@b;c", "a@b c", "a@$(x)"] {
            assert!(!valid_target(bad), "{bad}");
        }
    }
}
