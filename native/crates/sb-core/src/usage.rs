//! The usage panel's numbers (GET /api/usage): this week as one slot per local day since the reset, and the last
//! four weeks. Pure, so the Mac and any other client draw the same bars.

use chrono::{Datelike, Duration, FixedOffset, NaiveDate, TimeZone};

use crate::api::{ApiUsage, ApiUsageWeek};

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// One bar of "This week by day".
#[derive(Debug, Clone, PartialEq)]
pub struct DaySlot {
    pub date: NaiveDate,
    /// "Tue"
    pub label: &'static str,
    /// Weekly-percent points used that day, never negative. None when the server has no number for it.
    pub used: Option<f64>,
    pub today: bool,
    /// Later this week: an empty slot, so the week's shape shows from its first day.
    pub future: bool,
}

impl DaySlot {
    /// "Tue Oct 13"
    pub fn long_label(&self) -> String {
        format!("{} {} {}", self.label, crate::fmt::month_short(self.date.month0() as usize), self.date.day())
    }
}

fn local_date(epoch: f64, tz: &FixedOffset) -> Option<NaiveDate> {
    tz.timestamp_opt(epoch.floor() as i64, 0).single().map(|t| t.date_naive())
}

/// This week as day slots, oldest first: from the day the week began (the reset a week before `week_reset`) to the
/// last day of the week, today included. Days the server has no row for are None; days after today are `future`.
/// `offset_secs`: the local UTC offset (the server counts days in America/Los_Angeles, like the Mac it runs for).
pub fn week_days(u: &ApiUsage, week_reset: Option<f64>, now: f64, offset_secs: i32) -> Vec<DaySlot> {
    let Some(tz) = FixedOffset::east_opt(offset_secs) else { return Vec::new() };
    let Some(today) = local_date(now, &tz) else { return Vec::new() };
    let rows: Vec<(NaiveDate, Option<f64>)> = u
        .days
        .iter()
        .filter_map(|d| {
            NaiveDate::parse_from_str(d.day.trim(), "%Y-%m-%d").ok().map(|day| (day, d.used.map(|x| x.max(0.0))))
        })
        .collect();
    let reset = week_reset.or(u.week.as_ref().and_then(|w| w.reset)).filter(|r| *r > 0.0);
    let mut start = match reset.and_then(|r| local_date(r - 7.0 * 86400.0, &tz)) {
        Some(d) => d,
        None => rows.iter().map(|r| r.0).min().unwrap_or(today),
    };
    // A reset that has passed (numbers not caught up yet) or a far-off first row: keep to the last week.
    if start > today {
        start = today;
    }
    if today - start > Duration::days(7) {
        start = today - Duration::days(6);
    }
    let last_row = rows.iter().map(|r| r.0).filter(|d| *d >= start).max();
    let mut end = start + Duration::days(6);
    // The reset's own day can hold a few hours of this week: one more slot when the server has a row for it.
    if let Some(l) = last_row.filter(|l| *l > end) {
        end = l.min(start + Duration::days(7));
    }
    if today > end {
        end = today.min(start + Duration::days(7));
    }
    let mut out = Vec::new();
    let mut d = start;
    while d <= end {
        out.push(DaySlot {
            date: d,
            label: DAYS[d.weekday().num_days_from_monday() as usize],
            used: rows.iter().find(|r| r.0 == d).and_then(|r| r.1),
            today: d == today,
            future: d > today,
        });
        d += Duration::days(1);
    }
    out
}

/// The top of the day chart's scale: a little over the biggest day, and never under 2 points so a quiet week
/// does not fill the chart with one small bar.
pub fn day_scale(slots: &[DaySlot]) -> f64 {
    let top = slots.iter().filter_map(|s| s.used).fold(0.0_f64, f64::max);
    (top * 1.15).max(2.0)
}

/// The last `n` past weeks, oldest first (the server sends newest first).
pub fn last_weeks(u: &ApiUsage, n: usize) -> Vec<ApiUsageWeek> {
    let mut w: Vec<ApiUsageWeek> = u.weeks.iter().take(n).cloned().collect();
    w.reverse();
    w
}

/// The average final percent of those weeks, over the ones with a number.
pub fn weeks_avg(weeks: &[ApiUsageWeek]) -> Option<f64> {
    let known: Vec<f64> = weeks.iter().filter_map(|w| w.pct).collect();
    if known.is_empty() { None } else { Some(known.iter().sum::<f64>() / known.len() as f64) }
}

