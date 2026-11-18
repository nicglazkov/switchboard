//! The usage panel's numbers (GET /api/usage): this week as one slot per local day since the reset, and the last
//! four weeks. Pure, so the Mac and any other client draw the same bars.

use chrono::{Datelike, Duration, FixedOffset, NaiveDate, TimeZone};

use crate::api::{ApiAccount, ApiMoney, ApiUsage, ApiUsageWeek};
use crate::fmt;
use crate::model::{Plan, Tone};

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
///
/// The server counts the days in its own time zone and its rows run from the week's first day to its today, so the
/// rows say which day is today and where the week starts. `offset_secs`, this computer's UTC offset, stands in only
/// when there are no rows, or they are too far from this computer's date to be current.
pub fn week_days(u: &ApiUsage, week_reset: Option<f64>, now: f64, offset_secs: i32) -> Vec<DaySlot> {
    let Some(tz) = FixedOffset::east_opt(offset_secs) else { return Vec::new() };
    let Some(here) = local_date(now, &tz) else { return Vec::new() };
    let rows: Vec<(NaiveDate, Option<f64>)> = u
        .days
        .iter()
        .filter_map(|d| {
            NaiveDate::parse_from_str(d.day.trim(), "%Y-%m-%d").ok().map(|day| (day, d.used.map(|x| x.max(0.0))))
        })
        .collect();
    // Two places are never more than a day apart on the calendar: a last row further off is an old report.
    let server_today = rows.iter().map(|r| r.0).max().filter(|d| (*d - here).num_days().abs() <= 1);
    let today = server_today.unwrap_or(here);
    let reset = week_reset.or(u.week.as_ref().and_then(|w| w.reset)).filter(|r| *r > 0.0);
    let first_row = rows.iter().map(|r| r.0).min();
    let mut start = match (server_today.and(first_row), reset.and_then(|r| local_date(r - 7.0 * 86400.0, &tz))) {
        // The reset's day in this computer's zone can be a day off the server's: its first row, when next to it, wins.
        (Some(f), Some(d)) if (f - d).num_days().abs() <= 1 => f,
        (_, Some(d)) => d,
        (_, None) => first_row.unwrap_or(today),
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

// ---------- the account's limits, extra usage and products, as the panel draws them ----------

/// Where to manage the plan and extra usage: the Claude apps' own usage page.
pub const MANAGE_URL: &str = "https://claude.ai/settings/usage";

/// Numbers older than this say so in words (and why), not only by their age.
pub const ACCOUNT_STALE: f64 = 2.5 * 3600.0;

/// One limit of the plan, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct LimitRow {
    /// "5-hour session", "Week, all models", "Week, Fable"
    pub title: String,
    /// Percent used, 0 to 100. None once the window has reset and nothing newer was seen.
    pub pct: Option<f64>,
    pub tone: Tone,
    /// "Resets Tue 9pm, in 6d 20h" or what stands in for it.
    pub note: String,
    /// Where the week ends at this rate (the all-models week only).
    pub pace: Option<f64>,
    /// The number is older than the 5-hour window, and use since then only adds: "at least 62%".
    pub at_least: bool,
}

fn pct_ok(x: f64) -> Option<f64> {
    x.is_finite().then(|| x.clamp(0.0, 100.0))
}

fn tone_of(pct: f64, severity: Option<&str>) -> Tone {
    match severity {
        Some("critical") | Some("serious") => Tone::Bad,
        Some("warning") => Tone::Warn,
        // The source's own reading stands; without one, the meter grades itself.
        Some(_) => Tone::Ok,
        None if pct >= 90.0 => Tone::Bad,
        None if pct >= 75.0 => Tone::Warn,
        None => Tone::Ok,
    }
}

/// "Resets Tue 9pm, in 6d 20h", in this computer's time zone.
pub fn reset_note(reset: f64, now: f64) -> String {
    reset_note_with(reset, now, &fmt::when_exact(reset, now))
}

/// `reset_note` at a fixed UTC offset (seconds east), for tests.
pub fn reset_note_at_offset(reset: f64, now: f64, offset_secs: i32) -> String {
    reset_note_with(reset, now, &fmt::when_exact_at_offset(reset, now, offset_secs))
}

fn reset_note_with(reset: f64, now: f64, when: &str) -> String {
    format!("Resets {when}, in {}", fmt::until(reset - now))
}

fn scoped_title(label: Option<&str>) -> String {
    match label.map(str::trim).filter(|l| !l.is_empty()) {
        Some(l) => format!("Week, {l}"),
        None => "Week, one model".into(),
    }
}

/// Every limit the account has, top to bottom: the 5-hour session, the week for all models, then each model's week.
///
/// The session and the all-models week are the numbers the header chip shows (`plan`: the highest any chat or
/// report has seen in the window, so they never dip); the account's report adds the rows only it knows. A window
/// whose reset has passed shows no number until a new one is seen.
pub fn limit_rows(plan: &Plan, account: Option<&ApiAccount>, now: f64) -> Vec<LimitRow> {
    let mut out = Vec::new();
    let acct = |kind: &str| account.and_then(|a| a.limits.iter().find(|l| l.kind == kind));
    // The 5-hour session: only when someone reports one.
    let five = plan.five_hour.zip(plan.five_hour_reset).or_else(|| acct("session").and_then(|l| l.pct.zip(l.reset)));
    if let Some((p, r)) = five {
        let live = r > now;
        out.push(LimitRow {
            title: "5-hour session".into(),
            pct: pct_ok(p).filter(|_| live),
            // The header's own grading when the number is the header's; the source's otherwise.
            tone: match (live, plan.five_hour.is_some()) {
                (false, _) => Tone::Ok,
                (true, true) => plan.five_tone(),
                (true, false) => tone_of(p, acct("session").and_then(|l| l.severity.as_deref())),
            },
            note: if live {
                reset_note(r, now)
            } else {
                "This window has reset. A new one starts with your next message.".into()
            },
            pace: None,
            at_least: false,
        });
    }
    let week = plan.week.zip(plan.week_reset).or_else(|| acct("weekly_all").and_then(|l| l.pct.zip(l.reset)));
    match week {
        Some((p, r)) if r > now => out.push(LimitRow {
            title: "Week, all models".into(),
            pct: pct_ok(p),
            tone: if plan.week.is_some() { plan.week_tone() } else { tone_of(p, None) },
            note: reset_note(r, now),
            pace: plan.week_pace(now).filter(|x| x.is_finite()).map(|x| x.max(0.0)),
            at_least: plan.week.is_some() && plan.limits_age(now).is_some_and(|a| a > crate::model::FIVE_WINDOW),
        }),
        Some(_) => out.push(LimitRow {
            title: "Week, all models".into(),
            pct: None,
            tone: Tone::Ok,
            note: "The week has reset. New numbers come with the next reply.".into(),
            pace: None,
            at_least: false,
        }),
        None => {}
    }
    for l in account.map(|a| a.limits.as_slice()).unwrap_or_default() {
        if l.kind == "session" || l.kind == "weekly_all" {
            continue;
        }
        let Some(p) = l.pct.and_then(pct_ok) else { continue };
        let live = l.reset.is_none_or(|r| r > now);
        out.push(LimitRow {
            title: scoped_title(l.label.as_deref()),
            pct: live.then_some(p),
            tone: if live { tone_of(p, l.severity.as_deref()) } else { Tone::Ok },
            note: match l.reset {
                Some(r) if r > now => reset_note(r, now),
                Some(_) => "Has reset.".into(),
                None => "Reset time not reported".into(),
            },
            pace: None,
            at_least: false,
        });
    }
    out
}

/// The quiet line for a limit the account does not report: today, a plan with no 5-hour window. None when every
/// limit is there, or when nothing is known yet at all.
pub fn missing_limits(plan: &Plan, account: Option<&ApiAccount>, now: f64) -> Option<&'static str> {
    let reported = account.is_some_and(|a| a.at.is_some() && a.available != Some(false) && !a.limits.is_empty());
    let has_five = plan.five_hour.is_some() || account.is_some_and(|a| a.limits.iter().any(|l| l.kind == "session"));
    (!has_five && (reported || plan.five_unreported(now))).then_some("5-hour session: not reported on this plan.")
}

