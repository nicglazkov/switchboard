//! Small shared pieces: icons, lamps, tags, chips, key caps, meters, buttons, labels.

use gpui::{Div, FontWeight, Hsla, ParentElement, SharedString, Styled, Svg, div, prelude::*, px, relative, svg};
use sb_core::Lamp;

use crate::theme::{NUM, Theme};

pub const MAC: bool = cfg!(target_os = "macos");
/// Windows draws no title bar of its own: the board's header holds the caption buttons, like Zed and the Office apps.
/// `shots` builds draw that header anywhere with SB_AS_WINDOWS=1, so it can be checked on a Linux display.
pub fn win() -> bool {
    #[cfg(feature = "shots")]
    if std::env::var_os("SB_AS_WINDOWS").is_some() {
        return true;
    }
    cfg!(target_os = "windows")
}

/// The header's height on every platform: the traffic lights or caption buttons sit centered on the tab row.
pub fn header() -> f32 {
    40.
}

/// The bottom bars' height (the board's key hints and the terminal's bar), on the same grid as the header.
pub const FOOTER: f32 = 32.;

/// The left edge of the header's and footer's content past the window controls, so both line up.
pub const EDGE: f32 = 20.;

/// One left inset for the header and every footer (the board's and the terminal's): past the traffic lights on a
/// Mac, so the footer's first item sits right under the header's first item (the Switchboard mark); the shared
/// edge elsewhere. One number, so the two bars can never drift apart again.
pub fn lead() -> f32 {
    if MAC { 84. } else { EDGE }
}

/// One radius scale for every rounded thing, so nested shapes read as one family. Rule: a child that sits inside
/// a rounded parent with padding p gets the parent's radius minus p (never below R_MIN). GPUI clips children to a
/// rectangle, not to the parent's curve, so a child that touches a rounded edge takes the same radius on that edge
/// (or is drawn as an inset shadow, like the "Needs you" edge on a card), never a square bar under a round corner.
/// Sheets and dialogs.
pub const R_SHEET: f32 = 14.;
/// Popovers and menus (6 px padding, so their rows get R_ITEM).
pub const R_POP: f32 = 12.;
/// Cards and panels inside a window or a sheet.
pub const R_CARD: f32 = 10.;
/// Buttons, fields and segmented controls.
pub const R_CTRL: f32 = 7.;
/// Rows inside a popover or a menu: R_POP less its 6 px padding.
pub const R_ITEM: f32 = 6.;
/// Small tags and badges.
pub const R_CHIP: f32 = 5.;
pub const R_MIN: f32 = 4.;
/// Padding inside a card.
pub const PAD_CARD: f32 = 12.;

/// The modifier as a key cap shows it: ⌘ on a Mac, Ctrl elsewhere.
pub fn mod_cap(key: &str) -> String {
    if MAC { format!("⌘{key}") } else { format!("Ctrl {key}") }
}

pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    let name = if crate::assets::has_icon(name) { name } else { "chev" };
    svg().path(SharedString::from(format!("icons/{name}.svg"))).size(px(size)).flex_none().text_color(color)
}

/// An icon that keeps turning (a Start fresh at work: its spiral spins until the new chat is up).
pub fn spin_icon(id: impl Into<gpui::ElementId>, name: &str, size: f32, color: Hsla) -> gpui::AnyElement {
    use gpui::AnimationExt;
    icon(name, size, color)
        .with_animation(id, gpui::Animation::new(std::time::Duration::from_millis(1100)).repeat(), |s, k| {
            s.with_transformation(gpui::Transformation::rotate(gpui::percentage(k)))
        })
        .into_any_element()
}

/// An icon that breathes: something queued, waiting its turn rather than working.
pub fn pulse_icon(id: impl Into<gpui::ElementId>, name: &str, size: f32, color: Hsla) -> gpui::AnyElement {
    use gpui::AnimationExt;
    icon(name, size, color)
        .with_animation(id, gpui::Animation::new(std::time::Duration::from_millis(1600)).repeat().with_easing(gpui::pulsating_between(0.35, 1.0)), |s, k| s.opacity(k))
        .into_any_element()
}

