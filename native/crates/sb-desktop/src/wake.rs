//! A window that comes back (uncovered, unminimized, a display waking) gets its frames going again.
//!
//! GPUI's Mac window restarts its display link when it turns visible, but skips that while AppKit briefly reports no
//! screen (a display waking or reconfiguring). The window then draws nothing until something restarts the link: a
//! resize does, through the view's displayLayer:. So on coming back we ask for that same displayLayer: a few times,
//! as a resize would, until the screen is there again.

use std::time::Duration;

use gpui::{App, Subscription, Window};

/// After the first nudge, again at these delays: the screen can take a few seconds to come back after a wake.
const AGAIN: [u64; 3] = [800, 2500, 6000];

/// Keep this for the life of the window's root view.
pub fn watch(window: &Window) -> Subscription {
    window.observe_window_visibility(|v, window, cx| {
        if v.is_visible() {
            soon(window, cx);
        }
    })
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
fn nudge(window: &mut Window) {
    use objc::runtime::{Object, YES};
    use objc::{msg_send, sel, sel_impl};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    window.refresh();
    let Ok(h) = window.window_handle() else { return };
    let RawWindowHandle::AppKit(h) = h.as_raw() else { return };
    let view = h.ns_view.as_ptr() as *mut Object;
    // SAFETY: GPUI's own NSView, alive while the window is; messages to it on the main thread, where we are.
    unsafe {
        let _: () = msg_send![view, setNeedsDisplay: YES];
        let layer: *mut Object = msg_send![view, layer];
        if !layer.is_null() {
            let _: () = msg_send![layer, setNeedsDisplay];
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn nudge(window: &mut Window) {
    window.refresh();
}
