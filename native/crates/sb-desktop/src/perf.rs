//! Timing for the terminal, on with SB_PERF=1: every two seconds one line on stderr with how many frames were
//! drawn, the time to build and paint them, the time spent parsing output, and the longest gap between frames; and
//! every second that output comes, one line ("sb-perf term:") with the chunks and bytes the terminals took in, the
//! repaints they asked for and the snapshots they got. Off, each call is one relaxed load.

use std::sync::Mutex;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum Kind {
    Build,
    Paint,
    Hub,
}

#[derive(Default)]
struct Acc {
    n: [u32; 3],
    sum: [Duration; 3],
    max: [Duration; 3],
    bytes: usize,
    last_frame: Option<Instant>,
    gap: Duration,
    since: Option<Instant>,
    stall: Duration,
    wheel_at: Option<Instant>,
    wheel_lag: Duration,
}

fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("SB_PERF").is_some_and(|v| v == "1"))
}

static ACC: Mutex<Option<Acc>> = Mutex::new(None);

/// Runs `f`, and with SB_PERF=1 counts how long it took.
pub fn time<R>(kind: Kind, f: impl FnOnce() -> R) -> R {
    if !on() {
        return f();
    }
    let t = Instant::now();
    let r = f();
    note(kind, t.elapsed(), 0);
    r
}

/// The main thread's worst stall: a 20 ms tick on it, and how late it came. Started once, with SB_PERF=1.
pub fn watch(cx: &mut gpui::App) {
    if !on() {
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            let t = Instant::now();
            cx.background_executor().timer(Duration::from_millis(20)).await;
            let late = t.elapsed().saturating_sub(Duration::from_millis(20));
            if let Ok(mut g) = ACC.lock() {
                let a = g.get_or_insert_with(Acc::default);
                a.stall = a.stall.max(late);
            }
        }
    })
    .detach();
}

/// A wheel event: the next frame says how long it took to show.
pub fn wheel() {
    if on()
        && let Ok(mut g) = ACC.lock()
    {
        let a = g.get_or_insert_with(Acc::default);
        a.wheel_at.get_or_insert(Instant::now());
    }
}

pub fn bytes(n: usize) {
    if on() {
        note(Kind::Hub, Duration::ZERO, n);
    }
}

/// What the terminals took in and asked to have drawn, for the second line: one a second while output comes.
/// A chat on a Windows machine sends many small chunks; this says how many, and what they cost here.
#[derive(Default)]
pub struct TermStat {
    /// Pieces of output from the hub, and their bytes.
    pub chunks: u32,
    pub bytes: usize,
    /// Parse slices run.
    pub slices: u32,
    /// Repaints asked for, and slices that shared one already on its way.
    pub notifies: u32,
    pub shared: u32,
    /// Screens sent whole (a reconnect, a redraw, a viewer that fell behind), and the time to clear the grid for them.
    pub snapshots: u32,
    pub rebuild: Duration,
    since: Option<Instant>,
}

static TERM: Mutex<Option<TermStat>> = Mutex::new(None);

/// Counts into the terminals' line with SB_PERF=1.
pub fn term(f: impl FnOnce(&mut TermStat)) {
    if !on() {
        return;
    }
    let Ok(mut g) = TERM.lock() else { return };
    let t = g.get_or_insert_with(TermStat::default);
    f(t);
    let now = Instant::now();
    let since = *t.since.get_or_insert(now);
    if now - since >= Duration::from_secs(1) {
        eprintln!("sb-perf term: {}", term_line(t, now - since));
        *t = TermStat::default();
    }
}

fn term_line(t: &TermStat, over: Duration) -> String {
    format!(
        "{} chunks ({:.1} KB) in {:.1}s, {} parse slices, {} repaints asked ({} shared one), {} snapshots, grid cleared in {:.2} ms",
        t.chunks,
        t.bytes as f64 / 1024.,
        over.as_secs_f64(),
        t.slices,
        t.notifies,
        t.shared,
        t.snapshots,
        t.rebuild.as_secs_f64() * 1000.
    )
}

fn note(kind: Kind, d: Duration, bytes: usize) {
    let Ok(mut g) = ACC.lock() else { return };
    let a = g.get_or_insert_with(Acc::default);
    let now = Instant::now();
    let i = kind as usize;
    if bytes > 0 {
        a.bytes += bytes;
    } else {
        a.n[i] += 1;
        a.sum[i] += d;
        a.max[i] = a.max[i].max(d);
    }
    if matches!(kind, Kind::Build) {
        if let Some(l) = a.last_frame {
            a.gap = a.gap.max(now - l);
        }
        if let Some(w) = a.wheel_at.take() {
            a.wheel_lag = a.wheel_lag.max(now - w);
        }
        a.last_frame = Some(now);
    }
    let since = *a.since.get_or_insert(now);
    if now - since >= Duration::from_secs(2) {
        let ms = |d: Duration| d.as_secs_f64() * 1000.;
        let avg = |k: usize| if a.n[k] > 0 { ms(a.sum[k]) / a.n[k] as f64 } else { 0. };
        eprintln!(
            "sb-perf: {} frames/2s, build avg {:.2} max {:.2} ms, paint avg {:.2} max {:.2} ms, parse {}x avg {:.2} max {:.2} ms ({} KB), longest frame gap {:.1} ms, main thread stall {:.1} ms, wheel to frame {:.1} ms",
            a.n[0],
            avg(0),
            ms(a.max[0]),
            avg(1),
            ms(a.max[1]),
            a.n[2],
            avg(2),
            ms(a.max[2]),
            a.bytes / 1024,
            ms(a.gap),
            ms(a.stall),
            ms(a.wheel_lag)
        );
        *a = Acc { last_frame: a.last_frame, since: Some(now), ..Acc::default() };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminal_line_says_what_a_flood_was_made_of() {
        let t = TermStat {
            chunks: 412,
            bytes: 39_000,
            slices: 96,
            notifies: 58,
            shared: 38,
            snapshots: 1,
            rebuild: Duration::from_micros(210),
            since: None,
        };
        assert_eq!(
            term_line(&t, Duration::from_millis(1004)),
            "412 chunks (38.1 KB) in 1.0s, 96 parse slices, 58 repaints asked (38 shared one), 1 snapshots, grid cleared in 0.21 ms"
        );
    }
}
