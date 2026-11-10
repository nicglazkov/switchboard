//! A watchdog for a stuck main thread. The window sometimes froze until the app was quit, with nothing in the log to
//! say where. The main thread beats once a second; a thread of its own watches the beat, and when it stops for
//! `HANG` it logs it and, on macOS, samples the app into the log folder (hang-<time>.txt, the newest few kept), so
//! the next freeze comes with the main thread's stack. When the beat comes back it logs how long the stall was.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A main thread silent this long is stuck: far past any frame, and past timer slack (App Nap delays timers seconds).
pub const HANG: Duration = Duration::from_secs(8);
/// At most one sample per this long: a long freeze is captured once.
const EVERY: Duration = Duration::from_secs(600);
/// Samples kept in the log folder.
const KEEP: usize = 5;

static BEAT: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// What the watchdog does on one look.
#[derive(Debug, PartialEq)]
pub enum Step {
    Nothing,
    /// Stuck: log it, and sample when `sample` (not one too recently).
    Stuck {
        sample: bool,
    },
    /// The beat is back after a stall.
    Back,
}

/// The watchdog's memory between looks.
#[derive(Default)]
pub struct Watch {
    stuck_since_beat: Option<u64>,
    last_sample: Option<Instant>,
}

impl Watch {
    /// One look: the last beat `beat_age` ago (beat stamp `beat`), at `now`. `slept`: the watchdog itself was paused
    /// (the computer slept), so a stale beat says nothing.
    pub fn look(&mut self, beat: u64, beat_age: Duration, slept: bool, now: Instant) -> Step {
        if slept {
            self.stuck_since_beat = None;
            return Step::Nothing;
        }
        match self.stuck_since_beat {
            Some(b) if beat != b => {
                self.stuck_since_beat = None;
                Step::Back
            }
            Some(_) => Step::Nothing,
            None if beat_age >= HANG => {
                self.stuck_since_beat = Some(beat);
                let sample = self.last_sample.is_none_or(|t| now.duration_since(t) >= EVERY);
                if sample {
                    self.last_sample = Some(now);
                }
                Step::Stuck { sample }
            }
            None => Step::Nothing,
        }
    }
}

/// Starts the beat on the main thread and the watchdog beside it.
pub fn start(cx: &mut gpui::App) {
    BEAT.store(now_ms(), Ordering::Relaxed);
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            BEAT.store(now_ms(), Ordering::Relaxed);
        }
    })
    .detach();
    let spawned = std::thread::Builder::new().name("sb-hang".into()).spawn(|| {
        let mut w = Watch::default();
        let mut last = Instant::now();
        let mut stuck_at = 0u64;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let now = Instant::now();
            let slept = now.duration_since(last) > Duration::from_secs(4);
            last = now;
            let beat = BEAT.load(Ordering::Relaxed);
            let age = Duration::from_millis(now_ms().saturating_sub(beat));
            match w.look(beat, age, slept, now) {
                Step::Nothing => {}
                Step::Stuck { sample } => {
                    stuck_at = beat;
                    log::warn!("main thread stuck for {}s", age.as_secs());
                    if sample {
                        capture();
                    }
                }
                Step::Back => {
                    let secs = beat.saturating_sub(stuck_at) as f64 / 1000.;
                    log::warn!("main thread back after a {secs:.1}s stall");
                }
            }
        }
    });
    if let Err(e) = spawned {
        log::warn!("no hang watchdog: {e}");
    }
}

/// Samples this process for three seconds into the log folder (macOS), keeping the newest `KEEP`.
fn capture() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let Some(dir) = crate::logs::dir() else { return };
    let stamp = chrono_stamp();
    let path = dir.join(format!("hang-{stamp}.txt"));
    let ok = std::process::Command::new("/usr/bin/sample")
        .arg(std::process::id().to_string())
        .arg("3")
        .arg("-file")
        .arg(&path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        log::warn!("sampled the stuck app to {}", path.display());
    } else {
        log::warn!("could not sample the stuck app");
    }
    prune(&dir);
}

fn prune(dir: &std::path::Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut hangs: Vec<_> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("hang-") && n.ends_with(".txt"))
        })
        .collect();
    hangs.sort();
    let extra = hangs.len().saturating_sub(KEEP);
    for p in hangs.into_iter().take(extra) {
        let _ = std::fs::remove_file(p);
    }
}

/// "20261109-231502" in UTC, sortable.
fn chrono_stamp() -> String {
    let s = now_ms() / 1000;
    let (days, rem) = (s / 86400, s % 86400);
    let (y, m, d) = civil(days as i64);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Days since 1970-01-01 to (year, month, day) (Howard Hinnant's civil_from_days).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stall_past_the_limit_samples_once_and_logs_when_it_ends() {
        let mut w = Watch::default();
        let t0 = Instant::now();
        assert_eq!(w.look(1, Duration::from_secs(2), false, t0), Step::Nothing);
        assert_eq!(w.look(1, HANG, false, t0), Step::Stuck { sample: true });
        assert_eq!(w.look(1, HANG + Duration::from_secs(5), false, t0), Step::Nothing, "one stall, one capture");
        assert!(w.look(2, Duration::ZERO, false, t0) == Step::Back);
        // Another stall a minute later is logged but not sampled again.
        let t1 = t0 + Duration::from_secs(60);
        assert_eq!(w.look(2, HANG, false, t1), Step::Stuck { sample: false });
        assert!(w.look(3, Duration::ZERO, false, t1) == Step::Back);
        let t2 = t0 + EVERY;
        assert_eq!(w.look(3, HANG, false, t2), Step::Stuck { sample: true });
    }

    #[test]
    fn a_stale_beat_after_the_computer_slept_is_not_a_hang() {
        let mut w = Watch::default();
        assert_eq!(w.look(1, Duration::from_secs(3600), true, Instant::now()), Step::Nothing);
    }

    #[test]
    fn stamps_are_utc_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20766), (2026, 11, 9));
    }
}
