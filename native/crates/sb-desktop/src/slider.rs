//! A slider with named stops: a rail, a tick and a label for each stop, one stop marked as recommended. The thumb
//! follows the pointer while it is dragged and lands on the nearest stop when let go. A value that is not a stop
//! (saved some other way) is shown where it is, between its neighbours.
//!
//! Pointer and touch dragging and the accessibility role come from gpui-base's unstyled slider; the look, the
//! stops and the keyboard are here. Keyboard: Tab reaches the thumb, the arrow keys (and Page Up/Down) move one
//! stop, Home and End go to the ends. The focus ring shows only when the keyboard brought focus there, as on
//! every other control (ui::focusable).

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext as _, BoxShadow, ClickEvent, Context, Entity, EventEmitter, FontWeight, InteractiveElement,
    IntoElement, KeyDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Window, div, point, px, relative,
};
use gpui_base::slider::{Slider, SliderEvent, SliderIndicator, SliderState, SliderThumb, SliderTrack};

use crate::theme::Theme;
use crate::ui;

/// The rail stops this far short of the control's edges, so the first and last labels sit centered under their
/// ticks without leaving the control.
const INSET: f32 = 44.;
/// The box each label is centered in (twice INSET).
const LABEL_W: f32 = 88.;
const THUMB: f32 = 20.;
const RAIL: f32 = 6.;

/// One stop: the words under its tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub label: SharedString,
    /// The stop most people should pick: its tick is ringed and it says "Recommended" under its label.
    pub recommended: bool,
}

pub enum StopEvent {
    /// The thumb is being dragged and is nearest this stop: nothing is chosen yet.
    Preview(usize),
    /// This stop was chosen: the thumb was let go there, a label was clicked, or a key moved it.
    Commit(usize),
}

pub struct StopSlider {
    state: Entity<SliderState>,
    stops: Vec<Stop>,
    dragging: bool,
    _sub: Subscription,
}

impl EventEmitter<StopEvent> for StopSlider {}

/// The stop one key press away from `pos`, which may be between two stops: up goes to the next stop above, down
/// to the next below, and both stay inside 0..=last.
pub fn step_from(pos: f32, up: bool, last: usize) -> usize {
    let pos = if pos.is_finite() { pos.clamp(0., last as f32) } else { 0. };
    if up { (pos.floor() as usize + 1).min(last) } else { (pos.ceil() as usize).saturating_sub(1) }
}

/// What a key does to a slider at `pos`: the stop it moves to, or None when the key is not the slider's.
pub fn key_stop(key: &str, pos: f32, last: usize) -> Option<usize> {
    match key {
        "right" | "up" | "pageup" => Some(step_from(pos, true, last)),
        "left" | "down" | "pagedown" => Some(step_from(pos, false, last)),
        "home" => Some(0),
        "end" => Some(last),
        _ => None,
    }
}

/// The stop a position rests on, or None between two stops.
pub fn stop_at(pos: f32, last: usize) -> Option<usize> {
    let near = pos.round();
    (pos.is_finite() && (pos - near).abs() < 0.01 && near >= 0. && near <= last as f32).then_some(near as usize)
}

impl StopSlider {
    /// `pos` is where the thumb starts, in stops (1.5 is halfway between the second and third).
    pub fn new(stops: Vec<Stop>, pos: f32, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let last = stops.len().saturating_sub(1).max(1) as f32;
        let state = cx.new(|_| SliderState::new().min(0.).max(last).step(1.).default_value(pos.clamp(0., last)));
        let _sub = cx.subscribe_in(&state, window, |this: &mut Self, state, ev: &SliderEvent, window, cx| match ev {
            SliderEvent::Change(v) => {
                this.dragging = true;
                cx.emit(StopEvent::Preview(this.index(v.end())));
                cx.notify();
            }
            SliderEvent::Release(v) => {
                // let go: the thumb lands on the stop it was nearest
                let i = this.index(v.end());
                state.update(cx, |s, cx| s.set_value(i as f32, window, cx));
                this.dragging = false;
                cx.emit(StopEvent::Commit(i));
                cx.notify();
            }
        });
        StopSlider { state, stops, dragging: false, _sub }
    }

