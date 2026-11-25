//! A pairing link as a QR code (Settings > Devices, Add a device). The `qrcode` crate works out which modules are
//! dark; the app draws them itself, as plain rectangles on a white tile. The tile is white in the dark theme too:
//! a camera reads dark modules on a light ground, and the quiet zone around the code has to be that same light.

use gpui::{AnyElement, Bounds, IntoElement, ParentElement, Styled, canvas, div, fill, point, prelude::*, px, size};

use crate::theme::Theme;
use crate::ui;

/// Light modules on every side of the code, as the standard asks.
pub const QUIET: usize = 4;

/// Which modules are dark, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Qr {
    pub side: usize,
    dark: Vec<bool>,
}

impl Qr {
    /// The code for `text`. None when it is empty or too long for a QR code. Medium error correction, or low when
    /// that is what it takes for a long link to fit.
    pub fn of(text: &str) -> Option<Qr> {
        use qrcode::{Color, EcLevel, QrCode};
        if text.is_empty() {
            return None;
        }
        let code = QrCode::with_error_correction_level(text, EcLevel::M)
            .or_else(|_| QrCode::with_error_correction_level(text, EcLevel::L))
            .ok()?;
        let side = code.width();
        let dark: Vec<bool> = code.to_colors().into_iter().map(|c| c == Color::Dark).collect();
        (dark.len() == side * side).then_some(Qr { side, dark })
    }

    pub fn dark(&self, col: usize, row: usize) -> bool {
        col < self.side && row < self.side && self.dark[row * self.side + col]
    }

    /// The dark modules as runs along each row, (row, first column, length): a few hundred rectangles to draw
    /// where there are a couple of thousand modules.
    pub fn runs(&self) -> Vec<(usize, usize, usize)> {
        let mut out = Vec::new();
        for row in 0..self.side {
            let mut col = 0;
            while col < self.side {
                if !self.dark(col, row) {
                    col += 1;
                    continue;
                }
                let start = col;
                while col < self.side && self.dark(col, row) {
                    col += 1;
                }
                out.push((row, start, col - start));
            }
        }
        out
    }
}

/// One module's edge in device pixels for a tile `tile` points wide at `scale`: whole pixels, so every module is
/// the same size and none is blurred, and never less than one. The code and its quiet zone always fit the tile.
pub fn module_px(side: usize, tile: f32, scale: f32) -> f32 {
    ((tile * scale) / (side + 2 * QUIET) as f32).floor().max(1.)
}

/// The white tile with the code in its middle, `tile` points on a side.
pub fn tile(t: &Theme, qr: &Qr, tile: f32) -> AnyElement {
    let runs = qr.runs();
    let side = qr.side;
    div()
        .flex_none()
        .size(px(tile))
        .rounded(px(ui::R_CTRL))
        .bg(gpui::white())
        // On the light theme the white tile needs an edge; on the dark one it is its own edge.
        .when(!t.dark, |d| d.border_1().border_color(t.line2))
        .overflow_hidden()
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let scale = window.scale_factor();
                    let m = module_px(side, f32::from(bounds.size.width), scale);
                    let whole = m * side as f32;
                    // The code's corner, on a whole device pixel, with what is left over shared out as quiet zone.
                    let x0 = ((f32::from(bounds.origin.x) * scale)
                        + ((f32::from(bounds.size.width) * scale - whole) / 2.))
                        .round();
                    let y0 = ((f32::from(bounds.origin.y) * scale)
                        + ((f32::from(bounds.size.height) * scale - whole) / 2.))
                        .round();
                    let ink = gpui::black();
                    for (row, col, len) in &runs {
                        let b = Bounds {
                            origin: point(px((x0 + *col as f32 * m) / scale), px((y0 + *row as f32 * m) / scale)),
                            size: size(px(*len as f32 * m / scale), px(m / scale)),
                        };
                        window.paint_quad(fill(b, ink));
                    }
                },
            )
            .size_full(),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LINK: &str = "switchboard://pair/1?h=3f9a1c0e5b7d2a4c6e8f0a1b3c5d7e9f1a2b3c4d5e6f708192a3b4c5d6e7f809&r=https%3A%2F%2Frelay.example&s=5d0c1b2a39485766&n=Studio";

    #[test]
    fn a_link_becomes_a_square_code_with_its_three_finders() {
        let qr = Qr::of(LINK).expect("a pairing link fits");
        assert!(
            qr.side >= 21 && (qr.side - 17).is_multiple_of(4),
            "a QR code's side is 17 + 4 x its version: {}",
            qr.side
        );
        // Each finder pattern: a dark 7 x 7 ring with a light ring inside and a dark 3 x 3 middle.
        for (cx, cy) in [(0, 0), (qr.side - 7, 0), (0, qr.side - 7)] {
            for i in 0..7 {
                assert!(
                    qr.dark(cx + i, cy) && qr.dark(cx + i, cy + 6) && qr.dark(cx, cy + i) && qr.dark(cx + 6, cy + i)
                );
            }
            assert!(!qr.dark(cx + 1, cy + 1) && !qr.dark(cx + 5, cy + 5));
            assert!(qr.dark(cx + 3, cy + 3));
        }
        assert!(!qr.dark(qr.side, 0) && !qr.dark(0, qr.side), "outside the code is light");
    }

    #[test]
    fn runs_cover_exactly_the_dark_modules() {
        let qr = Qr::of(LINK).unwrap();
        let mut seen = vec![false; qr.side * qr.side];
        for (row, col, len) in qr.runs() {
            assert!(len > 0 && col + len <= qr.side);
            for c in col..col + len {
                assert!(!seen[row * qr.side + c], "no module is drawn twice");
                seen[row * qr.side + c] = true;
            }
            // A run ends where the dark ends.
            assert!(col == 0 || !qr.dark(col - 1, row));
            assert!(!qr.dark(col + len, row));
        }
        for row in 0..qr.side {
            for col in 0..qr.side {
                assert_eq!(seen[row * qr.side + col], qr.dark(col, row));
            }
        }
        assert!(qr.runs().len() < qr.side * qr.side / 3, "far fewer rectangles than modules");
    }

    #[test]
    fn the_same_link_gives_the_same_code_and_nothing_gives_none() {
        assert_eq!(Qr::of(LINK), Qr::of(LINK));
        assert_ne!(Qr::of(LINK), Qr::of(&format!("{LINK}x")));
        assert_eq!(Qr::of(""), None);
        assert_eq!(Qr::of(&"x".repeat(8000)), None, "too long for any QR code");
        // The longest link the app takes still fits, at the lower correction level if it has to.
        assert!(Qr::of(&format!("switchboard://pair/1?h={}", "a".repeat(1000))).is_some());
    }

    #[test]
    fn modules_are_whole_pixels_and_the_quiet_zone_always_fits() {
        for side in [21usize, 37, 49, 77, 177] {
            for scale in [1.0f32, 1.25, 1.5, 2.0, 3.0] {
                let m = module_px(side, 176., scale);
                assert_eq!(m, m.floor());
                assert!(m >= 1.);
                if m > 1. {
                    assert!(m * (side + 2 * QUIET) as f32 <= 176. * scale, "side {side} at {scale}x");
                }
            }
        }
        assert_eq!(module_px(49, 176., 2.), 6.);
    }
}