/// "Max plan" from "max"; None when the login is not a Claude plan or nobody said.
pub fn plan_name(account: Option<&ApiAccount>) -> Option<String> {
    let p = account?.plan.as_deref()?.trim();
    let mut c = p.chars();
    let first = c.next()?;
    Some(format!("{}{} plan", first.to_uppercase(), c.as_str().replace('_', " ")))
}

/// An amount as people write it: "$77.25", "€1,204.00", "77.25 CAD". The source sends minor units and how many
/// decimals the currency has (7725 with 2 is 77.25); nothing is assumed about it. None when there is no amount.
pub fn money(m: &ApiMoney) -> Option<String> {
    let minor = m.minor.filter(|x| x.is_finite())?;
    let exp = m.exp.filter(|e| e.is_finite() && (0.0..=4.0).contains(e)).unwrap_or(2.0) as u32;
    let unit = 10_i64.pow(exp);
    let total = minor.round() as i64;
    let (whole, frac) = (total.abs() / unit, total.abs() % unit);
    let digits = whole.to_string();
    let mut grouped = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let num = if exp == 0 { grouped } else { format!("{grouped}.{frac:0width$}", width = exp as usize) };
    let sign = if total < 0 { "-" } else { "" };
    let code = m.currency.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(str::to_uppercase);
    Some(match code.as_deref() {
        Some("USD") => format!("{sign}${num}"),
        Some("EUR") => format!("{sign}€{num}"),
        Some("GBP") => format!("{sign}£{num}"),
        Some(c) => format!("{sign}{num} {c}"),
        None => format!("{sign}{num}"),
    })
}

/// Extra usage, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtraView {
    /// "On", "Off", or "Limit reached" when the month's limit is what stopped it.
    pub state: &'static str,
    pub on: bool,
    pub tone: Tone,
    /// "$12.34"
    pub spent: Option<String>,
    pub limit: Option<String>,
    /// What is left under the monthly limit, never below zero.
    pub left: Option<String>,
    /// A prepaid balance, when the account has one.
    pub balance: Option<String>,
    /// Percent of the monthly limit spent, 0 to 100.
    pub pct: Option<f64>,
    pub note: Option<String>,
}

/// None when the account reports nothing about extra usage (an older helper, or a plan without it).
pub fn extra_view(account: Option<&ApiAccount>) -> Option<ExtraView> {
    let e = account?.extra.as_ref()?;
    let same = |a: &ApiMoney, b: &ApiMoney| a.currency == b.currency && a.exp == b.exp;
    let (used, limit) = (e.used.as_ref(), e.limit.as_ref());
    let left = match (used, limit) {
        (Some(u), Some(l)) if same(u, l) => u.minor.zip(l.minor).and_then(|(spent, cap)| {
            money(&ApiMoney { minor: Some((cap - spent).max(0.0)), currency: l.currency.clone(), exp: l.exp })
        }),
        _ => None,
    };
    let pct = e.pct.and_then(pct_ok).or_else(|| match (used.and_then(|u| u.minor), limit.and_then(|l| l.minor)) {
        (Some(u), Some(l)) if l > 0.0 => pct_ok(u / l * 100.0),
        _ => None,
    });
    let on = e.enabled == Some(true);
    let reached = e.limit_reached == Some(true) || (limit.is_some() && pct.is_some_and(|p| p >= 100.0));
    let (state, tone) = match (on, reached) {
        (_, true) => ("Limit reached", Tone::Bad),
        (true, _) => ("On", if pct.is_some_and(|p| p >= 80.0) { Tone::Warn } else { Tone::Ok }),
        (false, _) => ("Off", Tone::Ok),
    };
    let note = if reached {
        Some(
            "This month's spend limit is used up, so extra usage can't cover more until it resets or you raise it."
                .into(),
        )
    } else if !on {
        Some("When it's on, extra usage keeps Claude going after you reach a plan limit.".into())
    } else if e.auto_reload == Some(true) {
        Some("Auto-reload is on.".into())
    } else {
        None
    };
    Some(ExtraView {
        state,
        on,
        tone,
        spent: used.and_then(money),
        limit: limit.and_then(money),
        left,
        balance: e.balance.as_ref().and_then(money),
        pct,
        note,
    })
}

