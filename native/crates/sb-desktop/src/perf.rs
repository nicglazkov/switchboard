//! Timing for the terminal, on with SB_PERF=1: every two seconds one line on stderr with how many frames were
//! drawn, the time to build and paint them, the time spent parsing output, and the longest gap between frames.
//! Off, each call is one relaxed load.

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