/// How a corner card or toast comes in: up a few pixels and into view.
pub fn rise<E: gpui::IntoElement + gpui::Styled + 'static>(el: E, id: impl Into<gpui::ElementId>) -> gpui::AnyElement {
    use gpui::AnimationExt;
    el.with_animation(id, gpui::Animation::new(std::time::Duration::from_millis(220)).with_easing(gpui::ease_out_quint()), |d, k| {
        d.opacity(k).top(gpui::px(8. * (1. - k)))
    })
    .into_any_element()
}

static MACHINE_OS: std::sync::RwLock<Vec<(String, String)>> = std::sync::RwLock::new(Vec::new());

/// Remembers each machine's OS from the latest snapshot, so a machine's icon never depends on its name.
pub fn set_machine_os(list: Vec<(String, String)>) {
    if let Ok(mut w) = MACHINE_OS.write() {
        *w = list;
    }
}

/// A machine's icon, by its OS: a Mac, or a PC tower for Windows and Linux. "auto" is the Auto pick.
pub fn machine_icon(name: &str, size: f32, color: Hsla) -> Svg {
    if name == "auto" {
        return icon("auto", size, color);
    }
    let os = MACHINE_OS.read().ok().and_then(|l| l.iter().find(|(n, _)| n == name).map(|(_, o)| o.clone())).unwrap_or_default();
    icon(if os == "windows" || os == "linux" { "tower" } else { "mac" }, size, color)
}

/// The Converge mark: three nodes converging on one dot.
pub fn mark(t: &Theme, size: f32) -> Div {
    let layer = |n: &str, c: Hsla| svg().path(SharedString::from(format!("icons/{n}.svg"))).absolute().top_0().left_0().size(px(size)).text_color(c);
    let (links, nodes) = if t.dark { (gpui::rgb(0x5C79C9).into(), gpui::rgb(0x7E8AD2).into()) } else { (gpui::rgb(0x7E95D6).into(), gpui::rgb(0x5F6CB8).into()) };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(layer("mark-links", links))
        .child(layer("mark-nodes", nodes))
        .child(layer("mark-dot", t.accent))
}

/// A state lamp. Held gets a halo, idle and stalled are hollow rings.
pub fn lamp(t: &Theme, l: Lamp, size: f32) -> Div {
    let c = t.lamp(l);
    let d = div().flex_none().size(px(size)).rounded_full();
    match l {
        Lamp::Idle | Lamp::Stall => d.border_1().border_color(c),
        Lamp::Held => d.bg(c).shadow(vec![gpui::BoxShadow {
            color: c.alpha(0.28),
            offset: gpui::point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(2.),
            inset: false,
        }]),
        _ => d.bg(c),
    }
}

/// Replaced chats: a hollow ring in the idle color.
pub fn ring_lamp(t: &Theme, size: f32) -> Div {
    div().flex_none().size(px(size)).rounded_full().border_1().border_color(t.line2)
}

/// "(part 2)", "(from Laptop)": the name's tag, small and quiet.
pub fn tag(t: &Theme, text: &str) -> Div {
    tag_in(text, t.chip_bg, t.ink2)
}

fn tag_in(text: &str, bg: Hsla, ink: Hsla) -> Div {
    div()
        .flex_none()
        .px(px(6.))
        .h(px(18.))
        .flex()
        .items_center()
        .rounded(px(R_CHIP))
        .bg(bg)
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(ink)
        .child(text.to_string())
}

/// A card's live state on the right of its name: "working", "warm", "cold". A soft fill of the state's color.
pub fn state_pill(t: &Theme, text: &str, c: Hsla) -> Div {
    tag_in(text, c.alpha(if t.dark { 0.16 } else { 0.12 }), c).rounded(px(9.)).px(px(7.))
}