/// One product's share of the week so far.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductRow {
    pub name: String,
    /// Share of the week's use so far, 0 to 100.
    pub share: f64,
    /// The same use as percent of the weekly limit, when the account says.
    pub of_limit: Option<f64>,
}

/// The week by product, biggest first (the source's order breaks ties). Empty when the account reports none.
pub fn product_rows(account: Option<&ApiAccount>) -> Vec<ProductRow> {
    let Some(p) = account.and_then(|a| a.products.as_ref()) else { return Vec::new() };
    let mut rows: Vec<ProductRow> = p
        .rows
        .iter()
        .filter_map(|r| {
            let name = r.name.as_deref().map(str::trim).filter(|n| !n.is_empty())?;
            Some(ProductRow {
                name: name.to_string(),
                share: r.pct.and_then(pct_ok)?,
                of_limit: r.of_limit.and_then(pct_ok),
            })
        })
        .collect();
    rows.sort_by(|a, b| b.share.partial_cmp(&a.share).unwrap_or(std::cmp::Ordering::Equal));
    rows
}

/// How many chats "Most used this week" shows.
pub const TOP_SHOWN: usize = 5;

/// One row of "Most used this week".
#[derive(Debug, Clone, PartialEq)]
pub struct TopRow {
    /// The chat's session key ("machine:sid") when the server says the apps list it, so the row can open it.
    pub key: Option<String>,
    pub name: String,
    /// Where it ran, and how many chats the row adds up when it is a thread: "Studio", "Studio, 3 chats".
    pub place: String,
    /// Share of everything Switchboard saw this week, 0 to 100.
    pub share: f64,
    /// The share in words: "42%", "<1%".
    pub pct: String,
    pub sent: bool,
}

/// What the "Most used this week" part of the panel shows.
#[derive(Debug, Clone, PartialEq)]
pub enum TopView {
    /// Nothing at all: an older server, or no machine to speak of.
    Hidden,
    /// One quiet line where the list would be: why there is none yet.
    Note(String),
    /// The chats, biggest first; the chats not listed as one line ("12 other chats", "18%"); the footnote.
    Rows { rows: Vec<TopRow>, rest: Option<(String, String)>, foot: String },
}

/// A share as the rows say it: whole percents, and "<1%" for a chat that used something but rounds to nothing.
pub fn share_pct(share: f64) -> String {
    if share > 0.0 && share < 0.5 { "<1%".into() } else { format!("{}%", share.round() as i64) }
}

/// "Most used this week" from GET /api/usage `top`: the `limit` biggest chats, each with its share of what
/// Switchboard saw, or the one line that says why there are none. The shares are estimates from tokens, and the
/// footnote says so: they are never shown as the plan's own percent.
pub fn top_view(u: Option<&ApiUsage>, limit: usize) -> TopView {
    let Some(top) = u.and_then(|u| u.top.as_ref()) else { return TopView::Hidden };
    let count = |x: Option<f64>| x.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.0) as i64;
    let (able, machines, pending) = (count(top.able), count(top.machines), count(top.pending));
    let total = top.total.filter(|t| t.is_finite() && *t > 0.0);
    let mut all: Vec<(&crate::api::ApiTopRow, f64)> = top
        .rows
        .iter()
        .filter_map(|r| {
            let w = r.weighted.filter(|w| w.is_finite() && *w > 0.0)?;
            // The share is worked out here when the total is known, so a row and the rest always add up.
            let share = total.map(|t| 100.0 * w / t).or(r.share).and_then(pct_ok)?;
            (!r.sid.is_empty()).then_some((r, share))
        })
        .collect();
    all.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    if all.is_empty() {
        return if machines == 0 {
            TopView::Hidden
        } else if able == 0 {
            TopView::Note("Shows once a machine runs the current Switchboard helper.".into())
        } else if top.since.is_none() {
            TopView::Note("Shows once a chat has replied, so the week's reset is known.".into())
        } else if pending > 0 {
            TopView::Note("Reading this week's chats. The list shows in a few minutes.".into())
        } else {
            TopView::Note("No chat has used your plan yet this week.".into())
        };
    }
    let rows: Vec<TopRow> = all
        .iter()
        .take(limit)
        .map(|(r, share)| {
            let label = Some(r.label.trim()).filter(|l| !l.is_empty()).unwrap_or(r.machine.trim());
            let parts = count(r.parts);
            TopRow {
                key: (r.alive == Some(true) && !r.machine.is_empty()).then(|| format!("{}:{}", r.machine, r.sid)),
                // The name without its stamp or its "(fresh)": the row is the whole thread.
                name: Some(r.name.trim())
                    .filter(|n| !n.is_empty())
                    .map(|n| crate::names::split(n).0)
                    .unwrap_or_else(|| "Untitled chat".into()),
                place: if parts > 1 { format!("{label}, {parts} chats") } else { label.to_string() },
                share: *share,
                pct: share_pct(*share),
                sent: r.sent == Some(true),
            }
        })
        .collect();
    let others = count(top.chats).max(all.len() as i64) - rows.len() as i64;
    let left = (100.0 - rows.iter().map(|r| r.share).sum::<f64>()).max(0.0);
    let rest = (others > 0 && left > 0.0)
        .then(|| (if others == 1 { "1 other chat".into() } else { format!("{others} other chats") }, share_pct(left)));
    let mut foot = "Estimated from each chat's tokens. Chats outside Switchboard aren't counted.".to_string();
    let behind = machines - able;
    if behind == 1 {
        foot.push_str(" 1 machine isn't counted until it runs the current Switchboard helper.");
    } else if behind > 1 {
        foot.push_str(&format!(" {behind} machines aren't counted until they run the current Switchboard helper."));
    }
    TopView::Rows { rows, rest, foot }
}

/// "just now", "2 min ago", "3 hr ago", "2 days ago": how old the numbers are, in full words for the header line.
pub fn age_words(age: f64) -> String {
    let s = if age.is_finite() { age.max(0.0) } else { 0.0 };
    let n = |x: f64, one: &str, many: &str| {
        let k = x.floor() as u64;
        format!("{k} {} ago", if k == 1 { one } else { many })
    };
    if s < 60.0 {
        "just now".into()
    } else if s < 3600.0 {
        n(s / 60.0, "min", "min")
    } else if s < 86400.0 {
        n(s / 3600.0, "hr", "hr")
    } else {
        n(s / 86400.0, "day", "days")
    }
}

