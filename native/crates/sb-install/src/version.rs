//! App versions as the bundle states them ("0.8.4", "1.0.0-beta.2") and what installing one over another means.

use std::cmp::Ordering;
use std::fmt;

/// A dotted version with an optional pre-release tag. Missing parts count as 0, so "1.2" equals "1.2.0".
#[derive(Clone, Debug)]
pub struct Version {
    parts: Vec<u64>,
    pre: Option<String>,
    text: String,
}

impl Version {
    /// Parses "0.8.4", "v0.8.4" or "1.0.0-beta.2". Build metadata after '+' is ignored. None for anything else.
    pub fn parse(s: &str) -> Option<Version> {
        let text = s.trim();
        let core = text.strip_prefix('v').unwrap_or(text);
        let core = core.split('+').next().unwrap_or(core);
        let (nums, pre) = match core.split_once('-') {
            Some((n, p)) if !p.is_empty() => (n, Some(p.to_string())),
            Some(_) => return None,
            None => (core, None),
        };
        let parts: Vec<u64> = nums.split('.').map(|p| p.parse().ok()).collect::<Option<_>>()?;
        if parts.is_empty() || parts.len() > 4 {
            return None;
        }
        Some(Version { parts, pre, text: text.to_string() })
    }

    fn part(&self, i: usize) -> u64 {
        self.parts.get(i).copied().unwrap_or(0)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        let n = self.parts.len().max(other.parts.len());
        for i in 0..n {
            match self.part(i).cmp(&other.part(i)) {
                Ordering::Equal => {}
                o => return o,
            }
        }
        // A pre-release comes before its release: 1.0.0-beta < 1.0.0.
        match (&self.pre, &other.pre) {
            (None, None) => Ordering::Equal,
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (Some(a), Some(b)) => cmp_pre(a, b),
        }
    }
}

impl PartialEq for Version {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Version {}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Pre-release tags compare part by part: numbers as numbers, below words ("beta.2" < "beta.10" < "rc").
fn cmp_pre(a: &str, b: &str) -> Ordering {
    let (mut x, mut y) = (a.split('.'), b.split('.'));
    loop {
        match (x.next(), y.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(p), Some(q)) => {
                let o = match (p.parse::<u64>(), q.parse::<u64>()) {
                    (Ok(m), Ok(n)) => m.cmp(&n),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => p.cmp(q),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
        }
    }
}

/// What installing `new` means, given the version already installed (None: nothing is).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Fresh,
    Update {
        from: Version,
    },
    /// The same version again: puts back a damaged or changed copy.
    Reinstall,
    /// Refused: the installed copy is newer.
    Downgrade {
        installed: Version,
    },
    /// The installed copy has no version we can read. Treated as an update, since it is ours by name.
    Unknown,
}

pub fn kind(installed: Option<Option<&Version>>, new: &Version) -> Kind {
    match installed {
        None => Kind::Fresh,
        Some(None) => Kind::Unknown,
        Some(Some(old)) => match old.cmp(new) {
            Ordering::Less => Kind::Update { from: old.clone() },
            Ordering::Equal => Kind::Reinstall,
            Ordering::Greater => Kind::Downgrade { installed: old.clone() },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn parses_the_shapes_bundles_use() {
        assert_eq!(v("0.8.4").to_string(), "0.8.4");
        assert_eq!(v("v1.2"), v("v1.2"));
        assert!(Version::parse("").is_none());
        assert!(Version::parse("1.x").is_none());
        assert!(Version::parse("1.0-").is_none());
        assert!(Version::parse("1.2.3.4.5").is_none());
        assert_eq!(v("1.0.0+build7"), v("1.0.0"));
    }

    #[test]
    fn compares_numbers_not_text() {
        assert!(v("0.8.4") < v("0.9.0"));
        assert!(v("0.9.0") < v("0.10.0"));
        assert!(v("0.8.10") > v("0.8.9"));
        assert_eq!(v("1.2"), v("1.2.0"));
        assert!(v("2") > v("1.99.99"));
    }

    #[test]
    fn pre_releases_come_first() {
        assert!(v("1.0.0-beta") < v("1.0.0"));
        assert!(v("1.0.0-beta.2") < v("1.0.0-beta.10"));
        assert!(v("1.0.0-beta.10") < v("1.0.0-rc.1"));
        assert!(v("1.0.0-rc.1") > v("0.9.9"));
        assert!(v("1.0.0-1") < v("1.0.0-alpha"));
    }

    #[test]
    fn kinds() {
        let new = v("0.9.0");
        assert_eq!(kind(None, &new), Kind::Fresh);
        assert_eq!(kind(Some(None), &new), Kind::Unknown);
        assert_eq!(kind(Some(Some(&v("0.8.4"))), &new), Kind::Update { from: v("0.8.4") });
        assert_eq!(kind(Some(Some(&v("0.9.0"))), &new), Kind::Reinstall);
        assert_eq!(kind(Some(Some(&v("0.9.1"))), &new), Kind::Downgrade { installed: v("0.9.1") });
        assert_eq!(kind(Some(Some(&v("0.10.0"))), &new), Kind::Downgrade { installed: v("0.10.0") });
    }
}
