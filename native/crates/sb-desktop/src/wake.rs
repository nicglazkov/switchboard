//! A window that comes back (uncovered, unminimized, a display waking) gets its frames going again, and so does one
//! whose frames stop while it is in front.
//!
//! GPUI's Mac window restarts its display link when it turns visible, but skips that while AppKit briefly reports no
//! screen (a display waking or reconfiguring). The window then draws nothing until something restarts the link: a
//! resize does, through the view's displayLayer:. So on coming back we ask for that same displayLayer: a few times,
//! as a resize would, until the screen is there again.
//!
//! The active window has also stopped drawing for 7 to 42 seconds with the main thread alive and no visibility
//! change to hang a nudge on. Nothing drawn can notice that, so a timer does (`FrameWatch`): it keeps one frame
//! callback asked for, and when the window is in front and on screen and the callback has not come for a second, it
//! nudges the same way. Past three seconds it logs the window's state and samples the app once, so the next freeze
//! says what stopped.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use gpui::{App, Subscription, Window};

/// After the first nudge, again at these delays: the screen can take a few seconds to come back after a wake.
const AGAIN: [u64; 3] = [800, 2500, 6000];

/// How often the frame watchdog looks.
const TICK: Duration = Duration::from_millis(500);
/// How often it looks while the window is not in front or not on screen: there is nothing to judge then, only
/// the window's return to notice.
const IDLE_TICK: Duration = Duration::from_secs(2);

/// The wait before the next look.
fn next_look(watching: bool) -> Duration {
    if watching { TICK } else { IDLE_TICK }
}

/// A look this long after the one before says the main thread was busy or the computer slept: no frame could have
/// come either, so nothing is judged.
const TICK_LATE: Duration = Duration::from_secs(1);
/// A frame asked for and not come for this long, in the active window on screen: nudge it.
pub const KICK_AFTER: Duration = Duration::from_secs(1);
/// Frames stopped for this long: say so in the log, with the window's state.
pub const REPORT_AFTER: Duration = Duration::from_secs(3);
/// After this many nudges in one stall they come only every `SLOW_KICK`: a nudge draws a frame, so the window keeps
/// showing what changed, but nothing here can spin.
const QUICK_KICKS: u32 = 10;
const SLOW_KICK: Duration = Duration::from_secs(5);
/// At most one sample per this long.
const SAMPLE_EVERY: Duration = Duration::from_secs(600);

/// What the frame watchdog does on one look.
#[derive(Debug, Default, PartialEq)]
pub struct Look {
    /// Ask the window for a frame callback.
    pub arm: bool,
    /// Nudge the window: its frames have stopped.
    pub kick: bool,
    /// Frames have been stopped for `REPORT_AFTER`: log it (once per stall), and sample when `sample`.
    pub report: Option<Report>,
    /// Frames come by themselves again, after this long and this many nudges.
    pub back: Option<(Duration, u32)>,
}

#[derive(Debug, PartialEq)]
pub struct Report {
    pub stalled: Duration,
    pub kicks: u32,
    pub sample: bool,
}

/// The frame callback asked for and not seen yet.
struct Asked {
    /// Since when it counts as waited for (None: nothing to judge it by just now).
    since: Option<Instant>,
    /// It was nudged, so when it comes it says nothing about frames coming by themselves.
    kicked: bool,
}

struct Stall {
    since: Instant,
    kicks: u32,
    reported: bool,
}

/// The frame watchdog's memory between looks.
#[derive(Default)]
pub struct FrameWatch {
    last_look: Option<Instant>,
    served: u64,
    asked: Option<Asked>,
    stall: Option<Stall>,
    last_sample: Option<Instant>,
}

