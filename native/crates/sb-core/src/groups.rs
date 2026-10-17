//! Tab groups, Chrome style, shared by every Mac through the server (GET/POST /api/groups, and `groups` on the
//! sessions snapshot). A group holds threads (a chat's `thread`, else its sid), so it follows Start fresh, Continue
//! and Resume. A thread is in at most one group; a group emptied by a change is gone. Which tabs are open, and
//! whether a group is folded, stays on each Mac.
//!
//! `Op::apply` is the server's rule set, run here first so the tab row changes at once (the server's answer then
//! replaces it).

use serde::{Deserialize, Serialize};

use crate::api::ApiGroup;

/// The eight colors a group can have, in the order "Add to new group" hands them out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupColor {
    Grey,
    Blue,
    Red,
    Yellow,
    Green,
    Pink,
    Purple,
    Cyan,
}

impl GroupColor {
    pub const ALL: [GroupColor; 8] = [
        GroupColor::Blue,
        GroupColor::Red,
        GroupColor::Yellow,
        GroupColor::Green,
        GroupColor::Pink,
        GroupColor::Purple,
        GroupColor::Cyan,
        GroupColor::Grey,
    ];

    pub fn name(self) -> &'static str {
        match self {
            GroupColor::Grey => "grey",
            GroupColor::Blue => "blue",
            GroupColor::Red => "red",
            GroupColor::Yellow => "yellow",
            GroupColor::Green => "green",
            GroupColor::Pink => "pink",
            GroupColor::Purple => "purple",
            GroupColor::Cyan => "cyan",
        }
    }

    /// What the menus call it.
    pub fn title(self) -> &'static str {
        match self {
            GroupColor::Grey => "Grey",
            GroupColor::Blue => "Blue",
            GroupColor::Red => "Red",
            GroupColor::Yellow => "Yellow",
            GroupColor::Green => "Green",
            GroupColor::Pink => "Pink",
            GroupColor::Purple => "Purple",
            GroupColor::Cyan => "Cyan",
        }
    }

    /// An unknown color (a newer server) reads as grey.
    pub fn parse(s: &str) -> GroupColor {
        GroupColor::ALL.into_iter().find(|c| c.name() == s).unwrap_or(GroupColor::Grey)
    }

    /// The first color no group has yet (Chrome's order), else the one fewest groups have.
    pub fn next_unused(groups: &[Group]) -> GroupColor {
        let used = |c: GroupColor| groups.iter().filter(|g| g.color == c).count();
        GroupColor::ALL.into_iter().min_by_key(|c| used(*c)).unwrap_or(GroupColor::Blue)
    }
}

/// A group as the apps use it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    /// Up to 40 characters; may be empty (the chip is then just its color).
    pub name: String,
    pub color: GroupColor,
    /// Thread sids, in the group's order.
    pub members: Vec<String>,
}

pub const NAME_MAX: usize = 40;

impl Group {
    pub fn from_api(g: &ApiGroup) -> Option<Group> {
        if g.id.is_empty() {
            return None;
        }
        Some(Group { id: g.id.clone(), name: g.name.clone(), color: GroupColor::parse(&g.color), members: g.members.clone() })
    }
}

/// The groups from a server reply (bad rows dropped).
pub fn from_api(gs: &[ApiGroup]) -> Vec<Group> {
    gs.iter().filter_map(Group::from_api).collect()
}

/// The group a thread is in.
pub fn group_of<'a>(groups: &'a [Group], thread: &str) -> Option<&'a Group> {
    groups.iter().find(|g| g.members.iter().any(|m| m == thread))
}

/// A name as the server keeps it: runs of spaces as one, trimmed, at most 40 characters, no control characters.
pub fn clean_name(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(NAME_MAX).collect()
}

/// One change, as POST /api/groups takes it.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    /// A new group; `id` is made here so the tab row can show it before the server answers (the server makes its
    /// own, and its answer replaces this one).
    Create { name: String, color: GroupColor, members: Vec<String> },
    Update { id: String, name: Option<String>, color: Option<GroupColor> },
    /// Ungroups its chats (none close).
    Delete { id: String },
    /// Into group `id` at `index` (of its members once `thread` is out of it); out of any other group.
    Add { id: String, thread: String, index: Option<usize> },
    Remove { thread: String },
    /// Group `id` to place `index` among the groups.
    Move { id: String, index: usize },
}

