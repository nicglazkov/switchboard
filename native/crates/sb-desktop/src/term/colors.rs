//! Terminal color schemes: the 16 ANSI colors, the 256-color cube and grays, and the default foreground, background
//! and cursor. The terminal keeps its own scheme apart from the app's light or dark look, because Claude Code paints
//! for a dark terminal by default. Programs that set colors (OSC 4/10/11/12) win.

use alacritty_terminal::term::color::{COUNT, Colors};
use alacritty_terminal::vte::ansi::{Color, Rgb};
use gpui::{App, Hsla, Rgba};

use crate::theme::{Theme, mix};

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub fg: Hsla,
    pub bg: Hsla,
    pub cursor: Hsla,
    pub sel: Hsla,
    pub ansi: [Hsla; 16],
}

/// The shared list (sb-core's schemes.json, the same file the server and the phone read).
pub use sb_core::term::scheme::{self, MATCH, Scheme, TermLook};

fn hex(s: &str) -> Hsla {
    let (r, g, b, a) = scheme::hex(s).unwrap_or((128, 128, 128, 255));
    Rgba { r: r as f32 / 255., g: g as f32 / 255., b: b as f32 / 255., a: a as f32 / 255. }.into()
}

/// A shared scheme as a palette to paint with.
pub fn palette_of(s: &Scheme) -> Palette {
    let (bg, cursor) = (hex(&s.bg), hex(&s.cursor));
    let mut ansi = [bg; 16];
    for (i, c) in s.ansi.iter().take(16).enumerate() {
        ansi[i] = hex(c);
    }
    let sel = s.sel.as_deref().map(hex).unwrap_or_else(|| mix(bg, cursor, if s.dark { 0.34 } else { 0.2 }));
    Palette { dark: s.dark, fg: hex(&s.fg), bg, cursor, sel, ansi }
}

/// While the pointer rests on a scheme in the picker, every terminal shows it (Esc or leaving puts it back).
#[derive(Default)]
pub struct Preview(pub Option<String>);
impl gpui::Global for Preview {}

pub fn set_preview(id: Option<String>, cx: &mut App) {
    if cx.try_global::<Preview>().map(|p| p.0.as_ref()) != Some(id.as_ref()) {
        cx.set_global(Preview(id));
        cx.refresh_windows();
    }
}

/// The saved terminal look (scheme normalized; synced through /api/prefs, cached in settings.json).
pub fn look(cx: &App) -> TermLook {
    let s = &crate::state::app(cx).read(cx).settings;
    TermLook {
        scheme: scheme::normalize(s.term_colors.as_deref()),
        size: s.term_size,
        size_phone: None,
        cursor: s.term_cursor.clone().unwrap_or_else(|| "block".into()),
        blink: s.term_blink.unwrap_or(true),
    }
}

/// The scheme id the settings ask for (the default when unset or unknown), or MATCH.
pub fn chosen(cx: &App) -> String {
    look(cx).scheme
}

/// The scheme the terminals paint with now: the one being previewed, else the chosen one.
pub fn shown(cx: &App) -> String {
    cx.try_global::<Preview>().and_then(|p| p.0.clone()).unwrap_or_else(|| chosen(cx))
}

/// The palette every terminal tab paints with right now.
pub fn current(cx: &App) -> Palette {
    match shown(cx).as_str() {
        MATCH => palette(&Theme::get(cx)),
        id => palette_of(scheme::by_id(id).unwrap_or(&scheme::all()[0])),
    }
}

/// The name a choice shows.
pub fn name_of(id: &str) -> String {
    if id == MATCH {
        "Match the app".into()
    } else {
        scheme::by_id(id).map(|s| s.name.clone()).unwrap_or_else(|| id.to_string())
    }
}

/// The app's own look as a terminal palette (the "Match the app" choice).
pub fn palette(t: &Theme) -> Palette {
    palette_of(scheme::matching(t.dark))
}

/// The terminal text size in points (Cmd+Plus and Cmd+Minus change it).
pub fn font_size(cx: &App) -> f32 {
    look(cx).size.unwrap_or(scheme::SIZE_DEFAULT) as f32
}

fn from_rgb(c: Rgb) -> Hsla {
    Rgba { r: c.r as f32 / 255., g: c.g as f32 / 255., b: c.b as f32 / 255., a: 1. }.into()
}

pub fn to_rgb(c: Hsla) -> Rgb {
    let x = c.to_rgb();
    Rgb { r: (x.r * 255.).round() as u8, g: (x.g * 255.).round() as u8, b: (x.b * 255.).round() as u8 }
}

/// xterm's 256 colors past the first 16: a 6x6x6 cube, then 24 grays.
fn indexed(i: u8) -> Rgb {
    if i >= 232 {
        let v = 8 + 10 * (i - 232);
        return Rgb { r: v, g: v, b: v };
    }
    let i = i - 16;
    let step = |n: u8| if n == 0 { 0 } else { 55 + 40 * n };
    Rgb { r: step(i / 36), g: step((i / 6) % 6), b: step(i % 6) }
}

impl Palette {
    /// The color for index `i` of the terminal's color table (0..=255, then 256 foreground, 257 background,
    /// 258 cursor, the dim colors, bright and dim foreground).
    pub fn index(&self, i: usize, colors: &Colors) -> Hsla {
        if i < COUNT
            && let Some(c) = colors[i]
        {
            return from_rgb(c);
        }
        match i {
            0..=15 => self.ansi[i],
            16..=255 => from_rgb(indexed(i as u8)),
            256 => self.fg,
            257 => self.bg,
            258 => self.cursor,
            259..=266 => mix(self.ansi[i - 259], self.bg, 0.35),
            268 => mix(self.fg, self.bg, 0.35),
            _ => self.fg,
        }
    }

    pub fn resolve(&self, color: Color, colors: &Colors) -> Hsla {
        match color {
            Color::Spec(rgb) => from_rgb(rgb),
            Color::Indexed(i) => self.index(i as usize, colors),
            Color::Named(n) => self.index(n as usize, colors),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_grays() {
        assert_eq!(indexed(16), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(indexed(196), Rgb { r: 255, g: 0, b: 0 });
        assert_eq!(indexed(231), Rgb { r: 255, g: 255, b: 255 });
        assert_eq!(indexed(232), Rgb { r: 8, g: 8, b: 8 });
        assert_eq!(indexed(255), Rgb { r: 238, g: 238, b: 238 });
    }

    #[test]
    fn every_shared_scheme_paints() {
        for s in scheme::all() {
            let p = palette_of(s);
            assert_eq!(p.dark, s.dark);
            assert_ne!(p.fg, p.bg, "{}", s.id);
        }
        assert!(scheme::by_id(scheme::DEFAULT).is_some_and(|s| s.dark), "Claude Code paints for a dark terminal");
    }
}
