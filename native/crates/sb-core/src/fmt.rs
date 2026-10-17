//! Formatting, word for word what the web app and the design lab print. Times use the machine's local zone.

use chrono::{DateTime, Datelike, FixedOffset, Local, TimeZone, Timelike};

/// 180353 -> "180k", 1_250_000 -> "1.3M"
pub fn k(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", (n as f64 / 1e5).round() / 10.0)
    } else {
        format!("{}k", (n as f64 / 1000.0).round() as u64)
    }
}

fn two(n: u64) -> String {
    format!("{:02}", n)
}

/// "just now", "4m ago", "3h ago", "2d ago"
pub fn ago(t: f64, now: f64) -> String {
    let s = (now - t).max(0.0);
    if s < 60.0 {
        "just now".into()
    } else if s < 3600.0 {
        format!("{}m ago", (s / 60.0).floor() as u64)
    } else if s < 86400.0 {
        format!("{}h ago", (s / 3600.0).floor() as u64)
    } else {
        format!("{}d ago", (s / 86400.0).floor() as u64)
    }
}

/// Short age for rows: "now", "4m", "3h", "2d"
pub fn sago(t: f64, now: f64) -> String {
    let s = (now - t).max(0.0);
    if s < 60.0 {
        "now".into()
    } else if s < 3600.0 {
        format!("{}m", (s / 60.0).floor() as u64)
    } else if s < 86400.0 {
        format!("{}h", (s / 3600.0).floor() as u64)
    } else {
        format!("{}d", (s / 86400.0).floor() as u64)
    }
}

/// Seconds as "46m" or "1h 12m" (never "0m").
pub fn mins(s: u64) -> String {
    if s >= 3600 {
        format!("{}h {}m", s / 3600, s % 3600 / 60)
    } else {
        format!("{}m", ((s as f64 / 60.0).round() as u64).max(1))
    }
}

/// Seconds as a countdown clock, "45:59".
pub fn clock(s: u64) -> String {
    format!("{}:{}", s / 60, two(s % 60))
}

/// "2d 03:04:05", "3:04:05", "4:05"
pub fn dhms(sec: f64) -> String {
    let sec = sec.max(0.0).floor() as u64;
    let (d, h, m, s) = (sec / 86400, sec % 86400 / 3600, sec % 3600 / 60, sec % 60);
    if d > 0 {
        format!("{}d {}:{}:{}", d, two(h), two(m), two(s))
    } else if h > 0 {
        format!("{}:{}:{}", h, two(m), two(s))
    } else {
        format!("{}:{}", m, two(s))
    }
}

/// "opus-5-5" -> "Opus 5.5", "claude-sonnet-4-5-20250929" -> "Sonnet 4.5", "<synthetic>" -> "Unknown"
pub fn model(raw: &str) -> String {
    for fam in ["opus", "sonnet", "haiku", "fable"] {
        if let Some(i) = raw.find(&format!("{fam}-")) {
            let rest = &raw[i + fam.len() + 1..];
            let major: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if major.is_empty() {
                continue;
            }
            let after = &rest[major.len()..];
            let mut minor = String::new();
            if let Some(r) = after.strip_prefix('-') {
                let digits: String = r.chars().take_while(|c| c.is_ascii_digit()).collect();
                if (1..=2).contains(&digits.len()) {
                    minor = digits;
                }
            }
            let mut name = fam.to_string();
            name[..1].make_ascii_uppercase();
            return if minor.is_empty() { format!("{name} {major}") } else { format!("{name} {major}.{minor}") };
        }
    }
    if raw.is_empty() || raw.starts_with('<') {
        "Unknown".into()
    } else {
        raw.strip_prefix("claude-").unwrap_or(raw).to_string()
    }
}

/// The last path segment: "switchboard/.claude/worktrees/x" -> "x", and "C:\Users\you\code" (a Windows machine's) -> "code".
pub fn last_seg(f: &str) -> String {
    f.split(['/', '\\']).rfind(|s| !s.is_empty()).unwrap_or(f).to_string()
}