    fn last(&self) -> usize {
        self.stops.len().saturating_sub(1).max(1)
    }

    fn index(&self, v: f32) -> usize {
        if v.is_finite() { (v.round().max(0.) as usize).min(self.last()) } else { 0 }
    }

    /// Where the thumb is, in stops.
    pub fn pos(&self, cx: &App) -> f32 {
        self.state.read(cx).value().end()
    }

    /// True while the pointer holds the thumb.
    pub fn dragging(&self) -> bool {
        self.dragging
    }

    /// Moves the thumb from outside (the saved value changed). Nothing is emitted.
    pub fn set_pos(&mut self, pos: f32, window: &mut Window, cx: &mut Context<Self>) {
        let pos = if pos.is_finite() { pos.clamp(0., self.last() as f32) } else { 0. };
        self.state.update(cx, |s, cx| s.set_value(pos, window, cx));
        cx.notify();
    }

    fn choose(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let i = i.min(self.last());
        self.set_pos(i as f32, window, cx);
        cx.emit(StopEvent::Commit(i));
    }

    fn key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let m = &ev.keystroke.modifiers;
        if m.platform || m.control || m.alt {
            return;
        }
        let Some(to) = key_stop(ev.keystroke.key.as_str(), self.pos(cx), self.last()) else { return };
        cx.stop_propagation();
        // at an end already: the key is the slider's, and nothing changes
        if stop_at(self.pos(cx), self.last()) != Some(to) {
            self.choose(to, window, cx);
        }
    }
}

impl Render for StopSlider {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let last = self.last();
        let (frac, pos) = {
            let s = self.state.read(cx);
            (s.percentage().end.clamp(0., 1.), s.value().end())
        };
        // the label that is lit: the stop the thumb rests on, or is nearest while dragged
        let lit = if self.dragging { Some(self.index(pos)) } else { stop_at(pos, last) };
        let at = |i: usize| i as f32 / last as f32;

        let knob_shadow = BoxShadow {
            color: gpui::black().opacity(if t.dark { 0.5 } else { 0.22 }),
            offset: point(px(0.), px(1.)),
            blur_radius: px(3.),
            spread_radius: px(0.),
            inset: false,
        };
        let edge = BoxShadow {
            color: gpui::black().opacity(if t.dark { 0.4 } else { 0.14 }),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(1.),
            inset: false,
        };
        // a soft glow rather than a hard spread: a wide spread keeps the knob's own corner size and turns square
        let halo = |a: f32| BoxShadow {
            color: t.accent.alpha(a),
            offset: point(px(0.), px(0.)),
            blur_radius: px(8.),
            spread_radius: px(2.),
            inset: false,
        };
        let mut ring = Theme::focus_ring(t.accent);
        ring.push(knob_shadow.clone());
        let rest = vec![edge.clone(), knob_shadow.clone()];
        let lifted = |a: f32| vec![halo(a), edge.clone(), knob_shadow.clone()];
        let (hover, press) = (lifted(0.35), lifted(0.55));
        let thumb = SliderThumb::new(&self.state)
            .absolute()
            .top(px((RAIL - THUMB) / 2.))
            .left(relative(frac))
            .ml(px(-THUMB / 2.))
            .size(px(THUMB))
            .rounded_full()
            .bg(gpui::white())
            .cursor_pointer()
            .shadow(if self.dragging { press.clone() } else { rest })
            .hover(move |s| s.shadow(hover))
            .active(move |s| s.shadow(press))
            .tab_index(0)
            .focus_visible(move |s| s.shadow(ring))
            .on_key_down(cx.listener(Self::key));