impl FrameWatch {
    /// One look, at `now`. `served` counts the frame callbacks that have run; `watching`: the window is the active
    /// one and on screen, so its frames should be coming.
    pub fn look(&mut self, served: u64, watching: bool, now: Instant) -> Look {
        let late = self.last_look.is_some_and(|t| now.saturating_duration_since(t) > TICK_LATE);
        self.last_look = Some(now);
        let mut out = Look::default();
        if served != self.served {
            self.served = served;
            // one that came without a nudge: frames flow
            if self.asked.take().is_some_and(|a| !a.kicked)
                && let Some(st) = self.stall.take()
            {
                out.back = Some((now.saturating_duration_since(st.since), st.kicks));
            }
        }
        if !watching || late {
            if let Some(a) = &mut self.asked {
                a.since = None;
            }
            self.stall = None;
            return out;
        }
        match &mut self.asked {
            None => {
                self.asked = Some(Asked { since: Some(now), kicked: false });
                out.arm = true;
            }
            Some(a) => {
                let since = *a.since.get_or_insert(now);
                let kicks = self.stall.as_ref().map_or(0, |s| s.kicks);
                let wait = if kicks < QUICK_KICKS { KICK_AFTER } else { SLOW_KICK };
                if now.saturating_duration_since(since) >= wait {
                    a.kicked = true;
                    a.since = Some(now);
                    self.stall.get_or_insert(Stall { since, kicks: 0, reported: false }).kicks += 1;
                    out.kick = true;
                }
            }
        }
        if let Some(st) = &mut self.stall {
            let stalled = now.saturating_duration_since(st.since);
            if !st.reported && stalled >= REPORT_AFTER {
                st.reported = true;
                let sample = self.last_sample.is_none_or(|t| now.saturating_duration_since(t) >= SAMPLE_EVERY);
                if sample {
                    self.last_sample = Some(now);
                }
                out.report = Some(Report { stalled, kicks: st.kicks, sample });
            }
        }
        out
    }
}

// When the board last asked for a frame and last drew one, in milliseconds since the app started (0: never): the
// report says both, so a stall shows whether content was waiting.
static BOARD_WANTS: AtomicU64 = AtomicU64::new(0);
static BOARD_DREW: AtomicU64 = AtomicU64::new(0);

fn since_start() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// The board changed and wants a frame.
pub fn board_wants() {
    BOARD_WANTS.store(since_start(), Ordering::Relaxed);
}

/// The board is drawing a frame.
pub fn board_drew() {
    BOARD_DREW.store(since_start(), Ordering::Relaxed);
}

fn board_state() -> String {
    let now = since_start();
    let ago = |at: u64| match at {
        0 => "never".to_string(),
        at => format!("{:.1}s ago", now.saturating_sub(at) as f64 / 1000.),
    };
    let (wants, drew) = (BOARD_WANTS.load(Ordering::Relaxed), BOARD_DREW.load(Ordering::Relaxed));
    let waiting = if wants > drew { "a change is waiting to be drawn since" } else { "nothing waits; it last changed" };
    format!("the board: {waiting} {}, last drew {}", ago(wants), ago(drew))
}

/// Keep this for the life of the window's root view.
pub fn watch(window: &Window) -> Subscription {
    // Only the Mac window has stalled this way, and only there is the nudge known to restart frames.
    if cfg!(target_os = "macos") {
        window.on_next_frame(frames);
    }
    window.observe_window_visibility(|v, window, cx| {
        if v.is_visible() {
            soon(window, cx);
        }
    })
}

