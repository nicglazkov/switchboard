//! The screens. One header (tall on welcome and done, short elsewhere, gliding between the two), one body that
//! animates in on every screen change, and one footer with the screen's buttons.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, ClickEvent, Context, Div, FontWeight, Hsla, InteractiveElement, IntoElement,
    ObjectFit, ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement, Styled, StyledImage, Window,
    div, ease_in_out, ease_out_quint, img, prelude::*, pulsating_between, px, relative, svg,
};

use super::theme::{BRAND, NUM, R_CARD, R_CTRL, Theme, UI, mix};
use super::{Installer, RowState, Screen};
use crate::brand::NAME;
use crate::version::Kind;

const TALL: f32 = 168.;
const SHORT: f32 = 76.;
const FOOTER: f32 = 68.;

/// The tour: screenshot (packaging/tour), title, one line.
const TOUR: [(&str, &str, &str); 4] = [
    (
        "1-board",
        "Every chat on one board",
        "Chats from all your computers, in columns by what they need: you, work, or nothing.",
    ),
    (
        "2-inspector",
        "Know what picking up a chat costs",
        "Each chat shows what resuming it would read again, and offers a fresh start from its summary when that is cheaper.",
    ),
    (
        "3-new",
        "Start a chat on any computer",
        "Pick the computer, the folder and the model, and Claude Code starts there.",
    ),
    ("4-history", "Find any chat again", "Search every chat by name, folder or prompt, and reopen it where it ran."),
];

impl Render for Installer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme(window);
        let body = match self.screen.clone() {
            Screen::Welcome => self.welcome(&t, cx),
            Screen::Tour(i) => self.tour(&t, i),
            Screen::Options => self.options(&t, cx),
            Screen::Working => self.working(&t, cx),
            Screen::Done => self.done(&t, cx),
            Screen::Uninstall => self.uninstall(&t, cx),
            Screen::Error(e) => self.error(&t, &e),
        };
        let footer = self.footer(&t, cx);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .font_family(UI)
            .text_color(t.ink)
            .child(self.header(&t))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .px(px(44.))
                    .pt(px(24.))
                    .child(enter(("body", self.nav), body.into_any_element()))
                    .overflow_hidden(),
            )
            .child(footer)
    }
}