/// A name's tag by its kind: UltraCode in the accent, "for Studio" (a dispatched chat doing another machine's work) in
/// the accent with an arrow, every other tag quiet ("part 2" and "copy" say how a chat was born, not its state).
pub fn name_tag(t: &Theme, text: &str, kind: Option<sb_core::names::TagKind>) -> Div {
    use sb_core::names::TagKind;
    let soft = t.accent.alpha(if t.dark { 0.15 } else { 0.11 });
    match kind {
        Some(TagKind::For) => div()
            .flex_none()
            .pl(px(4.))
            .pr(px(6.))
            .h(px(18.))
            .flex()
            .items_center()
            .gap(px(3.))
            .rounded(px(R_CHIP))
            .bg(soft)
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.accent)
            .child(icon("for", 10., t.accent))
            .child(text.to_string()),
        Some(TagKind::Ultra) => tag_in(text, soft, t.accent),
        _ => tag(t, text),
    }
}

/// An outlined chip in the inspector's chip row for something Switchboard does, not Claude ("Kept running").
pub fn note_tag(t: &Theme, ic: &str, text: &str) -> Div {
    div()
        .flex_none()
        .h(px(20.))
        .px(px(7.))
        .gap(px(5.))
        .flex()
        .items_center()
        .rounded(px(R_MIN))
        .border_1()
        .border_color(t.line2)
        .font_family(NUM)
        .text_size(px(10.5))
        .text_color(t.ink2)
        .child(icon(ic, 11., t.ink2))
        .child(text.to_uppercase())
}

/// A key cap: 16 px, a hairline ring, number font.
pub fn kbd(t: &Theme, key: &str) -> Div {
    kbd_in(key, t.faint, t.line)
}

pub fn kbd_in(key: &str, ink: Hsla, ring: Hsla) -> Div {
    div()
        .flex_none()
        .h(px(16.))
        .min_w(px(16.))
        .px(px(4.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_MIN))
        .shadow(vec![Theme::ring(ring)])
        .font_family(NUM)
        .text_size(px(10.5))
        .text_color(ink)
        .child(SharedString::from(key.to_string()))
}

/// Section label: small, bold, uppercase.
pub fn lbl(text: &str, color: Hsla) -> Div {
    div().text_size(px(9.5)).font_weight(FontWeight::BOLD).text_color(color).child(text.to_uppercase())
}

/// A thin meter with an optional tick (where the week lands, or the 150k line).
pub fn meter(t: &Theme, frac: f32, color: Hsla, tick: Option<f32>, h: f32) -> Div {
    let frac = frac.clamp(0.0, 1.0);
    div()
        .relative()
        .w_full()
        .h(px(h))
        .rounded_full()
        .bg(t.track)
        .child(div().absolute().top_0().left_0().h_full().w(relative(frac)).rounded_full().bg(color))
        .when_some(tick, |d, k| {
            d.child(div().absolute().top(px(-2.)).h(px(h + 4.)).w(px(1.5)).left(relative(k.clamp(0.0, 1.0))).bg(t.ink2))
        })
}

/// Number text (JetBrains Mono).
pub fn num(text: impl Into<SharedString>, size: f32, color: Hsla) -> Div {
    div().flex_none().font_family(NUM).text_size(px(size)).text_color(color).child(text.into())
}

/// A quiet text button: 24 px, dim, hover wash.
pub fn ghost(t: &Theme, id: impl Into<gpui::ElementId>, label: Option<&str>, ic: Option<&str>) -> gpui::Stateful<Div> {
    let hover = t.hover;
    let press = t.press;
    div()
        .id(id.into())
        .flex_none()
        .h(px(24.))
        .px(px(6.))
        .gap(px(5.))
        .flex()
        .items_center()
        .rounded(px(R_ITEM))
        .text_size(px(12.))
        .text_color(t.dim)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(press))
        .when_some(ic, |d, i| d.child(icon(i, 13., t.dim)))
        .when_some(label, |d, l| d.child(SharedString::from(l.to_string())))
}

/// A bordered button (Cancel, + New).
pub fn line_btn(t: &Theme, id: impl Into<gpui::ElementId>, h: f32) -> gpui::Stateful<Div> {
    let hover = t.hover;
    let press = t.press;
    div()
        .id(id.into())
        .flex_none()
        .h(px(h))
        .px(px(10.))
        .gap(px(6.))
        .flex()
        .items_center()
        .rounded(px(R_CTRL))
        .shadow(vec![Theme::ring(t.line2)])
        .text_size(px(12.5))
        .text_color(t.ink)
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(press))
}