/// The frame watchdog for one window, until it closes. It runs off a timer, never from a draw. Asking for a frame
/// callback is itself a call for a frame (GPUI wakes the display link for it), so an idle window in front answers
/// each one; a callback that does not come is a window that has stopped drawing, not one with nothing to draw.
fn frames(window: &mut Window, cx: &mut App) {
    window
        .spawn(cx, async move |cx| {
            let served = Rc::new(Cell::new(0u64));
            let mut watch = FrameWatch::default();
            let mut wait = TICK;
            loop {
                cx.background_executor().timer(wait).await;
                let looked = cx.update(|window, _| {
                    let watching = window.is_window_active() && on_screen(window);
                    let look = watch.look(served.get(), watching, Instant::now());
                    if look.arm {
                        let served = served.clone();
                        window.on_next_frame(move |_, _| served.set(served.get() + 1));
                    }
                    if look.kick {
                        nudge(window);
                    }
                    if let Some(r) = &look.report {
                        log::warn!(
                            "window: no frames for {:.0}s though it is the active window and the main thread is alive ({} nudges); {}; {}",
                            r.stalled.as_secs_f32(),
                            r.kicks,
                            describe(window),
                            board_state()
                        );
                        if r.sample {
                            // on a thread of its own: sampling takes seconds, and the main thread should show as it is
                            let _ = std::thread::Builder::new()
                                .name("sb-frames".into())
                                .spawn(|| crate::hang::capture("frames"));
                        }
                    }
                    if let Some((after, kicks)) = look.back {
                        log::warn!(
                            "window: frames stopped for {:.1}s and come again ({kicks} nudges)",
                            after.as_secs_f32()
                        );
                    }
                    watching
                });
                match looked {
                    Ok(watching) => wait = next_look(watching),
                    Err(_) => break,
                }
            }
        })
        .detach();
}