/// When the numbers on the panel were last seen: the newer of the account's report and the last reply's limits.
pub fn seen_at(plan: &Plan, u: Option<&ApiUsage>) -> Option<f64> {
    let report = u.and_then(|u| u.account.as_ref()).and_then(|a| a.at);
    [plan.updated, u.and_then(|u| u.updated), report].into_iter().flatten().filter(|t| *t > 0.0).reduce(f64::max)
}

/// "Updated 2 min ago", or None when nobody has seen a number yet.
pub fn freshness(plan: &Plan, u: Option<&ApiUsage>, now: f64) -> Option<String> {
    seen_at(plan, u).map(|t| format!("Updated {}", age_words(now - t)))
}

/// A line that says why the account's details are missing, old or failing; None when they are there and current.
/// `tone` Warn marks a line that wants attention (something failed), Ok one that only explains.
pub fn account_note(u: Option<&ApiUsage>, now: f64) -> Option<(String, Tone)> {
    let u = u?;
    let sup = u.account_support.clone().unwrap_or_default();
    let count = |x: Option<f64>| x.filter(|v| v.is_finite()).unwrap_or(0.0) as i64;
    let Some(a) = u.account.as_ref() else {
        return Some(if u.account_support.is_none() {
            ("Per-model limits, extra usage and usage by product need a newer Switchboard server.".into(), Tone::Ok)
        } else if count(sup.machines) == 0 {
            ("Add a machine to see your plan's limits.".into(), Tone::Ok)
        } else if count(sup.able) == 0 {
            ("Per-model limits, extra usage and usage by product show once one machine runs the current Switchboard helper.".into(), Tone::Ok)
        } else if count(sup.online) == 0 {
            ("No machine that can ask Claude Code is online, so plan details will show when one is.".into(), Tone::Ok)
        } else {
            ("Asking Claude Code for your plan's details. This takes a few seconds.".into(), Tone::Ok)
        });
    };
    let who = a.machine.as_deref().map(str::trim).filter(|m| !m.is_empty()).unwrap_or("a machine");
    let why = a.error.as_deref().map(str::trim).filter(|e| !e.is_empty());
    match (a.at, why) {
        (None, Some(e)) => {
            Some((format!("Claude Code on {who} didn't give its usage: {e}. Refresh to try again."), Tone::Warn))
        }
        (None, None) => None,
        (Some(at), Some(e)) => Some((
            format!("The last refresh on {who} failed ({e}), so plan details are from {}.", age_words(now - at)),
            Tone::Warn,
        )),
        (Some(_), None) if a.available == Some(false) => Some((
            format!("Claude Code on {who} isn't signed in with a Claude plan, so it has no plan limits to report."),
            Tone::Ok,
        )),
        (Some(at), None) if now - at >= ACCOUNT_STALE => Some((
            if count(sup.online) == 0 {
                format!("Plan details are from {}: no machine that can ask Claude Code is online.", age_words(now - at))
            } else {
                format!("Plan details are from {}. Refresh to ask {who} again.", age_words(now - at))
            },
            Tone::Ok,
        )),
        _ => None,
    }
}