        let mut rail = SliderIndicator::new(&self.state)
            .relative()
            .flex_1()
            .h(px(RAIL))
            .rounded_full()
            .bg(t.track)
            .child(div().absolute().top_0().left_0().h_full().w(relative(frac)).rounded_full().bg(t.accent));
        for (i, s) in self.stops.iter().enumerate() {
            let passed = at(i) <= frac + 0.001;
            rail = rail.child(if s.recommended {
                // the recommended tick: a ring that stands out of the rail on both sides
                div()
                    .absolute()
                    .top(px((RAIL - 12.) / 2.))
                    .left(relative(at(i)))
                    .ml(px(-6.))
                    .size(px(12.))
                    .rounded_full()
                    .bg(t.bg)
                    .border_2()
                    .border_color(t.accent)
            } else {
                div()
                    .absolute()
                    .top(px((RAIL - 4.) / 2.))
                    .left(relative(at(i)))
                    .ml(px(-2.))
                    .size(px(4.))
                    .rounded_full()
                    .bg(if passed { t.on_accent.alpha(0.55) } else { t.faint })
            });
        }
        let track = SliderTrack::new(&self.state)
            .w_full()
            .h(px(28.))
            .px(px(INSET))
            .flex()
            .items_center()
            .cursor_pointer()
            .child(rail.child(thumb));

        let mut labels = div().relative().h(px(36.)).mx(px(INSET));
        for (i, s) in self.stops.iter().enumerate() {
            let on = lit == Some(i);
            let ink2 = t.ink2;
            labels = labels.child(
                div()
                    .id(("stop", i))
                    .absolute()
                    .top_0()
                    .left(relative(at(i)))
                    .ml(px(-LABEL_W / 2.))
                    .w(px(LABEL_W))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(1.))
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .text_color(if on { t.ink } else { t.dim })
                    .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .when(!on, |d| d.hover(move |st| st.text_color(ink2)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.choose(i, window, cx)))
                    .child(div().font_features(ui::tnum()).child(ui::line(s.label.clone())))
                    .when(s.recommended, |d| {
                        d.child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(t.faint)
                                .child(ui::line("Recommended")),
                        )
                    }),
            );
        }
        Slider::new(&self.state).w_full().flex().flex_col().gap(px(2.)).child(track).child(labels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arrow_moves_one_stop_and_stays_inside() {
        assert_eq!(step_from(2., true, 4), 3);
        assert_eq!(step_from(2., false, 4), 1);
        assert_eq!(step_from(4., true, 4), 4);
        assert_eq!(step_from(0., false, 4), 0);
    }

    #[test]
    fn from_between_two_stops_an_arrow_goes_to_the_stop_on_its_side() {
        assert_eq!(step_from(1.5, true, 4), 2);
        assert_eq!(step_from(1.5, false, 4), 1);
        assert_eq!(step_from(3.9, true, 4), 4);
        assert_eq!(step_from(0.2, false, 4), 0);
    }

    #[test]
    fn a_position_that_makes_no_sense_is_held_to_the_rail() {
        assert_eq!(step_from(-7., true, 4), 1);
        assert_eq!(step_from(99., false, 4), 3);
        assert_eq!(step_from(f32::NAN, true, 4), 1);
        assert_eq!(step_from(f32::INFINITY, false, 4), 0);
    }

    #[test]
    fn the_keys_a_slider_answers_to() {
        for k in ["right", "up", "pageup"] {
            assert_eq!(key_stop(k, 2., 4), Some(3), "{k}");
        }
        for k in ["left", "down", "pagedown"] {
            assert_eq!(key_stop(k, 2., 4), Some(1), "{k}");
        }
        assert_eq!(key_stop("home", 3., 4), Some(0));
        assert_eq!(key_stop("end", 1., 4), Some(4));
        for k in ["enter", "space", "tab", "a", "escape", ""] {
            assert_eq!(key_stop(k, 2., 4), None, "{k}");
        }
    }

    #[test]
    fn a_label_is_lit_only_when_the_thumb_rests_on_its_stop() {
        assert_eq!(stop_at(2., 4), Some(2));
        assert_eq!(stop_at(2.004, 4), Some(2));
        assert_eq!(stop_at(1.5, 4), None);
        assert_eq!(stop_at(3.45, 4), None);
        assert_eq!(stop_at(5., 4), None);
        assert_eq!(stop_at(-1., 4), None);
        assert_eq!(stop_at(f32::NAN, 4), None);
    }
}