/// "4.5" or "12": points with one decimal under 10, whole above.
pub fn pts(x: f64) -> String {
    if x < 9.95 { format!("{:.1}", x).trim_end_matches(".0").to_string() } else { format!("{}", x.round() as i64) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ApiUsageDay, ApiUsageWindow};

    const PT: i32 = -7 * 3600;

    fn epoch(y: i32, m: u32, d: u32, h: u32) -> f64 {
        FixedOffset::east_opt(PT).unwrap().with_ymd_and_hms(y, m, d, h, 0, 0).unwrap().timestamp() as f64
    }

    fn usage(days: &[(&str, Option<f64>)], weeks: &[Option<f64>]) -> ApiUsage {
        ApiUsage {
            ok: true,
            days: days.iter().map(|(d, u)| ApiUsageDay { day: d.to_string(), used: *u }).collect(),
            weeks: weeks
                .iter()
                .enumerate()
                .map(|(i, p)| ApiUsageWeek { reset: Some(1000.0 - i as f64), pct: *p })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn a_week_is_seven_days_from_the_reset_with_today_marked() {
        // The week resets Thu Oct 15 at 10:00, so it began Thu Oct 8; today is Tue Oct 13.
        let u = usage(&[("2026-10-08", Some(3.0)), ("2026-10-09", Some(-1.0)), ("2026-10-13", Some(4.5))], &[]);
        let s = week_days(&u, Some(epoch(2026, 10, 15, 10)), epoch(2026, 10, 13, 15), PT);
        assert_eq!(s.len(), 7);
        assert_eq!(s[0].label, "Thu");
        assert_eq!(s[0].used, Some(3.0));
        assert_eq!(s[1].used, Some(0.0), "never negative");
        assert_eq!(s[2].used, None, "no row: unknown, not zero");
        assert!(s[5].today && s[5].label == "Tue" && s[5].used == Some(4.5));
        assert!(s[6].future && !s[4].future);
        assert_eq!(s[5].long_label(), "Tue Oct 13");
    }

    #[test]
    fn the_reset_day_gets_its_own_slot_when_the_server_has_it() {
        let u = usage(&[("2026-10-08", Some(1.0)), ("2026-10-15", Some(2.0))], &[]);
        let s = week_days(&u, Some(epoch(2026, 10, 15, 10)), epoch(2026, 10, 15, 9), PT);
        assert_eq!(s.len(), 8);
        assert!(s[7].today && s[7].used == Some(2.0));
    }

    #[test]
    fn without_a_reset_the_rows_set_the_start() {
        let u = usage(&[("2026-10-12", Some(1.0)), ("2026-10-13", None)], &[]);
        let s = week_days(&u, None, epoch(2026, 10, 13, 12), PT);
        assert_eq!(s[0].date, NaiveDate::from_ymd_opt(2026, 10, 12).unwrap());
        assert_eq!(s.len(), 7);
        assert!(s[1].today);
        // Nothing at all: still today's slot, so the chart has a place to start.
        let s = week_days(&ApiUsage::default(), None, epoch(2026, 10, 13, 12), PT);
        assert!(s[0].today);
    }

    #[test]
    fn weeks_come_oldest_first_and_average_the_known_ones() {
        let u = usage(&[], &[Some(78.0), None, Some(40.0), Some(60.0), Some(99.0)]);
        let w = last_weeks(&u, 4);
        assert_eq!(w.iter().map(|w| w.pct).collect::<Vec<_>>(), vec![Some(60.0), Some(40.0), None, Some(78.0)]);
        assert_eq!(weeks_avg(&w), Some(178.0 / 3.0));
        assert_eq!(weeks_avg(&[]), None);
        assert_eq!(last_weeks(&ApiUsage::default(), 4).len(), 0);
    }

    #[test]
    fn scale_and_points() {
        let u = usage(&[("2026-10-13", Some(10.0))], &[]);
        let s = week_days(&u, None, epoch(2026, 10, 13, 12), PT);
        assert!((day_scale(&s) - 11.5).abs() < 1e-9);
        assert_eq!(day_scale(&[]), 2.0);
        assert_eq!(pts(4.5), "4.5");
        assert_eq!(pts(4.0), "4");
        assert_eq!(pts(12.4), "12");
    }

    #[test]
    fn usage_parses_from_the_contract_and_from_odd_values() {
        let raw = r#"{"ok": true, "now": 0, "tz": "America/Los_Angeles",
            "five": {"pct": 12.0, "reset": 0}, "week": {"pct": 31.0, "reset": 0, "pace": null},
            "days": [{"day": "2026-10-13", "used": 4.5}], "weeks": [{"reset": 0, "pct": 78.0}]}"#;
        let u: ApiUsage = serde_json::from_str(raw).unwrap();
        assert_eq!(u.five, Some(ApiUsageWindow { pct: Some(12.0), reset: Some(0.0), pace: None }));
        assert_eq!(u.days[0].used, Some(4.5));
        assert_eq!(u.weeks[0].pct, Some(78.0));
        let odd: ApiUsage = serde_json::from_str(
            r#"{"week": {"pct": "x"}, "days": [{"day": "2026-10-13", "used": null}, 5], "weeks": null}"#,
        )
        .unwrap();
        assert_eq!(odd.week.unwrap().pct, None);
        assert_eq!(odd.days.len(), 1);
        assert!(odd.weeks.is_empty());
        let empty: ApiUsage = serde_json::from_str("{}").unwrap();
        assert!(!empty.ok && empty.days.is_empty());
    }
}