fn ampm<T: Timelike>(t: &T, drop_zero: bool) -> String {
    let (pm, h) = t.hour12();
    let m = t.minute();
    let suffix = if pm { "pm" } else { "am" };
    if drop_zero && m == 0 { format!("{h}{suffix}") } else { format!("{h}:{}{suffix}", two(m as u64)) }
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const WEEKDAYS_LONG: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

fn at<Tz: TimeZone>(epoch: f64, tz: &Tz) -> DateTime<Tz> {
    let secs = epoch.floor() as i64;
    let nanos = ((epoch - epoch.floor()) * 1e9) as u32;
    tz.timestamp_opt(secs, nanos).single().unwrap_or_else(|| tz.timestamp_opt(0, 0).unwrap())
}

fn when_tz<Tz: TimeZone>(epoch: f64, now: f64, tz: &Tz) -> String {
    if epoch <= 0.0 {
        return String::new();
    }
    let t = at(epoch, tz);
    let n = at(now, tz);
    let tm = ampm(&t, true);
    if t.date_naive() == n.date_naive() {
        tm
    } else if (epoch - now).abs() < 6.0 * 86400.0 {
        format!("{} {}", WEEKDAYS[t.weekday().num_days_from_monday() as usize], tm)
    } else {
        format!("{} {}", MONTHS[t.month0() as usize], t.day())
    }
}

/// "6:41pm" today, "Tue 9pm" within six days, "Oct 2" otherwise. Local time zone.
pub fn when(epoch: f64, now: f64) -> String {
    when_tz(epoch, now, &Local)
}

/// `when` at a fixed UTC offset (seconds east), for tests and for anyone who wants a stable zone.
pub fn when_at_offset(epoch: f64, now: f64, offset_secs: i32) -> String {
    when_tz(epoch, now, &FixedOffset::east_opt(offset_secs).unwrap_or(FixedOffset::east_opt(0).unwrap()))
}

/// "6:41pm" (keeps ":00").
pub fn time(epoch: f64) -> String {
    ampm(&at(epoch, &Local), false)
}

pub fn time_at_offset(epoch: f64, offset_secs: i32) -> String {
    ampm(&at(epoch, &FixedOffset::east_opt(offset_secs).unwrap_or(FixedOffset::east_opt(0).unwrap())), false)
}

/// "Saturday, October 11"
pub fn day(epoch: f64) -> String {
    let t = at(epoch, &Local);
    format!(
        "{}, {} {}",
        WEEKDAYS_LONG[t.weekday().num_days_from_monday() as usize],
        MONTHS_LONG[t.month0() as usize],
        t.day()
    )
}

/// History's day headings: "Today", "Yesterday", then "Saturday, October 11". Local time.
pub fn day_heading(epoch: f64, now: f64) -> String {
    day_heading_tz(epoch, now, &Local)
}

pub fn day_heading_at_offset(epoch: f64, now: f64, offset_secs: i32) -> String {
    day_heading_tz(epoch, now, &FixedOffset::east_opt(offset_secs).unwrap_or(FixedOffset::east_opt(0).unwrap()))
}

fn day_heading_tz<Z: TimeZone>(epoch: f64, now: f64, tz: &Z) -> String {
    let (t, n) = (at(epoch, tz).date_naive(), at(now, tz).date_naive());
    match (n - t).num_days() {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        _ => {
            let d = at(epoch, tz);
            let mut s = format!(
                "{}, {} {}",
                WEEKDAYS_LONG[d.weekday().num_days_from_monday() as usize],
                MONTHS_LONG[d.month0() as usize],
                d.day()
            );
            if d.year() != at(now, tz).year() {
                s.push_str(&format!(", {}", d.year()));
            }
            s
        }
    }
}

/// "Oct 9", local time ("" for no time). History rows and their summary.
pub fn md(epoch: f64) -> String {
    if epoch <= 0.0 {
        return String::new();
    }
    let t = at(epoch, &Local);
    format!("{} {}", MONTHS[t.month0() as usize], t.day())
}

pub fn md_at_offset(epoch: f64, offset_secs: i32) -> String {
    if epoch <= 0.0 {
        return String::new();
    }
    let t = at(epoch, &FixedOffset::east_opt(offset_secs).unwrap_or(FixedOffset::east_opt(0).unwrap()));
    format!("{} {}", MONTHS[t.month0() as usize], t.day())
}

/// Megabytes as the machine header says them: 3120 -> "3.1 GB", 410 -> "410 MB".
pub fn mem(mb: f64) -> String {
    if mb >= 1000.0 { format!("{:.1} GB", mb / 1000.0) } else { format!("{} MB", mb.round().max(0.0) as u64) }
}

/// The month and day prefix the server stamps on names: "Oct 11 3:40p ".
pub fn month_short(i: usize) -> &'static str {
    MONTHS[i % 12]
}