/// Nudges now and a few more times over the next seconds.
pub fn soon(window: &mut Window, cx: &mut App) {
    nudge(window);
    window
        .spawn(cx, async move |cx| {
            let mut slept = 0;
            for at in AGAIN {
                cx.background_executor().timer(Duration::from_millis(at - slept)).await;
                slept = at;
                if cx.update(|window, _| nudge(window)).is_err() {
                    break;
                }
            }
        })
        .detach();
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // objc 0.2's msg_send! checks a cfg newer rustc does not know
mod mac {
    use gpui::Window;
    use objc::runtime::{BOOL, NO, Object, YES};
    use objc::{class, msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    /// NSWindowOcclusionStateVisible.
    const OCCLUSION_VISIBLE: usize = 1 << 1;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGMainDisplayID() -> u32;
        fn CGDisplayIsAsleep(display: u32) -> u32;
    }

    fn view(window: &Window) -> Option<*mut Object> {
        // the trait's handle (GPUI's own method of this name is its window id)
        let h = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(h) = h.as_raw() else { return None };
        Some(h.ns_view.as_ptr() as *mut Object)
    }

    pub fn nudge(window: &mut Window) {
        window.refresh();
        let Some(view) = view(window) else { return };
        // SAFETY: GPUI's own NSView, alive while the window is; messages to it on the main thread, where we are.
        unsafe {
            let _: () = msg_send![view, setNeedsDisplay: YES];
            let layer: *mut Object = msg_send![view, layer];
            if !layer.is_null() {
                let _: () = msg_send![layer, setNeedsDisplay];
            }
        }
    }

    /// AppKit shows the window (not covered, not on another Space, not minimized) and the display is awake.
    pub fn on_screen(window: &Window) -> bool {
        let Some(view) = view(window) else { return false };
        // SAFETY: as in `nudge`; the view's window is nil-checked, and the display calls take no pointers.
        unsafe {
            let win: *mut Object = msg_send![view, window];
            if win.is_null() {
                return false;
            }
            let occlusion: usize = msg_send![win, occlusionState];
            occlusion & OCCLUSION_VISIBLE != 0 && CGDisplayIsAsleep(CGMainDisplayID()) == 0
        }
    }

    /// What AppKit says about the window, for the log.
    pub fn describe(window: &Window) -> String {
        let gpui = format!("gpui: active {}, visible {}", window.is_window_active(), window.visibility().is_visible());
        let Some(view) = view(window) else { return format!("{gpui}; no view") };
        // SAFETY: as in `nudge`; every object is nil-checked before it is messaged for anything but a count.
        unsafe {
            let win: *mut Object = msg_send![view, window];
            if win.is_null() {
                return format!("{gpui}; the view has no window");
            }
            let yes = |b: BOOL| b != NO;
            let occlusion: usize = msg_send![win, occlusionState];
            let visible: BOOL = msg_send![win, isVisible];
            let key: BOOL = msg_send![win, isKeyWindow];
            let mini: BOOL = msg_send![win, isMiniaturized];
            let screen: *mut Object = msg_send![win, screen];
            let screens: *mut Object = msg_send![class!(NSScreen), screens];
            let count: usize = if screens.is_null() { 0 } else { msg_send![screens, count] };
            let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
            let app_active: BOOL = if app.is_null() { NO } else { msg_send![app, isActive] };
            format!(
                "{gpui}; AppKit: occlusion {occlusion:#x} (visible {}), on screen {}, key {}, minimized {}, has a screen {} of {count}, app active {}, display asleep {}",
                occlusion & OCCLUSION_VISIBLE != 0,
                yes(visible),
                yes(key),
                yes(mini),
                !screen.is_null(),
                yes(app_active),
                CGDisplayIsAsleep(CGMainDisplayID()) != 0
            )
        }
    }
}

#[cfg(target_os = "macos")]
use mac::{describe, nudge, on_screen};

#[cfg(not(target_os = "macos"))]
fn nudge(window: &mut Window) {
    window.refresh();
}

// The watchdog only runs on a Mac; these keep `frames` compiling elsewhere.
#[cfg(not(target_os = "macos"))]
fn on_screen(_: &Window) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
fn describe(_: &Window) -> String {
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn frames_that_come_are_asked_for_again_and_nothing_else() {
        let mut w = FrameWatch::default();
        let t0 = Instant::now();
        for i in 0..20 {
            // each one asked for has come by the next look
            let look = w.look(u64::from(i), true, t0 + TICK * i);
            assert_eq!(look, Look { arm: true, ..Look::default() }, "look {i}");
        }
    }

    #[test]
    fn a_frame_that_never_comes_is_nudged_after_a_second_and_reported_once() {
        let mut w = FrameWatch::default();
        let t0 = Instant::now();
        assert!(w.look(0, true, t0).arm);
        assert_eq!(w.look(0, true, t0 + ms(500)), Look::default(), "half a second is no stall");
        assert_eq!(w.look(0, true, t0 + ms(1000)), Look { kick: true, ..Look::default() });
        assert_eq!(w.look(0, true, t0 + ms(1500)), Look::default(), "one nudge a second");
        assert_eq!(w.look(0, true, t0 + ms(2000)), Look { kick: true, ..Look::default() });
        assert_eq!(w.look(0, true, t0 + ms(2500)), Look::default());
        let look = w.look(0, true, t0 + ms(3000));
        assert_eq!(
            look,
            Look { kick: true, report: Some(Report { stalled: ms(3000), kicks: 3, sample: true }), ..Look::default() }
        );
        // a long stall: reported once, nudged at a bounded rate
        let (mut kicks, mut reports) = (3, 0);
        for i in 7..=240 {
            let look = w.look(0, true, t0 + TICK * i);
            kicks += u32::from(look.kick);
            reports += u32::from(look.report.is_some());
            assert!(!look.arm && look.back.is_none());
        }
        assert_eq!(reports, 0, "one report per stall");
        assert_eq!(kicks, QUICK_KICKS + 22, "every second ten times, then every five seconds for 110 s");
    }

    #[test]
    fn a_nudged_frame_keeps_the_stall_open_until_one_comes_by_itself() {
        let mut w = FrameWatch::default();
        let t0 = Instant::now();
        assert!(w.look(0, true, t0).arm);
        w.look(0, true, t0 + ms(500));
        assert!(w.look(0, true, t0 + ms(1000)).kick);
        // the nudge drew a frame, which ran the callback: asked again, and the stall is not over
        assert_eq!(w.look(1, true, t0 + ms(1500)), Look { arm: true, ..Look::default() });
        w.look(1, true, t0 + ms(2000));
        assert!(w.look(1, true, t0 + ms(2500)).kick, "still no frame by itself");
        let look = w.look(2, true, t0 + ms(3000));
        assert!(look.arm && look.report.is_some_and(|r| r.kicks == 2 && r.sample), "three seconds in");
        // this one comes without a nudge: the link runs again
        assert_eq!(w.look(3, true, t0 + ms(3500)), Look { arm: true, back: Some((ms(3500), 2)), ..Look::default() });
        assert_eq!(w.look(4, true, t0 + ms(4000)), Look { arm: true, ..Look::default() });
    }

    #[test]
    fn a_window_that_is_not_in_front_or_not_on_screen_is_left_alone() {
        let mut w = FrameWatch::default();
        let t0 = Instant::now();
        for i in 0..40 {
            assert_eq!(w.look(0, false, t0 + TICK * i), Look::default());
        }
        // in front again: the wait starts now, not when it left
        let t1 = t0 + TICK * 40;
        assert!(w.look(0, true, t1).arm);
        assert_eq!(w.look(0, true, t1 + ms(500)), Look::default());
        assert!(w.look(0, true, t1 + ms(1000)).kick);
        // hidden in the middle of a stall: dropped, and no word when frames return
        assert_eq!(w.look(0, false, t1 + ms(1500)), Look::default());
        assert_eq!(w.look(1, true, t1 + ms(2000)), Look { arm: true, ..Look::default() });
    }

    #[test]
    fn a_window_behind_is_looked_at_less_often_and_watched_again_when_it_returns() {
        assert_eq!(next_look(true), TICK);
        assert_eq!(next_look(false), IDLE_TICK);
        let mut w = FrameWatch::default();
        let t0 = Instant::now();
        for i in 0..10 {
            assert_eq!(w.look(0, false, t0 + IDLE_TICK * i), Look::default());
        }
        // in front again, seen at the next slow look: that look judges nothing (it came late), the next one asks
        let t1 = t0 + IDLE_TICK * 10;
        assert_eq!(w.look(0, true, t1), Look::default());
        assert!(w.look(0, true, t1 + TICK).arm);
        assert_eq!(w.look(1, true, t1 + TICK * 2), Look { arm: true, ..Look::default() }, "frames come: no nudge");
    }

    #[test]
    fn a_late_look_says_the_main_thread_was_busy_and_judges_nothing() {
        let mut w = FrameWatch::default();
        let t0 = Instant::now();
        assert!(w.look(0, true, t0).arm);
        // the main thread was stuck for ten seconds (or the computer slept): no frame could come
        let t1 = t0 + Duration::from_secs(10);
        assert_eq!(w.look(0, true, t1), Look::default());
        assert_eq!(w.look(0, true, t1 + ms(500)), Look::default(), "the wait starts over");
        assert!(w.look(0, true, t1 + ms(1500)).kick);
    }

    #[test]
    fn a_second_stall_soon_after_is_reported_but_not_sampled() {
        let mut w = FrameWatch::default();
        let t0 = Instant::now();
        let stall = |w: &mut FrameWatch, from: Instant, served: u64| {
            let mut report = None;
            for i in 0..=8 {
                if let Some(r) = w.look(served, true, from + TICK * i).report {
                    report = Some(r.sample);
                }
            }
            // the nudged frame, then one by itself, which ends it
            w.look(served + 1, true, from + TICK * 9);
            assert!(w.look(served + 2, true, from + TICK * 10).back.is_some());
            report
        };
        assert_eq!(stall(&mut w, t0, 0), Some(true));
        assert_eq!(stall(&mut w, t0 + Duration::from_secs(60), 2), Some(false));
        assert_eq!(stall(&mut w, t0 + SAMPLE_EVERY + Duration::from_secs(60), 4), Some(true));
    }
}