/// The accent button (Start, Open in Claude).
pub fn primary_btn(t: &Theme, id: impl Into<gpui::ElementId>, h: f32) -> gpui::Stateful<Div> {
    let hov = crate::theme::mix(t.accent, if t.dark { t.ink } else { t.ink }, 0.1);
    div()
        .id(id.into())
        .flex_none()
        .h(px(h))
        .px(px(12.))
        .gap(px(8.))
        .flex()
        .items_center()
        .rounded(px(R_CTRL))
        .bg(t.accent)
        .text_color(t.on_accent)
        .font_weight(FontWeight::BOLD)
        .text_size(px(13.))
        .cursor_pointer()
        .hover(move |s| s.bg(hov))
}

/// A square icon button (close, tear off).
pub fn icon_btn(t: &Theme, id: impl Into<gpui::ElementId>, ic: &str, size: f32) -> gpui::Stateful<Div> {
    let hover = t.hover;
    let press = t.press;
    div()
        .id(id.into())
        .flex_none()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(R_ITEM))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(press))
        .child(icon(ic, 14., t.dim))
}

/// A Windows caption button (minimize, maximize or restore, close). Windows itself handles the click, Snap Layouts on
/// the maximize button and the tooltips, through the hit test `area`.
pub fn caption_btn(t: &Theme, id: &'static str, ic: &str, area: gpui::WindowControlArea, close: bool) -> gpui::Stateful<Div> {
    let hover = if close { gpui::rgb(0xC42B1C).into() } else { t.hover };
    let press = if close { gpui::rgb(0xB0281A).into() } else { t.press };
    let ink = t.ink2;
    div()
        .id(id)
        .flex_none()
        .w(px(46.))
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .window_control_area(area)
        .hover(move |s| s.bg(hover))
        .active(move |s| s.bg(press))
        .group(id)
        .child(icon(ic, 16., ink).group_hover(id, |s| if close { s.text_color(gpui::white()) } else { s }))
}

/// Text that may run long: one line, cut with an ellipsis.
pub fn line(text: impl Into<SharedString>) -> Div {
    div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(text.into())
}

/// Text that wraps as far as it needs, never cut.
pub fn wrap(text: impl Into<SharedString>) -> Div {
    div().min_w_0().child(text.into())
}

/// A segmented control button.
pub fn seg_btn(t: &Theme, id: impl Into<gpui::ElementId>, on: bool, disabled: bool) -> gpui::Stateful<Div> {
    let hover = t.hover;
    div()
        .id(id.into())
        .flex_none()
        .h(px(26.))
        .px(px(10.))
        .gap(px(6.))
        .flex()
        .items_center()
        .rounded(px(R_ITEM))
        .text_size(px(12.5))
        .when(on, |d| d.bg(t.panel).shadow(vec![Theme::ring(t.line2)]).text_color(t.ink).font_weight(FontWeight::BOLD))
        .when(!on && !disabled, |d| d.text_color(t.ink2).cursor_pointer().hover(move |s| s.bg(hover)))
        .when(disabled, |d| d.text_color(t.faint).opacity(0.6))
}

pub fn seg_box(t: &Theme) -> Div {
    div().flex_none().flex().items_center().gap(px(2.)).p(px(2.)).rounded(px(R_CTRL)).bg(t.press)
}

/// Splits a sentence of runs into text plus bold ranges for StyledText.
pub fn runs_text(runs: &[sb_core::TextRun], bold_color: Hsla) -> gpui::StyledText {
    let mut text = String::new();
    let mut hl = Vec::new();
    for r in runs {
        let start = text.len();
        text.push_str(&r.text);
        if r.bold {
            hl.push((start..text.len(), gpui::HighlightStyle { font_weight: Some(FontWeight::BOLD), color: Some(bold_color), ..Default::default() }));
        }
    }
    gpui::StyledText::new(text).with_highlights(hl)
}

pub fn spacer() -> Div {
    div().flex_1().min_w_0()
}