/// Should opening the panel ask for a new report? Yes when there is none or it is older than `max_age`, and some
/// machine could give one. An older server (no `account_support`) is never asked: it has no such route.
pub fn wants_refresh(u: Option<&ApiUsage>, now: f64, max_age: f64) -> bool {
    let Some(u) = u else { return false };
    let Some(sup) = u.account_support.as_ref() else { return false };
    if sup.online.unwrap_or(0.0) < 1.0 {
        return false;
    }
    let last = u.account.as_ref().and_then(|a| a.tried.or(a.at)).unwrap_or(0.0);
    now - last >= max_age
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
    fn today_is_the_servers_day_not_this_computers() {
        // The server (Los Angeles) says it is still Tue Oct 13; this computer is nine hours ahead, in Wed Oct 14.
        const AHEAD: i32 = 2 * 3600;
        let u = usage(&[("2026-10-08", Some(3.0)), ("2026-10-12", Some(2.0)), ("2026-10-13", Some(4.5))], &[]);
        let now = epoch(2026, 10, 13, 22);
        let s = week_days(&u, Some(epoch(2026, 10, 15, 10)), now, AHEAD);
        assert_eq!(s.len(), 7);
        assert!(s[5].today && s[5].used == Some(4.5), "the server's last row is today");
        assert!(s[6].future, "tomorrow there is still to come, though it has begun here");
        // The week began Thu Oct 8 at 10:00 there, which is already Oct 8 evening here: the first row sets the start.
        const BEHIND: i32 = -12 * 3600;
        let s = week_days(&u, Some(epoch(2026, 10, 15, 0)), now, BEHIND);
        assert_eq!(s[0].date, NaiveDate::from_ymd_opt(2026, 10, 8).unwrap());
        assert!(s[5].today);
        // An old report (its last row days back) does not decide today.
        let old = usage(&[("2026-10-08", Some(3.0)), ("2026-10-09", Some(1.0))], &[]);
        let s = week_days(&old, Some(epoch(2026, 10, 15, 10)), epoch(2026, 10, 13, 15), PT);
        assert!(s[5].today && s[5].used.is_none());
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

    // ---------- the account's report ----------

    use crate::api::{ApiAccountSupport, ApiExtra, ApiLimit, ApiProduct, ApiProducts};

    fn limit(kind: &str, pct: f64, reset: Option<f64>, label: Option<&str>) -> ApiLimit {
        ApiLimit { kind: kind.into(), pct: Some(pct), reset, label: label.map(String::from), ..Default::default() }
    }

    fn usd(minor: f64) -> ApiMoney {
        ApiMoney { minor: Some(minor), currency: Some("USD".into()), exp: Some(2.0) }
    }

    fn plan(week: Option<f64>, five: Option<f64>, now: f64) -> Plan {
        Plan {
            week,
            week_projected: week.map(|w| w * 1.5),
            week_reset: week.map(|_| now + 6.0 * 86400.0 + 20.0 * 3600.0),
            five_hour: five,
            five_hour_reset: five.map(|_| now + 3.0 * 3600.0 + 5.0 * 60.0),
            note: None,
            updated: Some(now - 120.0),
        }
    }

    #[test]
    fn the_sample_report_parses_with_every_part() {
        let u: ApiUsage = serde_json::from_str(
            r#"{"ok":true,"now":1000,"updated":900,
                "account":{"at":880,"tried":880,"error":null,"machine":"studio","plan":"max","available":true,
                  "limits":[{"kind":"session","pct":41,"reset":5000,"label":null,"severity":"normal","active":false},
                            {"kind":"weekly_scoped","pct":18,"reset":9000,"label":"Fable","severity":"normal","active":false}],
                  "extra":{"enabled":true,"used":{"minor":1234,"currency":"USD","exp":2},
                           "limit":{"minor":7500,"currency":"USD","exp":2},"balance":null,"pct":16,
                           "reason":null,"limit_reached":false,"auto_reload":null},
                  "products":{"as_of":870,"start":100,"rows":[{"key":"claude_code","name":"Claude Code","pct":81,"of_limit":50}]}},
                "account_support":{"able":1,"online":1,"machines":3}}"#,
        )
        .unwrap();
        let a = u.account.as_ref().unwrap();
        assert_eq!((a.at, a.plan.as_deref(), a.machine.as_deref()), (Some(880.0), Some("max"), Some("studio")));
        assert_eq!(a.limits.len(), 2);
        assert_eq!(a.limits[1].label.as_deref(), Some("Fable"));
        assert_eq!(a.extra.as_ref().unwrap().used, Some(usd(1234.0)));
        assert_eq!(a.products.as_ref().unwrap().rows[0].of_limit, Some(50.0));
        assert_eq!(u.account_support.unwrap().machines, Some(3.0));
        assert_eq!(u.updated, Some(900.0));
    }

    #[test]
    fn an_older_server_and_odd_shapes_parse_to_nothing_not_an_error() {
        let old: ApiUsage = serde_json::from_str(r#"{"ok":true,"now":1000,"five":{"pct":3,"reset":null}}"#).unwrap();
        assert!(old.account.is_none() && old.account_support.is_none() && old.updated.is_none());
        let odd: ApiUsage = serde_json::from_str(
            r#"{"ok":true,"account":{"at":"soon","limits":"none","extra":"on","products":{"rows":[{"key":7,"pct":"x"}]}},
                "account_support":"yes","updated":"never"}"#,
        )
        .unwrap();
        let a = odd.account.unwrap();
        assert!(a.at.is_none() && a.limits.is_empty() && a.extra.is_none());
        assert!(product_rows(Some(&a)).is_empty());
        assert!(odd.account_support.is_none());
    }

    #[test]
    fn resets_say_the_exact_time_and_how_long() {
        // Now: Wed Nov 18 2026, 1:00am Pacific (standard time, UTC-8).
        const PST: i32 = -8 * 3600;
        let now =
            FixedOffset::east_opt(PST).unwrap().with_ymd_and_hms(2026, 11, 18, 1, 0, 0).unwrap().timestamp() as f64;
        let day = 86400.0;
        assert_eq!(reset_note_at_offset(now + 3.0 * 3600.0 + 5.0 * 60.0, now, PST), "Resets 4:05am, in 3h 5m");
        assert_eq!(reset_note_at_offset(now + 6.0 * day + 20.0 * 3600.0, now, PST), "Resets Tue 9pm, in 6d 20h");
        assert_eq!(reset_note_at_offset(now + 2.0 * day, now, PST), "Resets Fri 1am, in 2d");
        assert_eq!(reset_note_at_offset(now + 3600.0, now, PST), "Resets 2am, in 1h");
        assert_eq!(reset_note_at_offset(now + 42.0 * 60.0 + 10.0, now, PST), "Resets 1:42am, in 42m");
        assert_eq!(reset_note_at_offset(now + 20.0, now, PST), "Resets 1am, in under a minute");
        // The same weekday a week on carries its date, so "Wed" never means two days.
        assert_eq!(reset_note_at_offset(now + 7.0 * day, now, PST), "Resets Nov 25, 1am, in 7d");
        // Under a day away but on tomorrow's date: the weekday says so.
        assert_eq!(reset_note_at_offset(now + 23.5 * 3600.0, now, PST), "Resets Thu 12:30am, in 23h 30m");
    }

    #[test]
    fn a_reset_across_a_clock_change_keeps_its_wall_time_and_real_wait() {
        // Clocks go back on Sun Nov 1 2026 in Los Angeles. Sat Oct 31 9pm PDT to Sat Nov 7 8pm PST is exactly seven
        // days, read on each side in that side's offset: the wait is the real one, the clock time the local one.
        let pdt = FixedOffset::east_opt(-7 * 3600).unwrap();
        let now = pdt.with_ymd_and_hms(2026, 10, 31, 21, 0, 0).unwrap().timestamp() as f64;
        let reset = now + 7.0 * 86400.0;
        assert_eq!(fmt::when_exact_at_offset(reset, now, -8 * 3600), "Nov 7, 8pm");
        assert_eq!(fmt::until(reset - now), "7d");
    }

    #[test]
    fn countdowns_never_go_negative_or_nan() {
        assert_eq!(fmt::until(-500.0), "under a minute");
        assert_eq!(fmt::until(f64::NAN), "under a minute");
        assert_eq!(fmt::until(f64::INFINITY), "under a minute");
        assert_eq!(fmt::until(59.9), "under a minute");
        assert_eq!(fmt::until(60.0), "1m");
        assert_eq!(fmt::until(86400.0 + 3599.0), "1d");
    }

    #[test]
    fn limits_list_the_session_the_week_then_each_model() {
        let now = 1_795_000_000.0;
        let a = ApiAccount {
            at: Some(now - 60.0),
            limits: vec![
                limit("session", 41.0, Some(now + 100.0), None),
                limit("weekly_all", 62.0, Some(now + 500.0), None),
                limit("weekly_scoped", 18.0, Some(now + 86400.0), Some("Fable")),
                limit("weekly_scoped", 250.0, None, None),
                ApiLimit { kind: "weekly_scoped".into(), pct: Some(f64::NAN), ..Default::default() },
            ],
            ..Default::default()
        };
        let rows = limit_rows(&plan(Some(62.0), Some(41.0), now), Some(&a), now);
        let titles: Vec<&str> = rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, ["5-hour session", "Week, all models", "Week, Fable", "Week, one model"]);
        assert_eq!(rows[0].pct, Some(41.0));
        assert!(rows[1].pace.is_some() && rows[0].pace.is_none());
        assert_eq!(rows[3].pct, Some(100.0), "clamped");
        assert_eq!(rows[3].note, "Reset time not reported");
        assert!(rows[2].note.starts_with("Resets "));
    }

    #[test]
    fn limits_fall_back_to_what_chats_saw_without_a_report() {
        let now = 1_795_000_000.0;
        let rows = limit_rows(&plan(Some(7.0), None, now), None, now);
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].title.as_str(), rows[0].pct), ("Week, all models", Some(7.0)));
        assert_eq!(
            missing_limits(&plan(Some(7.0), None, now), None, now),
            Some("5-hour session: not reported on this plan.")
        );
        assert_eq!(missing_limits(&plan(Some(7.0), Some(3.0), now), None, now), None);
        // Nothing known at all is "no numbers yet", not "not reported".
        assert_eq!(missing_limits(&plan(None, None, now), None, now), None);
        assert!(limit_rows(&plan(None, None, now), None, now).is_empty());
    }

    #[test]
    fn a_window_whose_reset_passed_shows_no_number() {
        let now = 1_795_000_000.0;
        let mut p = plan(Some(80.0), Some(90.0), now);
        let later = now + 8.0 * 86400.0;
        let a = ApiAccount {
            limits: vec![limit("weekly_scoped", 50.0, Some(now + 60.0), Some("Fable"))],
            ..Default::default()
        };
        let rows = limit_rows(&p, Some(&a), later);
        assert!(rows.iter().all(|r| r.pct.is_none() && r.pace.is_none() && r.tone == Tone::Ok));
        assert_eq!(rows[1].note, "The week has reset. New numbers come with the next reply.");
        // And the moment it passes while the panel is open: the same rows a second apart.
        p.five_hour_reset = Some(now + 1.0);
        assert!(limit_rows(&p, None, now)[0].pct.is_some());
        assert!(limit_rows(&p, None, now + 1.0)[0].pct.is_none());
    }

    #[test]
    fn money_is_written_from_minor_units_and_the_currencys_decimals() {
        assert_eq!(money(&usd(7725.0)).as_deref(), Some("$77.25"));
        assert_eq!(money(&usd(0.0)).as_deref(), Some("$0.00"));
        assert_eq!(money(&usd(123_456_789.0)).as_deref(), Some("$1,234,567.89"));
        assert_eq!(money(&usd(5.0)).as_deref(), Some("$0.05"));
        assert_eq!(money(&usd(-250.0)).as_deref(), Some("-$2.50"));
        let m = |minor, cur: Option<&str>, exp| ApiMoney { minor: Some(minor), currency: cur.map(String::from), exp };
        assert_eq!(money(&m(120_400.0, Some("eur"), Some(2.0))).as_deref(), Some("€1,204.00"));
        assert_eq!(money(&m(7500.0, Some("JPY"), Some(0.0))).as_deref(), Some("7,500 JPY"));
        assert_eq!(money(&m(7725.0, Some("CAD"), Some(2.0))).as_deref(), Some("77.25 CAD"));
        assert_eq!(money(&m(7725.0, None, None)).as_deref(), Some("77.25"));
        assert_eq!(money(&m(1500.0, Some("KWD"), Some(3.0))).as_deref(), Some("1.500 KWD"));
        assert_eq!(money(&m(f64::NAN, Some("USD"), Some(2.0))), None);
        assert_eq!(money(&ApiMoney::default()), None);
    }

    fn extra(on: bool, used: f64, lim: f64) -> ApiAccount {
        ApiAccount {
            at: Some(1.0),
            extra: Some(ApiExtra {
                enabled: Some(on),
                used: Some(usd(used)),
                limit: Some(usd(lim)),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn extra_usage_says_spent_limit_and_what_is_left() {
        let v = extra_view(Some(&extra(true, 1234.0, 7500.0))).unwrap();
        assert_eq!((v.state, v.on, v.tone), ("On", true, Tone::Ok));
        assert_eq!(
            (v.spent.as_deref(), v.limit.as_deref(), v.left.as_deref()),
            (Some("$12.34"), Some("$75.00"), Some("$62.66"))
        );
        assert_eq!(v.pct.map(|p| p.round()), Some(16.0));
        // Spent past the limit: nothing left, never a negative amount, and the bar is full.
        let v = extra_view(Some(&extra(false, 7725.0, 7500.0))).unwrap();
        assert_eq!(
            (v.state, v.tone, v.left.as_deref(), v.pct),
            ("Limit reached", Tone::Bad, Some("$0.00"), Some(100.0))
        );
        // Off with nothing spent.
        let v = extra_view(Some(&extra(false, 0.0, 7500.0))).unwrap();
        assert_eq!((v.state, v.on, v.pct), ("Off", false, Some(0.0)));
        assert!(v.note.unwrap().starts_with("When it's on"));
        // No limit set: no percent, no "left".
        let mut a = extra(true, 500.0, 0.0);
        a.extra.as_mut().unwrap().limit = None;
        let v = extra_view(Some(&a)).unwrap();
        assert_eq!((v.limit, v.left, v.pct), (None, None, None));
        // Two currencies are never subtracted.
        let mut a = extra(true, 500.0, 7500.0);
        a.extra.as_mut().unwrap().limit.as_mut().unwrap().currency = Some("EUR".into());
        assert_eq!(extra_view(Some(&a)).unwrap().left, None);
        assert!(extra_view(None).is_none() && extra_view(Some(&ApiAccount::default())).is_none());
    }

    #[test]
    fn products_are_biggest_first_and_clamped() {
        let row =
            |n: &str, p: f64| ApiProduct { key: n.into(), name: Some(n.into()), pct: Some(p), of_limit: Some(p / 2.0) };
        let a = ApiAccount {
            products: Some(ApiProducts {
                rows: vec![row("Chats", 12.0), row("Claude Code", 181.0), row("", 5.0), row("Other", -3.0)],
                ..Default::default()
            }),
            ..Default::default()
        };
        let r = product_rows(Some(&a));
        assert_eq!(r.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["Claude Code", "Chats", "Other"]);
        assert_eq!((r[0].share, r[2].share, r[2].of_limit), (100.0, 0.0, Some(0.0)));
    }

    fn top_row(sid: &str, name: &str, weighted: f64) -> crate::api::ApiTopRow {
        crate::api::ApiTopRow {
            machine: "studio".into(),
            sid: sid.into(),
            name: name.into(),
            label: "Studio".into(),
            weighted: Some(weighted),
            share: None,
            alive: Some(true),
            sent: Some(false),
            parts: Some(1.0),
        }
    }

    fn top(rows: Vec<crate::api::ApiTopRow>, total: f64, chats: f64) -> ApiUsage {
        ApiUsage {
            top: Some(crate::api::ApiTop {
                since: Some(1000.0),
                total: Some(total),
                chats: Some(chats),
                rows,
                able: Some(2.0),
                machines: Some(2.0),
                pending: Some(0.0),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn most_used_lists_the_biggest_first_with_their_share() {
        let mut rows = vec![
            top_row("b", "Taxes", 100.0),
            top_row("a", "Oct 12 12:50a Kitchen remodel (fresh 2)", 600.0),
            top_row("c", " ", 3.0),
        ];
        rows[0].parts = Some(3.0);
        rows[0].sent = Some(true);
        rows[2].alive = Some(false);
        rows[2].label = String::new();
        let u = top(rows, 1000.0, 9.0);
        let TopView::Rows { rows, rest, foot } = top_view(Some(&u), TOP_SHOWN) else { panic!("no rows") };
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["Kitchen remodel", "Taxes", "Untitled chat"]
        );
        assert_eq!(rows.iter().map(|r| r.pct.as_str()).collect::<Vec<_>>(), ["60%", "10%", "<1%"]);
        assert_eq!(rows.iter().map(|r| r.place.as_str()).collect::<Vec<_>>(), ["Studio", "Studio, 3 chats", "studio"]);
        assert_eq!(rows[0].key.as_deref(), Some("studio:a"));
        assert_eq!(rows[2].key, None); // no machine lists it now: nothing to open
        assert_eq!((rows[0].sent, rows[1].sent), (false, true));
        assert_eq!((rows[0].share, rows[1].share), (60.0, 10.0));
        // what is not listed is one line, so the shares on show add up to all of it
        assert_eq!(rest, Some(("6 other chats".into(), "30%".into())));
        assert_eq!(foot, "Estimated from each chat's tokens. Chats outside Switchboard aren't counted.");
    }

    #[test]
    fn most_used_shows_five_and_folds_the_rest() {
        let rows: Vec<_> =
            (0..8).map(|i| top_row(&format!("s{i}"), &format!("chat {i}"), 80.0 - i as f64 * 10.0)).collect();
        let u = top(rows, 360.0, 8.0);
        let TopView::Rows { rows, rest, .. } = top_view(Some(&u), TOP_SHOWN) else { panic!("no rows") };
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[4].name, "chat 4");
        assert_eq!(rest, Some(("3 other chats".into(), "17%".into())));
        // everything on show: no line for the rest
        let u = top(vec![top_row("a", "One", 5.0)], 5.0, 1.0);
        let TopView::Rows { rows, rest, .. } = top_view(Some(&u), TOP_SHOWN) else { panic!("no rows") };
        assert_eq!((rows[0].pct.as_str(), rest), ("100%", None));
        let u = top(vec![top_row("a", "One", 5.0)], 6.0, 2.0);
        let TopView::Rows { rest, .. } = top_view(Some(&u), TOP_SHOWN) else { panic!("no rows") };
        assert_eq!(rest, Some(("1 other chat".into(), "17%".into())));
    }

    #[test]
    fn most_used_says_why_it_is_empty_or_stays_away() {
        assert_eq!(top_view(None, 5), TopView::Hidden);
        assert_eq!(top_view(Some(&ApiUsage::default()), 5), TopView::Hidden); // an older server
        let note = |f: &dyn Fn(&mut crate::api::ApiTop)| {
            let mut u = top(Vec::new(), 0.0, 0.0);
            f(u.top.as_mut().unwrap());
            match top_view(Some(&u), 5) {
                TopView::Note(n) => n,
                other => format!("{other:?}"),
            }
        };
        assert_eq!(note(&|_| {}), "No chat has used your plan yet this week.");
        assert!(note(&|t| t.able = Some(0.0)).contains("current Switchboard helper"));
        assert!(note(&|t| t.pending = Some(1.0)).starts_with("Reading this week's chats"));
        assert!(note(&|t| t.since = None).contains("week's reset is known"));
        assert_eq!(note(&|t| t.machines = Some(0.0)), "Hidden");
        // rows that say nothing usable are no rows
        let mut bad = top_row("a", "x", f64::NAN);
        bad.share = Some(40.0);
        let u = top(vec![bad, top_row("", "no id", 5.0), top_row("z", "zero", 0.0)], 100.0, 3.0);
        assert_eq!(top_view(Some(&u), 5), TopView::Note("No chat has used your plan yet this week.".into()));
    }

    #[test]
    fn most_used_names_the_machines_it_cannot_count() {
        let mut u = top(vec![top_row("a", "One", 5.0)], 5.0, 1.0);
        u.top.as_mut().unwrap().machines = Some(3.0);
        let TopView::Rows { foot, .. } = top_view(Some(&u), 5) else { panic!("no rows") };
        assert!(foot.ends_with("1 machine isn't counted until it runs the current Switchboard helper."));
        u.top.as_mut().unwrap().machines = Some(4.0);
        let TopView::Rows { foot, .. } = top_view(Some(&u), 5) else { panic!("no rows") };
        assert!(foot.ends_with("2 machines aren't counted until they run the current Switchboard helper."));
    }

    #[test]
    fn most_used_reads_what_the_server_sends() {
        let u: ApiUsage = serde_json::from_str(
            r#"{"ok": true, "top": {"since": 1000, "total": 1000, "chats": 6, "able": 2, "machines": 2, "pending": 0,
                "rows": [{"machine": "studio", "sid": "a", "name": "Kitchen remodel", "label": "Studio", "weighted": 600,
                          "share": 60.0, "alive": true, "sent": false, "parts": 1},
                         {"machine": "laptop", "sid": "c", "name": null, "label": 7, "weighted": "x", "alive": "yes"},
                         "junk"]}}"#,
        )
        .unwrap();
        let TopView::Rows { rows, rest, .. } = top_view(Some(&u), 5) else { panic!("no rows") };
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].key.as_deref(), rows[0].pct.as_str()), (Some("studio:a"), "60%"));
        assert_eq!(rest, Some(("5 other chats".into(), "40%".into())));
        let old: ApiUsage = serde_json::from_str(r#"{"ok": true, "top": "soon"}"#).unwrap();
        assert_eq!(top_view(Some(&old), 5), TopView::Hidden);
    }

    #[test]
    fn freshness_takes_the_newest_sighting() {
        let now = 10_000.0;
        let mut p = plan(Some(5.0), None, now);
        p.updated = Some(now - 600.0);
        let mut u = ApiUsage { updated: Some(now - 600.0), ..Default::default() };
        assert_eq!(freshness(&p, Some(&u), now).as_deref(), Some("Updated 10 min ago"));
        u.account = Some(ApiAccount { at: Some(now - 30.0), ..Default::default() });
        assert_eq!(freshness(&p, Some(&u), now).as_deref(), Some("Updated just now"));
        p.updated = None;
        assert_eq!(freshness(&p, None, now), None);
        assert_eq!(age_words(3600.0), "1 hr ago");
        assert_eq!(age_words(2.0 * 86400.0), "2 days ago");
        assert_eq!(age_words(86400.0), "1 day ago");
        assert_eq!(age_words(-5.0), "just now");
        assert_eq!(age_words(f64::NAN), "just now");
    }

    fn sup(able: f64, online: f64, machines: f64) -> Option<ApiAccountSupport> {
        Some(ApiAccountSupport { able: Some(able), online: Some(online), machines: Some(machines) })
    }

    #[test]
    fn a_missing_or_old_report_says_why() {
        let now = 100_000.0;
        let note = |u: &ApiUsage| account_note(Some(u), now);
        let mut u = ApiUsage::default();
        assert!(note(&u).unwrap().0.contains("newer Switchboard server"));
        u.account_support = sup(0.0, 0.0, 3.0);
        assert!(note(&u).unwrap().0.contains("current Switchboard helper"));
        u.account_support = sup(1.0, 0.0, 3.0);
        assert!(note(&u).unwrap().0.contains("is online"));
        u.account_support = sup(1.0, 1.0, 3.0);
        assert!(note(&u).unwrap().0.starts_with("Asking Claude Code"));
        u.account = Some(ApiAccount { at: Some(now - 60.0), machine: Some("studio".into()), ..Default::default() });
        assert_eq!(note(&u), None);
        u.account.as_mut().unwrap().error = Some("Claude Code took too long to answer".into());
        let (s, t) = note(&u).unwrap();
        assert_eq!(t, Tone::Warn);
        assert_eq!(
            s,
            "The last refresh on studio failed (Claude Code took too long to answer), so plan details are from 1 min ago."
        );
        u.account = Some(ApiAccount {
            at: None,
            error: Some("not signed in".into()),
            machine: Some("studio".into()),
            ..Default::default()
        });
        assert_eq!(
            note(&u).unwrap().0,
            "Claude Code on studio didn't give its usage: not signed in. Refresh to try again."
        );
        u.account =
            Some(ApiAccount { at: Some(now - 4.0 * 3600.0), machine: Some("studio".into()), ..Default::default() });
        u.account_support = sup(1.0, 1.0, 3.0);
        assert_eq!(note(&u).unwrap().0, "Plan details are from 4 hr ago. Refresh to ask studio again.");
        u.account_support = sup(1.0, 0.0, 3.0);
        assert!(note(&u).unwrap().0.ends_with("no machine that can ask Claude Code is online."));
        u.account = Some(ApiAccount { at: Some(now), available: Some(false), ..Default::default() });
        assert!(note(&u).unwrap().0.contains("isn't signed in with a Claude plan"));
        assert_eq!(account_note(None, now), None);
    }

    #[test]
    fn opening_asks_again_only_when_it_could_help() {
        let now = 100_000.0;
        let mut u = ApiUsage::default();
        assert!(!wants_refresh(None, now, 120.0));
        assert!(!wants_refresh(Some(&u), now, 120.0), "an older server has no refresh");
        u.account_support = sup(1.0, 0.0, 2.0);
        assert!(!wants_refresh(Some(&u), now, 120.0), "nobody online to ask");
        u.account_support = sup(1.0, 1.0, 2.0);
        assert!(wants_refresh(Some(&u), now, 120.0), "no report yet");
        u.account = Some(ApiAccount { at: Some(now - 60.0), tried: Some(now - 60.0), ..Default::default() });
        assert!(!wants_refresh(Some(&u), now, 120.0));
        u.account = Some(ApiAccount { at: Some(now - 900.0), tried: Some(now - 30.0), ..Default::default() });
        assert!(!wants_refresh(Some(&u), now, 120.0), "a try that just failed is not repeated at once");
        u.account = Some(ApiAccount { at: Some(now - 900.0), tried: Some(now - 900.0), ..Default::default() });
        assert!(wants_refresh(Some(&u), now, 120.0));
    }

    #[test]
    fn plan_names_read_as_words() {
        let a = |p: Option<&str>| ApiAccount { plan: p.map(String::from), ..Default::default() };
        assert_eq!(plan_name(Some(&a(Some("max")))).as_deref(), Some("Max plan"));
        assert_eq!(plan_name(Some(&a(Some("team_premium")))).as_deref(), Some("Team premium plan"));
        assert_eq!(plan_name(Some(&a(Some("  ")))), None);
        assert_eq!(plan_name(Some(&a(None))), None);
    }
}
