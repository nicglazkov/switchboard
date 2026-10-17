//! The app's Lanes tokens, so the installer looks like the app. Copied from native/crates/sb-desktop/src/theme.rs
//! (only the tokens the installer draws with); when the app's tokens change, change these to match.

use gpui::{BoxShadow, Hsla, Rgba, point, px, rgb, rgba};

pub const UI: &str = "Instrument Sans";
pub const NUM: &str = "JetBrains Mono";
pub const BRAND: &str = "Manrope";

/// Corner radii, from sb-desktop/src/ui.rs.
pub const R_CARD: f32 = 10.;
pub const R_CTRL: f32 = 7.;

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub dark: bool,
    pub bg: Hsla,
    pub panel: Hsla,
    pub panel3: Hsla,
    pub line: Hsla,
    pub line2: Hsla,
    pub track: Hsla,
    pub ink: Hsla,
    pub ink2: Hsla,
    pub dim: Hsla,
    pub faint: Hsla,
    pub accent: Hsla,
    pub on_accent: Hsla,
    pub warm: Hsla,
    pub cold: Hsla,
    pub bad: Hsla,
    pub press: Hsla,
    pub hover: Hsla,
    pub shadow: Hsla,
    shadow_y: f32,
    shadow_blur: f32,
}

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
            panel: c(0x1D2125),
            panel3: c(0x24292E),
            line: c(0x262A2F),
            line2: c(0x353A40),
            track: c(0x2C3136),
            ink: c(0xEDEFF1),
            ink2: c(0xB8BDC3),
            dim: c(0x9AA0A7),
            faint: c(0x858B92),
            accent: c(0x3CC7C2),
            on_accent: c(0x072624),
            warm: c(0x5FD08B),
            cold: c(0xE9B04F),
            bad: c(0xF2675F),
            press: ca(0xEDEFF112),
            hover: ca(0xEDEFF10F),
            shadow: ca(0x0000008C),
            shadow_y: 20.,
            shadow_blur: 56.,
        }
    }

    pub fn light() -> Theme {
        Theme {
            dark: false,
            bg: c(0xF4F5F5),
            panel: c(0xFFFFFF),
            panel3: c(0xEEF0F1),
            line: c(0xE3E5E7),
            line2: c(0xD0D4D8),
            track: c(0xE3E5E7),
            ink: c(0x15191C),
            ink2: c(0x454B52),
            dim: c(0x6A7078),
            faint: c(0x7B8189),
            accent: c(0x0C7480),
            on_accent: c(0xFFFFFF),
            warm: c(0x2B8F52),
            cold: c(0xA86A12),
            bad: c(0xC8322B),
            press: ca(0x15191C0F),
            hover: ca(0x15191C0D),
            shadow: ca(0x15191C2E),
            shadow_y: 18.,
            shadow_blur: 50.,
        }
    }

    /// The soft drop shadow under cards that float (the tour's screenshots).
    pub fn drop(&self) -> Vec<BoxShadow> {
        vec![BoxShadow {
            color: self.shadow.opacity(0.6),
            offset: point(px(0.), px(self.shadow_y * 0.6)),
            blur_radius: px(self.shadow_blur * 0.7),
            spread_radius: px(0.),
            inset: false,
        }]
    }

    /// A 1px ring drawn inside the box (no layout shift).
    pub fn ring(color: Hsla) -> BoxShadow {
        BoxShadow { color, offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(1.), inset: true }
    }

    /// The mark's link and node colors, from sb-desktop/src/ui.rs `mark`.
    pub fn mark_colors(&self) -> (Hsla, Hsla) {
        if self.dark { (c(0x5C79C9), c(0x7E8AD2)) } else { (c(0x7E95D6), c(0x5F6CB8)) }
    }
}

/// `a` moved toward `b` by `t`, in RGB.
pub fn mix(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b) = (a.to_rgb(), b.to_rgb());
    Rgba { r: a.r + (b.r - a.r) * t, g: a.g + (b.g - a.g) * t, b: a.b + (b.b - a.b) * t, a: a.a + (b.a - a.a) * t }
        .into()
}
