//! The two-step stop: the first click morphs the button into "Confirm stop" with a visible three-second countdown and
//! a draining bar; it puts itself back when the time runs out, on Esc, or on a click anywhere else. Never "click again
//! to stop". A confirmed stop sends the chat flying to "Stopped today", where the freed memory ticks up.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, div, px, relative,
};

use crate::theme::Theme;
use crate::ui;
use crate::view::{BoardView, Ctx};

/// How long the confirm stays armed.
pub const ARM_SECS: f64 = 3.0;

#[derive(Clone, Debug)]
pub struct Armed {
    pub id: String,
    pub at: f64,
}

/// A chat on its way to "Stopped today".
#[derive(Clone, Debug)]
pub struct Flight {
    pub title: String,
    pub mb: Option<f64>,
    pub from: (f32, f32),
    pub at: f64,
}

pub const FLIGHT_MS: u64 = 780;

impl BoardView {
    pub fn armed(&self, id: &str, now: f64) -> Option<f64> {
        self.stop_armed.as_ref().filter(|a| a.id == id && now - a.at < ARM_SECS).map(|a| now - a.at)
    }

    /// Puts an armed Stop back (Esc, a click elsewhere). True when one was armed.
    pub fn disarm_stop(&mut self) -> bool {
        self.stop_armed.take().is_some()
    }

    /// Arms `id`, and redraws on each whole second so the number steps evenly (3, 2, 1) until it runs out; then it
    /// reverts by itself.
    pub fn arm_stop(&mut self, id: String, cx: &mut Context<Self>) {
        let at = crate::state::AppState::now();
        self.stop_armed = Some(Armed { id: id.clone(), at });
        cx.spawn(async move |this, cx| {
            for k in 1..=(ARM_SECS as u32) {
                let wait = (at + k as f64 - crate::state::AppState::now()).max(0.);
                cx.background_executor().timer(Duration::from_secs_f64(wait)).await;
                let gone = this
                    .update(cx, |v, cx| {
                        let still = v.stop_armed.as_ref().is_some_and(|a| a.id == id && a.at == at);
                        if still && crate::state::AppState::now() - at >= ARM_SECS {
                            v.stop_armed = None;
                        }
                        cx.notify();
                        !still
                    })
                    .unwrap_or(true);
                if gone {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    /// A Stop button for `id`. The second click (while armed) runs `go` and starts the flight from where it was clicked.
    pub fn stop_btn(
        &self,
        c: &Ctx,
        id: String,
        label: &str,
        title: String,
        mb: Option<f64>,
        go: impl Fn(&mut BoardView, &mut Context<BoardView>) + 'static,
    ) -> AnyElement {
        let t = c.t;
        let el = SharedString::from(format!("stop-{id}"));
        match self.armed(&id, c.now) {
            Some(gone) => {
                let left = (ARM_SECS - gone).ceil().max(1.) as i64;
                let bad = t.bad;
                let start = self.stop_armed.as_ref().map(|a| a.at).unwrap_or(0.);
                let frac0 = (1. - gone / ARM_SECS).clamp(0., 1.) as f32;
                div()
                    .id(el)
                    .flex_none()
                    .relative()
                    .h(px(28.))
                    .pl(px(11.))
                    .pr(px(5.))
                    .gap(px(8.))
                    .flex()
                    .items_center()
                    .rounded(px(ui::R_CTRL))
                    .overflow_hidden()
                    .bg(bad)
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(gpui::white())
                    .cursor_pointer()
                    .on_mouse_down_out(c.cx.listener(|this, _, _, cx| {
                        if this.disarm_stop() {
                            cx.notify();
                        }
                    }))
                    .child("Confirm stop")
                    .child(
                        div()
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(gpui::white().alpha(0.22))
                            .text_size(px(11.5))
                            .font_weight(FontWeight::BOLD)
                            .child(left.to_string()),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .bottom_0()
                            .h(px(2.))
                            .w_full()
                            .bg(gpui::white().alpha(0.55))
                            .with_animation(
                                SharedString::from(format!("drain-{start}")),
                                Animation::new(Duration::from_millis(((ARM_SECS - gone) * 1000.) as u64)),
                                move |d, k| d.w(relative(frac0 * (1. - k))),
                            ),
                    )
                    .on_click(c.cx.listener(move |this, ev: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.stop_armed = None;
                        let p = ev.position();
                        this.flight = Some(Flight {
                            title: title.clone(),
                            mb,
                            from: (f32::from(p.x), f32::from(p.y)),
                            at: crate::state::AppState::now(),
                        });
                        go(this, cx);
                        let at = this.flight.as_ref().map(|f| f.at);
                        cx.spawn(async move |this, cx| {
                            cx.background_executor().timer(Duration::from_millis(FLIGHT_MS + 120)).await;
                            let _ = this.update(cx, |v, cx| {
                                if v.flight.as_ref().map(|f| f.at) == at {
                                    v.flight = None;
                                }
                                cx.notify();
                            });
                        })
                        .detach();
                    }))
                    .with_animation(
                        SharedString::from(format!("morph-{start}")),
                        Animation::new(ui::FADE),
                        move |d, k| d.bg(bad.alpha(0.35 + 0.65 * k)),
                    )
                    .into_any_element()
            }
            None => ui::line_btn(&t, el, 28.)
                .px(px(11.))
                .text_size(px(12.5))
                .child(ui::icon("stop", 13., t.dim))
                .child(label.to_string())
                .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.arm_stop(id.clone(), cx);
                }))
                .into_any_element(),
        }
    }

    /// The flying pill, from the stop button to "Stopped today" (`to`, in window coordinates).
    pub fn flight_layer(&self, c: &Ctx, to: (f32, f32)) -> Option<AnyElement> {
        let f = self.flight.clone()?;
        let t = c.t;
        let (fx, fy) = (f.from.0 - 120., f.from.1 - 16.);
        let (tx, ty) = (to.0 - 120., to.1 - 16.);
        let lift = ((tx - fx).abs() * 0.15 + 30.).min(90.);
        Some(
            div()
                .absolute()
                .left(px(fx))
                .top(px(fy))
                .w(px(240.))
                .h(px(32.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded_full()
                .bg(t.panel2)
                .shadow(t.drop())
                .shadow(vec![Theme::ring(t.line2)])
                .text_size(px(12.5))
                .text_color(t.ink)
                .child(div().flex_none().size(px(7.)).rounded_full().bg(t.idle))
                .child(ui::line(f.title.clone()).flex_1())
                .children(f.mb.map(|m| {
                    div().flex_none().font_weight(FontWeight::BOLD).text_color(t.warm).child(sb_core::fmt::mem(m))
                }))
                .with_animation(
                    SharedString::from(format!("fly-{}", f.at)),
                    Animation::new(Duration::from_millis(FLIGHT_MS)),
                    move |d, k| {
                        // Up and over in an arc, shrinking as it lands.
                        let e = 1. - (1. - k).powi(3);
                        let x = fx + (tx - fx) * e;
                        let y = fy + (ty - fy) * e - lift * (4. * e * (1. - e));
                        d.left(px(x)).top(px(y)).opacity(1. - 0.85 * k * k)
                    },
                )
                .into_any_element(),
        )
    }
}