/// One wrapping line of text in more than one font and color: (text, font family, color) parts.
pub fn mixed(parts: &[(&str, &'static str, Hsla)]) -> gpui::StyledText {
    let mut text = String::new();
    let mut runs = Vec::new();
    for (s, family, color) in parts {
        if s.is_empty() {
            continue;
        }
        text.push_str(s);
        runs.push(gpui::TextRun { len: s.len(), font: gpui::font(*family), color: *color, background_color: None, underline: None, strikethrough: None });
    }
    gpui::StyledText::new(text).with_runs(runs)
}

/// An on/off switch (Settings: Auto for a machine, each of its tools). `dim` when what it belongs to is off.
pub fn switch(t: &Theme, id: impl Into<gpui::ElementId>, on: bool, dim: bool) -> gpui::Stateful<Div> {
    let knob = div().size(px(16.)).rounded_full().bg(gpui::white()).shadow(vec![gpui::BoxShadow {
        color: gpui::black().opacity(0.25),
        offset: gpui::point(px(0.), px(1.)),
        blur_radius: px(2.),
        spread_radius: px(0.),
        inset: false,
    }]);
    div()
        .id(id.into())
        .flex_none()
        .w(px(34.))
        .h(px(20.))
        .p(px(2.))
        .flex()
        .items_center()
        .rounded_full()
        .cursor_pointer()
        .bg(if on { t.accent } else { t.track })
        .when(dim, |d| d.opacity(0.45))
        .when(on, |d| d.justify_end())
        .child(knob)
}

/// An option that is on or off: a check box, an icon and its name (the New sheet's options, a chat's tools).
/// `locked`: on, and not this place's to change.
pub fn check_chip(t: &Theme, id: impl Into<gpui::ElementId>, on: bool, locked: bool, ic: &str, label: &str) -> gpui::Stateful<Div> {
    let (hover, accent, line2, on_accent) = (t.hover, t.accent, t.line2, t.on_accent);
    div()
        .id(id.into())
        .flex_none()
        .h(px(30.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(7.))
        .rounded(px(R_CTRL))
        .border_1()
        .border_color(if on { accent.alpha(0.6) } else { t.line })
        .when(on, |d| d.bg(accent.alpha(if t.dark { 0.16 } else { 0.08 })))
        .when(!locked, |d| d.cursor_pointer())
        .when(!on && !locked, |d| d.hover(move |s| s.bg(hover)))
        .when(locked, |d| d.opacity(0.7))
        .text_size(px(12.5))
        .text_color(if on { t.ink } else { t.ink2 })
        .child(
            div()
                .flex_none()
                .size(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(R_MIN))
                .when(on, |d| d.bg(accent))
                .when(!on, |d| d.shadow(vec![Theme::ring(line2)]))
                .when(on, |d| d.child(icon(if locked { "lock" } else { "check" }, 10., on_accent))),
        )
        .child(icon(ic, 13., if on { accent } else { t.dim }))
        .child(label.to_string())
}

/// A library tool by name, as a quiet chip with the tool mark.
pub fn tool_chip(t: &Theme, name: &str) -> Div {
    div()
        .flex_none()
        .h(px(22.))
        .pl(px(6.))
        .pr(px(8.))
        .flex()
        .items_center()
        .gap(px(5.))
        .rounded(px(R_CHIP))
        .bg(t.chip_bg)
        .text_size(px(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.chip_ink)
        .child(icon("cube", 12., t.dim))
        .child(name.to_string())
}

/// Tabular figures in the UI font, so numbers that change keep their width without going monospace.
pub fn tnum() -> gpui::FontFeatures {
    gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}

/// A small tooltip, as `.tooltip(ui::tip("Settings"))` takes it.
pub fn tip(text: &'static str) -> impl Fn(&mut gpui::Window, &mut gpui::App) -> gpui::AnyView + 'static {
    move |_, cx| cx.new(|_| Tip(text)).into()
}

pub struct Tip(&'static str);

impl gpui::Render for Tip {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        let t = crate::theme::Theme::get(cx);
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(t.panel)
            .border_1()
            .border_color(t.line)
            .shadow(t.drop())
            .text_size(px(12.))
            .text_color(t.ink2)
            .child(self.0)
    }
}
