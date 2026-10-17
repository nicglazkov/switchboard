//! The Lanes tokens, for dark and light.
//! State colors keep their meaning everywhere: need (pink), working (cyan), warm (green), cold (amber), offline (red).

use gpui::{App, BoxShadow, Global, Hsla, Rgba, point, px, rgb, rgba};
use sb_core::{Lamp, MachineLamp};

pub const UI: &str = "Instrument Sans";
pub const NUM: &str = "JetBrains Mono";
pub const BRAND: &str = "Manrope";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    System,
    Dark,
    Light,
}

impl Mode {
    pub fn parse(s: Option<&str>) -> Mode {
        match s {
            Some("dark") => Mode::Dark,
            Some("light") => Mode::Light,
            _ => Mode::System,
        }
    }
    pub fn name(self) -> Option<&'static str> {
        match self {
            Mode::Dark => Some("dark"),
            Mode::Light => Some("light"),
            Mode::System => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub bg: Hsla,
    pub bg2: Hsla,
    pub panel: Hsla,
    pub panel2: Hsla,
    pub panel3: Hsla,
    pub line: Hsla,
    pub line2: Hsla,
    pub rule: Hsla,
    pub track: Hsla,
    pub fill: Hsla,
    pub ink: Hsla,
    pub ink2: Hsla,
    pub label: Hsla,
    pub dim: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub run: Hsla,
    pub warm: Hsla,
    pub cold: Hsla,
    pub need: Hsla,
    pub bad: Hsla,
    pub idle: Hsla,
    pub stall: Hsla,
    pub sel: Hsla,
    pub press: Hsla,
    pub hover: Hsla,
    pub chip_bg: Hsla,
    pub chip_ink: Hsla,
    pub scrim: Hsla,
    pub shadow: Hsla,
    shadow_y: f32,
    shadow_blur: f32,
}

impl Global for Theme {}

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}
fn ca(hex: u32) -> Hsla {
    rgba(hex).into()
}

impl Theme {
    pub fn dark() -> Theme {
        Theme {
            dark: true,
            bg: c(0x121416),
            bg2: c(0x121416),
            panel: c(0x1D2125),
            panel2: c(0x1D2125),
            panel3: c(0x24292E),
            line: c(0x262A2F),
            line2: c(0x353A40),
            rule: c(0x262A2F),
            track: c(0x2C3136),
            fill: c(0xB8BDC3),
            ink: c(0xEDEFF1),
            ink2: c(0xB8BDC3),
            label: c(0xB8BDC3),
            dim: c(0x9AA0A7),
            faint: c(0x858B92),
            accent: c(0x3CC7C2),
            on_accent: c(0x072624),
            run: c(0x6EA4FF),
            warm: c(0x5FD08B),
            cold: c(0xE9B04F),
            need: c(0xFF6B9A),
            bad: c(0xF2675F),
            idle: c(0x5A6067),
            stall: c(0x858B92),
            sel: ca(0x3CC7C221),
            press: ca(0xEDEFF112),
            hover: ca(0xEDEFF10F),
            chip_bg: c(0x24292E),
            chip_ink: c(0xB8BDC3),
            scrim: ca(0x00000080),
            shadow: ca(0x0000008C),
            shadow_y: 20.,
            shadow_blur: 56.,
        }
    }

    pub fn light() -> Theme {
        Theme {
            dark: false,
            bg: c(0xF4F5F5),
            bg2: c(0xF4F5F5),
            panel: c(0xFFFFFF),
            panel2: c(0xFFFFFF),
            panel3: c(0xEEF0F1),
            line: c(0xE3E5E7),
            line2: c(0xD0D4D8),
            rule: c(0xE3E5E7),
            track: c(0xE3E5E7),
            fill: c(0x454B52),
            ink: c(0x15191C),
            ink2: c(0x454B52),
            label: c(0x454B52),
            dim: c(0x6A7078),
            faint: c(0x7B8189),
            accent: c(0x0C7480),
            on_accent: c(0xFFFFFF),
            run: c(0x2F6FDB),
            warm: c(0x2B8F52),
            cold: c(0xA86A12),
            need: c(0xCF2F66),
            bad: c(0xC8322B),
            idle: c(0xA3A8AE),
            stall: c(0x7B8189),
            sel: ca(0x0C74801A),
            press: ca(0x15191C0F),
            hover: ca(0x15191C0D),
            chip_bg: c(0xEEF0F1),
            chip_ink: c(0x454B52),
            scrim: ca(0x15191C42),
            shadow: ca(0x15191C2E),
            shadow_y: 18.,
            shadow_blur: 50.,
        }
    }

