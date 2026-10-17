//! Linux window frames. When the compositor draws no frame of its own (GNOME on Wayland, for one), every window draws
//! it: the board's header already holds the window buttons (chrome.rs), the other windows get a slim title bar here,
//! and every window gets edges to resize it by. A Mac or Windows window never draws any of this.

use gpui::{
    AnyElement, Decorations, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, ResizeEdge,
    Styled, Tiling, Window, div, prelude::*, px,
};

use crate::theme::Theme;
use crate::ui;

/// The title bar's height in the windows that are not the board.
pub const BAR: f32 = 36.;

/// The window draws its own frame (Linux client-side decorations): which of its sides are tiled, if it does.
/// `shots` builds pretend so with SB_AS_CSD=1, since Xvfb has no compositor to ask for it.
pub fn client(window: &Window) -> Option<Tiling> {
    #[cfg(feature = "shots")]
    if std::env::var_os("SB_AS_CSD").is_some() {
        return Some(Tiling::default());
    }
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return None;
    }
    match window.window_decorations() {
        Decorations::Client { tiling } => Some(tiling),
        Decorations::Server => None,
    }
}

/// The edges and corners a window is resized by, inside its border, skipping the sides tiled against the screen.
pub fn edges(tiling: Tiling) -> Vec<AnyElement> {
    const E: f32 = 5.;
    const C: f32 = 12.;
    use gpui::CursorStyle as Cur;
    let handle = |id: &'static str, edge: ResizeEdge, cursor: Cur| {
        div().id(id).absolute().occlude().cursor(cursor).on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            window.start_window_resize(edge);
        })
    };
    let (t, l, r, b) = (!tiling.top, !tiling.left, !tiling.right, !tiling.bottom);
    let mut out = Vec::new();
    if t {
        out.push(
            handle("rz-t", ResizeEdge::Top, Cur::ResizeUpDown)
                .top_0()
                .left(px(C))
                .right(px(C))
                .h(px(E))
                .into_any_element(),
        );
    }
    if b {
        out.push(
            handle("rz-b", ResizeEdge::Bottom, Cur::ResizeUpDown)
                .bottom_0()
                .left(px(C))
                .right(px(C))
                .h(px(E))
                .into_any_element(),
        );
    }
    if l {
        out.push(
            handle("rz-l", ResizeEdge::Left, Cur::ResizeLeftRight)
                .left_0()
                .top(px(C))
                .bottom(px(C))
                .w(px(E))
                .into_any_element(),
        );
    }
    if r {
        out.push(
            handle("rz-r", ResizeEdge::Right, Cur::ResizeLeftRight)
                .right_0()
                .top(px(C))
                .bottom(px(C))
                .w(px(E))
                .into_any_element(),
        );
    }
    if t && l {
        out.push(
            handle("rz-tl", ResizeEdge::TopLeft, Cur::ResizeUpLeftDownRight)
                .top_0()
                .left_0()
                .size(px(C))
                .into_any_element(),
        );
    }
    if t && r {
        out.push(
            handle("rz-tr", ResizeEdge::TopRight, Cur::ResizeUpRightDownLeft)
                .top_0()
                .right_0()
                .size(px(C))
                .into_any_element(),
        );
    }
    if b && l {
        out.push(
            handle("rz-bl", ResizeEdge::BottomLeft, Cur::ResizeUpRightDownLeft)
                .bottom_0()
                .left_0()
                .size(px(C))
                .into_any_element(),
        );
    }
    if b && r {
        out.push(
            handle("rz-br", ResizeEdge::BottomRight, Cur::ResizeUpLeftDownRight)
                .bottom_0()
                .right_0()
                .size(px(C))
                .into_any_element(),
        );
    }
    out
}

/// Minimize, maximize (or restore) and close, for a window that draws its own frame.
pub fn buttons(t: &Theme, maxed: bool) -> impl IntoElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(2.))
        .child(ui::icon_btn(t, "w-min", "min", 28.).on_click(|_, window, _| window.minimize_window()))
        .child(
            ui::icon_btn(t, "w-max", if maxed { "restore" } else { "max" }, 28.)
                .on_click(|_, window, _| window.zoom_window()),
        )
        .child(ui::icon_btn(t, "w-close", "close", 28.).on_click(|_, window, _| window.remove_window()))
}

/// Moves the window from a title bar: drag to move, double click to maximize, right click for the window menu.
pub fn drag_area(d: gpui::Stateful<gpui::Div>) -> gpui::Stateful<gpui::Div> {
    d.on_mouse_down(MouseButton::Left, |ev, window, _| {
        if ev.click_count >= 2 {
            window.zoom_window();
        } else {
            window.start_window_move();
        }
    })
    .on_mouse_down(MouseButton::Right, |ev, window, _| window.show_window_menu(ev.position))
}

/// A window other than the board: as it is when the system draws the frame, else under a title bar with the
/// window buttons and inside resize edges.
pub fn framed(window: &Window, t: &Theme, title: &str, body: impl IntoElement) -> AnyElement {
    let Some(tiling) = client(window) else { return body.into_any_element() };
    let maxed = window.is_maximized();
    let bar = drag_area(
        div()
            .id("frame-bar")
            .flex_none()
            .h(px(BAR))
            .pl(px(ui::EDGE))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(10.))
            .bg(t.bg2)
            .border_b_1()
            .border_color(t.rule),
    )
    .child(ui::mark(t, 14.))
    .child(ui::line(title.to_string()).flex_1().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink2))
    .child(buttons(t, maxed));
    div()
        .relative()
        .size_full()
        .flex()
        .flex_col()
        .font_family(crate::theme::UI)
        .child(bar)
        .child(div().flex_1().min_h_0().child(body))
        .children(edges(tiling))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_edges_skip_the_tiled_sides() {
        assert_eq!(edges(Tiling::default()).len(), 8, "four edges and four corners on a free window");
        assert!(edges(Tiling::tiled()).is_empty(), "a maximized or fullscreen window has none");
        // Snapped to the left half: no left edge and no left corners.
        assert_eq!(edges(Tiling { left: true, ..Default::default() }).len(), 5);
        // Snapped to the top right quarter: left, bottom and the corner between them.
        assert_eq!(edges(Tiling { top: true, right: true, ..Default::default() }).len(), 3);
    }
}