/// The screen's content fades and rises into place.
fn enter(id: (&'static str, usize), el: AnyElement) -> AnyElement {
    div()
        .size_full()
        .child(el)
        .with_animation(id, Animation::new(Duration::from_millis(380)).with_easing(ease_out_quint()), |d, k| {
            d.opacity(k).mt(px(12. * (1. - k)))
        })
        .into_any_element()
}

/// The app's mark (three links meeting at a dot) at `size`, its dot breathing.
fn mark(t: &Theme, size: f32, id: &'static str) -> Div {
    let (links, nodes) = t.mark_colors();
    let layer = |n: &str, c: Hsla| {
        svg()
            .path(SharedString::from(format!("icons/{n}.svg")))
            .absolute()
            .top_0()
            .left_0()
            .size(px(size))
            .text_color(c)
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(layer("mark-links", links))
        .child(layer("mark-nodes", nodes))
        .child(div().absolute().top_0().left_0().size(px(size)).child(layer("mark-dot", t.accent)).with_animation(
            id,
            Animation::new(Duration::from_millis(2400)).repeat().with_easing(pulsating_between(0.55, 1.)),
            |d, k| d.opacity(k),
        ))
}

fn icon(name: &str, size: f32, color: Hsla) -> gpui::Svg {
    svg().path(SharedString::from(format!("icons/{name}.svg"))).size(px(size)).flex_none().text_color(color)
}

fn title(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().text_size(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(text.into())
}

fn sub(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().max_w(px(620.)).text_size(px(14.)).line_height(relative(1.5)).text_color(t.dim).child(text.into())
}

fn mono(t: &Theme, text: impl Into<SharedString>) -> Div {
    div().font_family(NUM).text_size(px(11.5)).text_color(t.faint).child(text.into())
}

fn card(t: &Theme) -> Div {
    div().flex().flex_col().rounded(px(R_CARD)).bg(t.panel).shadow(vec![Theme::ring(t.line)])
}

/// A fact on the welcome screen: an icon in a soft square, a line, and smaller lines under it (paths and links in
/// the mono face).
fn fact(t: &Theme, ic: &str, color: Hsla, line: String, more: Vec<String>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .py(px(10.))
        .child(
            div()
                .flex_none()
                .size(px(34.))
                .rounded(px(R_CTRL + 2.))
                .bg(color.opacity(0.12))
                .flex()
                .items_center()
                .justify_center()
                .child(icon(ic, 17., color)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .min_w_0()
                .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(line))
                .children(more.into_iter().map(|s| {
                    if s.starts_with('/') || s.starts_with("http") {
                        mono(t, s)
                    } else {
                        div().text_size(px(12.5)).text_color(t.dim).child(s)
                    }
                })),
        )
}

fn primary(t: &Theme, id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    let hov = mix(t.accent, t.ink, 0.1);
    button(id, 34.)
        .px(px(18.))
        .bg(t.accent)
        .text_color(t.on_accent)
        .font_weight(FontWeight::SEMIBOLD)
        .hover(move |s| s.bg(hov))
        .child(label.into())
}

fn danger(t: &Theme, id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    let hov = mix(t.bad, t.ink, 0.1);
    button(id, 34.)
        .px(px(18.))
        .bg(t.bad)
        .text_color(gpui::white())
        .font_weight(FontWeight::SEMIBOLD)
        .hover(move |s| s.bg(hov))
        .child(label.into())
}

fn secondary(t: &Theme, id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    let (hover, press) = (t.hover, t.press);
    button(id, 34.)
        .px(px(14.))
        .shadow(vec![Theme::ring(t.line2)])
        .text_color(t.ink)
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(press))
        .child(label.into())
}

fn quiet(t: &Theme, id: &'static str, label: impl Into<SharedString>) -> Stateful<Div> {
    let hover = t.hover;
    button(id, 34.).px(px(10.)).text_color(t.dim).hover(move |s| s.bg(hover)).child(label.into())
}

fn button(id: &'static str, h: f32) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(h))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_CTRL))
        .text_size(px(13.5))
        .cursor_pointer()
}

/// An on/off switch, as in the app's Settings.
fn switch(t: &Theme, on: bool) -> Div {
    let knob = div().size(px(16.)).rounded_full().bg(gpui::white()).shadow(vec![gpui::BoxShadow {
        color: gpui::black().opacity(0.25),
        offset: gpui::point(px(0.), px(1.)),
        blur_radius: px(2.),
        spread_radius: px(0.),
        inset: false,
    }]);
    div()
        .flex_none()
        .w(px(34.))
        .h(px(20.))
        .p(px(2.))
        .flex()
        .items_center()
        .rounded_full()
        .bg(if on { t.accent } else { t.track })
        .when(on, |d| d.justify_end())
        .child(knob)
}

/// A check box and its label.
fn check(t: &Theme, on: bool) -> Div {
    div()
        .flex_none()
        .size(px(18.))
        .rounded(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .when(on, |d| d.bg(t.accent).child(icon("check", 13., t.on_accent)))
        .when(!on, |d| d.shadow(vec![Theme::ring(t.line2)]).bg(t.panel))
}

/// A setting row: icon, label and explanation, and a control on the right. The whole row toggles it.
fn setting(t: &Theme, id: &'static str, ic: &str, label: &str, explain: &str, control: Div) -> Stateful<Div> {
    let hover = t.hover;
    let still = super::still();
    div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(14.))
        .px(px(16.))
        .py(px(14.))
        .cursor_pointer()
        .when(!still, |d| d.hover(move |s| s.bg(hover)))
        .child(
            div()
                .flex_none()
                .size(px(34.))
                .rounded(px(R_CTRL + 2.))
                .bg(t.panel3)
                .flex()
                .items_center()
                .justify_center()
                .child(icon(ic, 17., t.ink2)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child(label.to_string()))
                .child(
                    div().text_size(px(12.5)).line_height(relative(1.4)).text_color(t.dim).child(explain.to_string()),
                ),
        )
        .child(control)
}

impl Installer {
    fn header(&self, t: &Theme) -> AnyElement {
        let to = if self.screen.tall() { TALL } else { SHORT };
        let tall = self.screen.tall();
        let art = if t.dark { "installer/header-dark.png" } else { "installer/header-light.png" };
        let version = self.setup.as_ref().ok().map(|s| s.version.to_string());
        let caption =
            if self.uninstalling || matches!(self.screen, Screen::Uninstall) { "Uninstaller" } else { "Installer" };
        let brand = if tall {
            div().flex().items_center().gap(px(14.)).child(mark(t, 44., "mark-tall")).child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().font_family(BRAND).text_size(px(28.)).font_weight(FontWeight::BOLD).child(NAME))
                    .child(div().text_size(px(12.5)).text_color(t.faint).child(caption)),
            )
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(mark(t, 24., "mark-short"))
                .child(div().font_family(BRAND).text_size(px(16.)).font_weight(FontWeight::BOLD).child(NAME))
                .child(div().text_size(px(13.)).text_color(t.faint).child(caption))
        };
        let el = div()
            .relative()
            .flex_none()
            .w_full()
            .overflow_hidden()
            // The short header shows the middle band of the art, where its lines meet behind the version.
            .child(
                img(art)
                    .absolute()
                    .top(px(if tall { 0. } else { -(TALL - SHORT) / 2. }))
                    .left_0()
                    .w_full()
                    .h(px(TALL))
                    .object_fit(ObjectFit::Cover),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(if tall { TALL } else { SHORT }))
                    .px(px(44.))
                    .pt(px(if MAC_TITLEBAR { 14. } else { 0. }))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(brand)
                    .when_some(version, |d, v| {
                        d.child(
                            div()
                                .px(px(8.))
                                .py(px(3.))
                                .rounded(px(R_CTRL))
                                .bg(t.panel)
                                .shadow(vec![Theme::ring(t.line)])
                                .child(mono(t, v).text_color(t.dim)),
                        )
                    }),
            );
        let from = self.header_from;
        if (from - to).abs() < 0.5 {
            return el.h(px(to)).into_any_element();
        }
        el.with_animation(
            ("header", self.nav),
            Animation::new(Duration::from_millis(420)).with_easing(ease_in_out),
            move |d, k| d.h(px(from + (to - from) * k)),
        )
        .into_any_element()
    }

    fn footer(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let mut left: Vec<AnyElement> = vec![];
        let mut right: Vec<AnyElement> = vec![];
        let installed = self.installed.is_some();
        match &self.screen {
            Screen::Welcome => {
                let downgrade = self.setup.as_ref().is_ok_and(|s| matches!(s.kind, Kind::Downgrade { .. }));
                if installed {
                    left.push(
                        quiet(t, "to-uninstall", "Uninstall")
                            .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.go(Screen::Uninstall, cx)))
                            .into_any_element(),
                    );
                }
                if downgrade || self.setup.is_err() {
                    right.push(primary(t, "close", "Close").on_click(|_, _, cx| cx.quit()).into_any_element());
                } else {
                    let update = self.setup.as_ref().is_ok_and(|s| s.installed.is_some());
                    if update {
                        right.push(
                            secondary(t, "tour", "Take the tour")
                                .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.go(Screen::Tour(0), cx)))
                                .into_any_element(),
                        );
                    }
                    let label = if update { "Update" } else { "Continue" };
                    let go = primary(t, "continue", label);
                    let go = if update {
                        go.on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.after_tour(cx)))
                    } else {
                        go.on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.after_welcome(cx)))
                    };
                    right.push(go.into_any_element());
                }
            }
            Screen::Tour(i) => {
                let i = *i;
                left.push(self.dots(t, i, cx));
                right.push(
                    quiet(t, "skip", "Skip the tour")
                        .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.after_tour(cx)))
                        .into_any_element(),
                );
                right.push(
                    secondary(t, "back", "Back")
                        .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| {
                            v.go(if i == 0 { Screen::Welcome } else { Screen::Tour(i - 1) }, cx)
                        }))
                        .into_any_element(),
                );
                let last = i + 1 == TOUR.len();
                right.push(
                    primary(t, "next", if last { "Continue" } else { "Next" })
                        .on_click(cx.listener(
                            move |v, _: &ClickEvent, _, cx| {
                                if last { v.after_tour(cx) } else { v.go(Screen::Tour(i + 1), cx) }
                            },
                        ))
                        .into_any_element(),
                );
            }
            Screen::Options => {
                let back = if self.setup.as_ref().is_ok_and(|s| s.installed.is_none()) {
                    Screen::Tour(TOUR.len() - 1)
                } else {
                    Screen::Welcome
                };
                right.push(
                    secondary(t, "back", "Back")
                        .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.go(back.clone(), cx)))
                        .into_any_element(),
                );
                let label = if self.setup.as_ref().is_ok_and(|s| s.is_update()) { "Update" } else { "Install" };
                right.push(
                    primary(t, "install", label)
                        .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.start_install(cx)))
                        .into_any_element(),
                );
            }
            Screen::Working => {
                left.push(
                    div()
                        .text_size(px(12.5))
                        .text_color(t.faint)
                        .child("Keep this window open until it finishes.")
                        .into_any_element(),
                );
            }
            Screen::Done => {
                right.push(
                    primary(t, "done", "Done")
                        .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.finish(cx)))
                        .into_any_element(),
                );
            }
            Screen::Uninstall => {
                right.push(
                    secondary(t, "cancel", "Cancel")
                        .on_click(cx.listener(
                            |v, _: &ClickEvent, _, cx| {
                                if v.uninstall_only { cx.quit() } else { v.go(Screen::Welcome, cx) }
                            },
                        ))
                        .into_any_element(),
                );
                right.push(
                    danger(t, "uninstall", "Uninstall")
                        .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.start_uninstall(cx)))
                        .into_any_element(),
                );
            }
            Screen::Error(_) => {
                right.push(primary(t, "close", "Close").on_click(|_, _, cx| cx.quit()).into_any_element());
            }
        }
        div()
            .flex_none()
            .h(px(FOOTER))
            .px(px(32.))
            .flex()
            .items_center()
            .justify_between()
            .border_t_1()
            .border_color(t.line)
            .bg(if t.dark { t.bg } else { t.panel })
            .child(div().flex().items_center().gap(px(8.)).children(left))
            .child(div().flex().items_center().gap(px(10.)).children(right))
            .into_any_element()
    }

    fn dots(&self, t: &Theme, at: usize, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .children((0..TOUR.len()).map(|i| {
                let on = i == at;
                div()
                    .id(("dot", i))
                    .h(px(8.))
                    .w(px(if on { 22. } else { 8. }))
                    .rounded_full()
                    .bg(if on { t.accent } else { t.line2 })
                    .cursor_pointer()
                    .on_click(cx.listener(move |v, _: &ClickEvent, _, cx| v.go(Screen::Tour(i), cx)))
            }))
            .child(div().ml(px(8.)).child(mono(t, format!("{} of {}", at + 1, TOUR.len()))))
            .into_any_element()
    }

    fn welcome(&self, t: &Theme, _cx: &mut Context<Self>) -> AnyElement {
        let s = match &self.setup {
            Ok(s) => s,
            Err(e) => {
                return div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(title(t, "The installer cannot continue"))
                    .child(sub(t, e.clone()))
                    .into_any_element();
            }
        };
        if let Some(no) = s.refusal() {
            return div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(title(t, s.title()))
                .child(sub(t, no))
                .child(
                    card(t).child(
                        fact(
                            t,
                            "info",
                            t.accent,
                            format!("To update, download {NAME} again from its home page."),
                            vec![crate::brand::HOMEPAGE.to_string()],
                        )
                        .px(px(16.)),
                    ),
                )
                .into_any_element();
        }
        let update = s.installed.is_some();
        let lead = if update {
            "Your settings, chats and choices stay as they are. Only the app is replaced.".to_string()
        } else {
            format!(
                "{NAME} puts every Claude Code chat from all of your computers on one board, so you can see which ones need you."
            )
        };
        let place = if update && s.why_dir.is_none() {
            format!("Replaces the copy in {}", s.dir.label)
        } else {
            format!("Installs into {}", s.dir.label)
        };
        let mut more: Vec<String> = s.why_dir.iter().map(|w| format!("{w}.")).collect();
        more.push(s.dest.display().to_string());
        let mut facts = vec![
            fact(t, "folder", t.accent, place, more),
            fact(t, "lock", t.warm, "No admin password, and nothing else is installed".into(), vec![]),
        ];
        if !update {
            facts.push(fact(
                t,
                "user",
                t.ink2,
                "Opening at login and the Dock icon start off".into(),
                vec!["You choose after the tour.".into()],
            ));
        }
        if s.running {
            facts.push(fact(t, "warn", t.cold, format!("{NAME} is open. It will be asked to quit first."), vec![]));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(title(t, s.title()))
            .child(sub(t, lead))
            .child(div().mt(px(10.)).flex().flex_col().children(facts))
            .into_any_element()
    }

    fn tour(&self, t: &Theme, i: usize) -> AnyElement {
        let (file, head, line) = TOUR[i];
        let shot = format!("tour/{file}-{}.png", if t.dark { "dark" } else { "light" });
        let (w, h) = (544., 340.);
        div()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w(px(w))
                    .h(px(h))
                    .rounded(px(R_CARD))
                    .overflow_hidden()
                    .bg(t.panel)
                    .shadow({
                        let mut s = t.drop();
                        s.push(Theme::ring(t.line2));
                        s
                    })
                    .child(img(shot).w(px(w)).h(px(h)).object_fit(ObjectFit::Cover))
                    .with_animation(
                        ("shot", self.nav),
                        Animation::new(Duration::from_millis(450)).with_easing(ease_out_quint()),
                        |d, k| d.opacity(0.2 + 0.8 * k),
                    ),
            )
            .child(div().mt(px(18.)).text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(head))
            .child(
                div()
                    .mt(px(4.))
                    .max_w(px(560.))
                    .text_center()
                    .text_size(px(13.5))
                    .line_height(relative(1.45))
                    .text_color(t.dim)
                    .child(line),
            )
            .into_any_element()
    }

    fn options(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let Ok(s) = &self.setup else { return div().into_any_element() };
        let c = self.choices;
        let rows = card(t)
            .overflow_hidden()
            .child(
                setting(
                    t,
                    "opt-login",
                    "user",
                    &format!("Open {NAME} when you log in"),
                    "Adds a login item that opens the app. Nothing else runs in the background.",
                    switch(t, c.open_at_login),
                )
                .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                    v.choices.open_at_login = !v.choices.open_at_login;
                    cx.notify();
                })),
            )
            .child(div().h(px(1.)).mx(px(16.)).bg(t.line))
            .child(
                setting(
                    t,
                    "opt-dock",
                    "window",
                    &format!("Add {NAME} to the Dock"),
                    "Puts its icon at the end of the Dock.",
                    switch(t, c.add_to_dock),
                )
                .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                    v.choices.add_to_dock = !v.choices.add_to_dock;
                    cx.notify();
                })),
            );
        let place = match &s.why_dir {
            Some(why) => format!("It goes in {}, since {}.", s.dir.label, lowercase_first(why)),
            None => format!("It goes in {}.", s.dir.label),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(title(t, "Two choices, both off unless you turn them on"))
            .child(sub(t, "These are asked once. Later updates leave them as you set them, and you can change them any time in System Settings."))
            .child(div().mt(px(12.)).child(rows))
            .child(
                div()
                    .mt(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(icon("folder", 15., t.faint))
                    .child(div().text_size(px(13.)).text_color(t.dim).child(place))
                    .child(mono(t, s.dest.display().to_string())),
            )
            .into_any_element()
    }

    fn working(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let head = if self.uninstalling {
            format!("Uninstalling {NAME}")
        } else {
            match self.setup.as_ref().map(|s| (&s.kind, &s.version)) {
                Ok((Kind::Update { .. } | Kind::Unknown, v)) => format!("Updating {NAME} to {v}"),
                Ok((_, v)) => format!("Installing {NAME} {v}"),
                Err(_) => format!("Installing {NAME}"),
            }
        };
        let total = self.rows.len().max(1) as f32;
        let done = self.rows.iter().filter(|r| r.state == RowState::Done).count() as f32;
        let active = self.rows.iter().any(|r| r.state == RowState::Active);
        let frac = ((done + if active { 0.5 } else { 0. }) / total).min(1.);
        let bar = div()
            .w_full()
            .h(px(6.))
            .rounded_full()
            .bg(t.track)
            .overflow_hidden()
            .child(div().h_full().rounded_full().bg(t.accent).w(relative(frac)));
        // While waiting on the user, only the step that is stuck is listed, so the question has room.
        let shown = self.rows.iter().enumerate().filter(|(_, r)| !self.stuck || r.state != RowState::Waiting);
        let rows = card(t).py(px(6.)).children(shown.map(|(i, r)| {
            let glyph: AnyElement = match r.state {
                RowState::Waiting => {
                    div().size(px(14.)).rounded_full().border_1().border_color(t.line2).into_any_element()
                }
                RowState::Active => div()
                    .size(px(14.))
                    .rounded_full()
                    .bg(t.accent)
                    .with_animation(
                        ("active", i),
                        Animation::new(Duration::from_millis(1100)).repeat().with_easing(pulsating_between(0.3, 1.)),
                        |d, k| d.opacity(k),
                    )
                    .into_any_element(),
                RowState::Done => div()
                    .size(px(18.))
                    .rounded_full()
                    .bg(t.accent)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("check", 12., t.on_accent))
                    .with_animation(
                        ("done", i),
                        Animation::new(Duration::from_millis(300)).with_easing(ease_out_quint()),
                        |d, k| d.opacity(k),
                    )
                    .into_any_element(),
            };
            div()
                .h(px(40.))
                .px(px(16.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(div().flex_none().size(px(20.)).flex().items_center().justify_center().child(glyph))
                .child(
                    div()
                        .text_size(px(14.))
                        .text_color(if r.state == RowState::Waiting { t.faint } else { t.ink })
                        .font_weight(if r.state == RowState::Active { FontWeight::MEDIUM } else { FontWeight::NORMAL })
                        .child(r.task.label()),
                )
                .child(div().flex_1().min_w_0().flex().justify_end().child(mono(t, r.detail.clone()).truncate()))
        }));
        let stuck = self.stuck.then(|| {
            card(t)
                .mt(px(14.))
                .p(px(16.))
                .gap(px(10.))
                .bg(t.cold.opacity(if t.dark { 0.10 } else { 0.08 }))
                .shadow(vec![Theme::ring(t.cold.opacity(0.45))])
                .child(div().flex().items_center().gap(px(10.)).child(icon("warn", 17., t.cold)).child(
                    div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(format!("{NAME} did not quit")),
                ))
                .child(div().text_size(px(13.)).line_height(relative(1.45)).text_color(t.ink2).child(
                    "It may be waiting on something, such as a question it asked. Switch to it and quit it, or \
                         stop it now. Stopping it closes it without asking.",
                ))
                .child(
                    div()
                        .flex()
                        .gap(px(10.))
                        .child(
                            danger(t, "force", "Stop it now")
                                .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.answer_force(true, cx))),
                        )
                        .child(
                            secondary(t, "noforce", "Cancel")
                                .on_click(cx.listener(|v, _: &ClickEvent, _, cx| v.answer_force(false, cx))),
                        ),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(title(t, head))
            .child(div().mt(px(10.)).child(bar))
            .child(div().mt(px(14.)).child(rows))
            .children(stuck)
            .into_any_element()
    }

    fn done(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (head, line) = if self.uninstalling {
            let kept = !self.remove_data;
            (
                format!("{NAME} is uninstalled"),
                if kept {
                    "Your settings are kept, so installing it again picks up where you left off.".to_string()
                } else {
                    "Your settings were removed too.".to_string()
                },
            )
        } else {
            match &self.setup {
                Ok(s) => (
                    match &s.kind {
                        Kind::Update { .. } | Kind::Unknown => format!("{NAME} is updated to {}", s.version),
                        Kind::Reinstall => format!("{NAME} {} is reinstalled", s.version),
                        _ => format!("{NAME} is installed"),
                    },
                    format!("It is in {}.", s.dir.label),
                ),
                Err(e) => (String::new(), e.clone()),
            }
        };
        let badge = div()
            .size(px(56.))
            .rounded_full()
            .bg(t.accent.opacity(0.14))
            .flex()
            .items_center()
            .justify_center()
            .child(div().size(px(36.)).rounded_full().bg(t.accent).flex().items_center().justify_center().child(icon(
                "check",
                20.,
                t.on_accent,
            )))
            .with_animation(
                ("badge", self.nav),
                Animation::new(Duration::from_millis(520)).with_easing(ease_out_quint()),
                |d, k| d.opacity(k).size(px(40. + 16. * k)),
            );
        let mut after: Vec<AnyElement> = vec![];
        if !self.uninstalling && self.setup.is_ok() {
            after.push(
                setting(
                    t,
                    "open-now",
                    "chev",
                    &format!("Open {NAME} now"),
                    "Starts it once this window closes.",
                    check(t, self.open_after),
                )
                .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                    v.open_after = !v.open_after;
                    cx.notify();
                }))
                .into_any_element(),
            );
            if self.setup.as_ref().is_ok_and(|s| s.disk_image.is_some()) {
                after.push(div().h(px(1.)).mx(px(16.)).bg(t.line).into_any_element());
                after.push(
                    setting(
                        t,
                        "eject",
                        "archive",
                        "Eject the disk image and move it to the Trash",
                        "It is not needed once the app is installed.",
                        check(t, self.eject_after),
                    )
                    .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                        v.eject_after = !v.eject_after;
                        cx.notify();
                    }))
                    .into_any_element(),
                );
            }
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(div().size(px(56.)).flex().items_center().justify_center().child(badge))
                    .child(div().flex().flex_col().gap(px(4.)).child(title(t, head)).child(sub(t, line))),
            )
            .children(self.notes.iter().map(|n| {
                div()
                    .mt(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("info", 14., t.cold))
                    .child(div().text_size(px(12.5)).text_color(t.ink2).child(n.clone()))
            }))
            .when(!after.is_empty(), |d| d.child(card(t).mt(px(18.)).overflow_hidden().children(after)))
            .into_any_element()
    }

    fn uninstall(&self, t: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let version = self.installed.as_ref().and_then(|(_, _, v)| v.clone());
        let place = self.installed.as_ref().map(|(d, _, _)| d.label.clone()).unwrap_or_default();
        let plan = self.shown_plan().unwrap_or_default();
        let list = |head: &str, ic: &str, color: Hsla, items: Vec<&'static str>| {
            card(t)
                .flex_1()
                .p(px(16.))
                .gap(px(10.))
                .child(
                    div().flex().items_center().gap(px(8.)).child(icon(ic, 15., color)).child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.dim)
                            .child(head.to_string()),
                    ),
                )
                .children(if items.is_empty() {
                    vec![div().text_size(px(13.5)).text_color(t.faint).child("Nothing")]
                } else {
                    items.into_iter().map(|l| div().text_size(px(13.5)).child(l)).collect()
                })
        };
        let removed = crate::uninstall::Plan::labels(&plan.remove);
        let kept = crate::uninstall::Plan::labels(&plan.keep);
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(title(
                t,
                match version {
                    Some(v) => format!("Uninstall {NAME} {v}"),
                    None => format!("Uninstall {NAME}"),
                },
            ))
            .child(sub(
                t,
                format!(
                    "From {place}. Your Claude Code chats, and the {NAME} server on any computer, are not touched."
                ),
            ))
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .gap(px(12.))
                    .child(list("Removed", "x", t.bad, removed))
                    .child(list("Kept", "lock", t.warm, kept)),
            )
            .child(
                card(t).mt(px(12.)).overflow_hidden().child(
                    setting(
                        t,
                        "remove-data",
                        "archive",
                        "Also remove your settings and saved layout",
                        "Off keeps them for a later install.",
                        switch(t, self.remove_data),
                    )
                    .on_click(cx.listener(|v, _: &ClickEvent, _, cx| {
                        v.remove_data = !v.remove_data;
                        cx.notify();
                    })),
                ),
            )
            .into_any_element()
    }

    fn error(&self, t: &Theme, e: &str) -> AnyElement {
        let head =
            if self.uninstalling { "Uninstall stopped".to_string() } else { format!("{NAME} was not installed") };
        div()
            .flex()
            .items_start()
            .gap(px(16.))
            .child(
                div()
                    .flex_none()
                    .size(px(48.))
                    .rounded_full()
                    .bg(t.bad.opacity(0.12))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(icon("warn", 22., t.bad)),
            )
            .child(div().flex().flex_col().gap(px(6.)).child(title(t, head)).child(sub(t, e.to_string())))
            .into_any_element()
    }
}

/// On a Mac the header sits under the transparent title bar, so its content moves down past the traffic lights.
const MAC_TITLEBAR: bool = super::MAC;

fn lowercase_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().chain(c).collect(),
        None => String::new(),
    }
}