pub fn now_epoch() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {

    #[test]
    fn last_seg_takes_windows_paths_too() {
        assert_eq!(last_seg("switchboard/.claude/worktrees/x"), "x");
        assert_eq!(last_seg("C:\\Users\\you\\Desktop\\Claude"), "Claude");
        assert_eq!(last_seg("C:\\Users\\you\\My Code\\"), "My Code");
        assert_eq!(last_seg("app\\src"), "src");
        assert_eq!(last_seg("plain"), "plain");
    }

    #[test]
    fn day_headings_say_today_yesterday_then_the_date() {
        // 2026-10-13 12:00 UTC
        let now = 1_791_892_800.0;
        assert_eq!(day_heading_at_offset(now - 3600.0, now, 0), "Today");
        assert_eq!(day_heading_at_offset(now - 86400.0, now, 0), "Yesterday");
        assert_eq!(day_heading_at_offset(now - 3.0 * 86400.0, now, 0), "Saturday, October 10");
        assert_eq!(day_heading_at_offset(now - 400.0 * 86400.0, now, 0), "Monday, September 8, 2025");
    }

    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(k(180353), "180k");
        assert_eq!(k(0), "0k");
        assert_eq!(k(1_250_000), "1.3M");
        assert_eq!(mins(30), "1m");
        assert_eq!(mins(46 * 60), "46m");
        assert_eq!(mins(3600 + 12 * 60), "1h 12m");
        assert_eq!(clock(2759), "45:59");
        assert_eq!(clock(5), "0:05");
        assert_eq!(dhms(3.0 * 86400.0 + 4.0 * 3600.0 + 5.0), "3d 04:00:05");
        assert_eq!(dhms(65.0), "1:05");
    }

    #[test]
    fn ages() {
        assert_eq!(ago(100.0, 130.0), "just now");
        assert_eq!(ago(0.0, 600.0), "10m ago");
        assert_eq!(sago(0.0, 7300.0), "2h");
        assert_eq!(sago(0.0, 3.0 * 86400.0), "3d");
        assert_eq!(sago(10.0, 5.0), "now");
    }

    #[test]
    fn models() {
        assert_eq!(model("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model("claude-sonnet-4-5-20250929"), "Sonnet 4.5");
        assert_eq!(model("claude-haiku-4-20250101"), "Haiku 4");
        assert_eq!(model("claude-fable-1"), "Fable 1");
        assert_eq!(model("<synthetic>"), "Unknown");
        assert_eq!(model(""), "Unknown");
        assert_eq!(model("claude-mystery"), "mystery");
    }

    #[test]
    fn whens() {
        let pdt = -7 * 3600;
        let now = 1_791_761_909.0; // 2026-10-11 16:38:29 PDT
        let t = now + 2.0 * 3600.0 + 3.0 * 60.0;
        assert_eq!(when_at_offset(t, now, pdt), "6:41pm");
        assert_eq!(when_at_offset(now - 3.0 * 86400.0, now, pdt), "Thu 4:38pm");
        assert_eq!(when_at_offset(now - 9.0 * 86400.0, now, pdt), "Oct 2");
        assert_eq!(time_at_offset(now - 38.0 * 60.0 - 29.0, pdt), "4:00pm");
        assert_eq!(when_at_offset(now - 38.0 * 60.0 - 29.0, now, pdt), "4pm");
        assert_eq!(when_at_offset(0.0, now, pdt), "");
    }
}