impl Op {
    pub fn body(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            Op::Create { name, color, members } => json!({"op": "create", "name": name, "color": color.name(), "members": members}),
            Op::Update { id, name, color } => {
                let mut b = json!({"op": "update", "id": id});
                if let Some(n) = name {
                    b["name"] = json!(n);
                }
                if let Some(c) = color {
                    b["color"] = json!(c.name());
                }
                b
            }
            Op::Delete { id } => json!({"op": "delete", "id": id}),
            Op::Add { id, thread, index } => match index {
                Some(i) => json!({"op": "add", "id": id, "thread": thread, "index": i}),
                None => json!({"op": "add", "id": id, "thread": thread}),
            },
            Op::Remove { thread } => json!({"op": "remove", "thread": thread}),
            Op::Move { id, index } => json!({"op": "move", "id": id, "index": index}),
        }
    }

    /// The server's rules, run on a local copy. `new_id` names a created group until the server's answer comes.
    pub fn apply(&self, groups: &mut Vec<Group>, new_id: &str) {
        let had: Vec<String> = groups.iter().filter(|g| !g.members.is_empty()).map(|g| g.id.clone()).collect();
        let take = |gs: &mut Vec<Group>, t: &str| {
            for g in gs.iter_mut() {
                g.members.retain(|m| m != t);
            }
        };
        match self {
            Op::Create { name, color, members } => {
                let mut ms: Vec<String> = Vec::new();
                for m in members {
                    if !ms.contains(m) {
                        ms.push(m.clone());
                    }
                }
                for m in &ms {
                    take(groups, m);
                }
                groups.push(Group { id: new_id.to_string(), name: clean_name(name), color: *color, members: ms });
            }
            Op::Update { id, name, color } => {
                if let Some(g) = groups.iter_mut().find(|g| g.id == *id) {
                    if let Some(n) = name {
                        g.name = clean_name(n);
                    }
                    if let Some(c) = color {
                        g.color = *c;
                    }
                }
            }
            Op::Delete { id } => groups.retain(|g| g.id != *id),
            Op::Add { id, thread, index } => {
                if groups.iter().any(|g| g.id == *id) {
                    take(groups, thread);
                    if let Some(g) = groups.iter_mut().find(|g| g.id == *id) {
                        let at = index.unwrap_or(g.members.len()).min(g.members.len());
                        g.members.insert(at, thread.clone());
                    }
                }
            }
            Op::Remove { thread } => take(groups, thread),
            Op::Move { id, index } => {
                if let Some(at) = groups.iter().position(|g| g.id == *id) {
                    let g = groups.remove(at);
                    let to = (*index).min(groups.len());
                    groups.insert(to, g);
                }
            }
        }
        groups.retain(|g| !g.members.is_empty() || !had.contains(&g.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(id: &str, members: &[&str]) -> Group {
        Group { id: id.into(), name: id.to_uppercase(), color: GroupColor::Blue, members: members.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn colors_read_and_hand_out_like_chrome() {
        assert_eq!(GroupColor::parse("pink"), GroupColor::Pink);
        assert_eq!(GroupColor::parse("mauve"), GroupColor::Grey);
        assert_eq!(GroupColor::next_unused(&[]), GroupColor::Blue);
        let mut gs = vec![g("a", &["1"])];
        assert_eq!(GroupColor::next_unused(&gs), GroupColor::Red);
        gs[0].color = GroupColor::Red;
        assert_eq!(GroupColor::next_unused(&gs), GroupColor::Blue);
        let all: Vec<Group> = GroupColor::ALL.iter().enumerate().map(|(i, c)| Group { color: *c, ..g(&i.to_string(), &["x"]) }).collect();
        assert_eq!(GroupColor::next_unused(&all), GroupColor::Blue, "all taken: the first again");
    }

    #[test]
    fn ops_follow_the_server_rules() {
        let mut gs = vec![g("a", &["1", "2"]), g("b", &["3"])];
        Op::Add { id: "b".into(), thread: "1".into(), index: Some(0) }.apply(&mut gs, "");
        assert_eq!(gs, vec![g("a", &["2"]), g("b", &["1", "3"])]);
        Op::Add { id: "b".into(), thread: "1".into(), index: Some(9) }.apply(&mut gs, "");
        assert_eq!(gs[1].members, vec!["3", "1"]);
        Op::Remove { thread: "2".into() }.apply(&mut gs, "");
        assert_eq!(gs.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(), vec!["b"], "emptied: gone");
        Op::Create { name: "  New   one ".into(), color: GroupColor::Green, members: vec!["3".into(), "3".into()] }.apply(&mut gs, "tmp");
        assert_eq!((gs[1].id.as_str(), gs[1].name.as_str(), gs[1].members.clone()), ("tmp", "New one", vec!["3".to_string()]));
        assert_eq!(gs[0].members, vec!["1"]);
        Op::Move { id: "tmp".into(), index: 0 }.apply(&mut gs, "");
        assert_eq!(gs[0].id, "tmp");
        Op::Update { id: "b".into(), name: Some("x".repeat(50)), color: Some(GroupColor::Cyan) }.apply(&mut gs, "");
        assert_eq!((gs[1].name.len(), gs[1].color), (40, GroupColor::Cyan));
        Op::Delete { id: "tmp".into() }.apply(&mut gs, "");
        assert_eq!(gs.len(), 1);
        assert_eq!(group_of(&gs, "1").map(|g| g.id.as_str()), Some("b"));
        assert!(group_of(&gs, "3").is_none());
    }

    #[test]
    fn bodies_are_what_the_server_reads() {
        assert_eq!(Op::Add { id: "a".into(), thread: "t".into(), index: None }.body(), serde_json::json!({"op": "add", "id": "a", "thread": "t"}));
        assert_eq!(Op::Update { id: "a".into(), name: None, color: Some(GroupColor::Red) }.body(), serde_json::json!({"op": "update", "id": "a", "color": "red"}));
        assert_eq!(
            Op::Create { name: "".into(), color: GroupColor::Grey, members: vec!["t".into()] }.body(),
            serde_json::json!({"op": "create", "name": "", "color": "grey", "members": ["t"]})
        );
    }
}