    pub fn get(cx: &App) -> Theme {
        *cx.global::<Theme>()
    }

    /// Installs the theme and hands the same colors to gpui-base's inputs.
    pub fn install(self, cx: &mut App) {
        let t = self;
        cx.set_global(t);
        let tok = &mut gpui_base::Theme::global_mut(cx).tokens.colors;
        tok.background = t.bg2;
        tok.foreground = t.ink;
        tok.surface = t.panel;
        tok.surface_foreground = t.ink;
        tok.primary = t.accent;
        tok.primary_foreground = t.on_accent;
        tok.muted = t.panel3;
        tok.muted_foreground = t.faint;
        tok.accent = t.accent;
        tok.accent_foreground = t.on_accent;
        tok.border = t.line;
        tok.input = t.line;
        tok.ring = t.accent;
        tok.selection = t.accent.alpha(if t.dark { 0.32 } else { 0.22 });
    }

    pub fn lamp(&self, l: Lamp) -> Hsla {
        match l {
            Lamp::Held | Lamp::Wait => self.need,
            Lamp::Run => self.run,
            Lamp::Stall => self.stall,
            Lamp::Warm => self.warm,
            Lamp::Cold => self.cold,
            Lamp::Idle => self.idle,
        }
    }

    pub fn machine(&self, l: MachineLamp) -> Hsla {
        match l {
            MachineLamp::On => self.warm,
            MachineLamp::Off => self.bad,
            MachineLamp::Warn => self.cold,
        }
    }

    /// The soft drop shadow under popovers, sheets and the floating inspector.
    pub fn drop(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: self.shadow,
            offset: point(px(0.), px(self.shadow_y)),
            blur_radius: px(self.shadow_blur),
            spread_radius: px(0.),
            inset: false,
        }]
    }

    /// A 1px ring drawn inside the box (no layout shift).
    pub fn ring(color: Hsla) -> BoxShadow {
        BoxShadow { color, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: true }
    }

    /// The keyboard focus ring: 2 px of the accent just outside the control, so it never covers the label.
    pub fn focus_ring(accent: Hsla) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: accent.alpha(0.75),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(2.),
            inset: false,
        }]
    }
}

/// Saves the app's look (System follows the window's appearance) and repaints every window.
pub fn set_mode(mode: Mode, window: &gpui::Window, cx: &mut App) {
    save_look(cx, |s| s.theme = mode.name().map(str::to_string));
    let dark = match mode {
        Mode::Dark => true,
        Mode::Light => false,
        Mode::System => {
            matches!(window.appearance(), gpui::WindowAppearance::Dark | gpui::WindowAppearance::VibrantDark)
        }
    };
    (if dark { Theme::dark() } else { Theme::light() }).install(cx);
    cx.refresh_windows();
}

/// Changes the saved look (theme or terminal), syncs it to the server and repaints every window.
pub fn save_look(cx: &mut App, f: impl FnOnce(&mut crate::state::Settings)) {
    crate::state::app(cx).update(cx, |s, cx| {
        f(&mut s.settings);
        s.save_settings();
        s.sync_appearance(cx);
    });
    cx.refresh_windows();
}

/// Saves the terminal's color scheme and repaints every window.
pub fn set_term_colors(id: &str, cx: &mut App) {
    let id = id.to_string();
    save_look(cx, |s| s.term_colors = Some(id));
}

/// `a` moved toward `b` by `t`, in RGB.
pub fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    Rgba { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
        .into()
}
