//! Drawing the terminal grid with GPUI: backgrounds merged per row, text shaped per word with every glyph pinned to
//! its cell, wide characters on two cells, box drawing and block characters drawn as shapes (so Claude Code's
//! boxes join up whatever the font), real bold and italic faces, every underline style (straight, double, curly,
//! dotted, dashed, in its own color), strikethrough, the selection, the cursor and IME text being composed.
//!
//! The frame is built while the tab is borrowed, then painted, because painting text needs the app mutably.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{self, TermMode};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::vte::ansi::CursorShape;
use gpui::{
    App, BorderStyle, Bounds, ContentMask, Font, FontFallbacks, FontFeatures, FontId, FontStyle, FontWeight, Hsla, Path, PathBuilder, Pixels,
    Point, ShapedLine, SharedString, TextAlign, TextRun, Window, bounds, fill, outline, point, px, size,
};

use super::colors::Palette;
use super::{Phase, TermTab};
use crate::theme::mix;

pub const PAD_X: f32 = 14.;
pub const PAD_Y: f32 = 10.;

/// The terminal font: the platform's own terminal face (it has real bold and italic, and its letters fill the cell
/// the way they do in Terminal or iTerm2), then fallbacks for what it lacks (Claude Code's ⏺ ⎿ ✻ ❯ ⏵, arrows, CJK,
/// emoji). Box drawing and block characters never reach the font.
pub fn font() -> Font {
    let (family, fallbacks): (&str, &[&str]) = if cfg!(target_os = "macos") {
        ("Menlo", &["Apple Symbols", "Apple Color Emoji"])
    } else if cfg!(target_os = "windows") {
        ("Cascadia Mono", &["Consolas", "Segoe UI Symbol", "Segoe UI Emoji"])
    } else {
        ("DejaVu Sans Mono", &["Noto Sans Mono", "Noto Sans Symbols 2", "Noto Sans Symbols", "Noto Color Emoji"])
    };
    let fallbacks: Vec<String> = fallbacks.iter().map(|f| f.to_string()).collect();
    Font {
        family: family.into(),
        // Every ligature off: a terminal pins one glyph to each cell.
        features: FontFeatures(std::sync::Arc::new(["calt", "liga", "clig", "dlig", "rlig"].iter().map(|t| (t.to_string(), 0)).collect())),
        fallbacks: Some(FontFallbacks::from_fonts(fallbacks)),
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

/// Cell geometry, snapped to device pixels so block characters and box lines meet without seams.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub font_id: FontId,
    pub size: f32,
    pub cell_w: f32,
    pub line_h: f32,
    pub scale: f32,
}

impl Metrics {
    /// At `size` points (the synced terminal text size, 13 by default).
    pub fn new(window: &Window, size: f32) -> Metrics {
        let ts = window.text_system();
        let font_id = ts.resolve_font(&font());
        let scale = window.scale_factor();
        let adv = ts.advance(font_id, px(size), 'm').map(|s| f32::from(s.width)).unwrap_or(size * 0.55);
        let snap = |v: f32| (v * scale).round().max(1.) / scale;
        Metrics { font_id, size, cell_w: snap(adv), line_h: snap(size * 1.36), scale }
    }

    pub fn snap(&self, v: f32) -> f32 {
        (v * self.scale).round() / self.scale
    }

    /// Columns and rows that fit in `b` (at least 20 by 5).
    pub fn grid_for(&self, b: Bounds<Pixels>) -> (usize, usize) {
        let w = f32::from(b.size.width) - PAD_X * 2.;
        let h = f32::from(b.size.height) - PAD_Y * 2.;
        (((w / self.cell_w).floor() as usize).max(20), ((h / self.line_h).floor() as usize).max(5))
    }

    /// Where the first cell starts in `b`.
    pub fn origin(&self, b: Bounds<Pixels>) -> Point<Pixels> {
        point(px(self.snap(f32::from(b.origin.x) + PAD_X)), px(self.snap(f32::from(b.origin.y) + PAD_Y)))
    }

    /// Light line thickness for box drawing: about a font stroke.
    fn light(&self) -> f32 {
        ((self.size / 13.) * self.scale).round().max(1.) / self.scale
    }
}

/// Everything one frame paints, in order.
#[derive(Default)]
pub struct Frame {
    bgs: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    shapes: Vec<(Bounds<Pixels>, Hsla)>,
    paths: Vec<(Path<Pixels>, Hsla)>,
    cursor: Vec<(Bounds<Pixels>, Hsla)>,
    hollow: Vec<(Bounds<Pixels>, Hsla)>,
    top: Vec<(Point<Pixels>, ShapedLine)>,
    clip: Option<Bounds<Pixels>>,
    line_h: f32,
}

/// One cell's look after inverse, dim, hidden and the selection.
#[derive(Clone, Copy, PartialEq)]
struct Look {
    fg: Hsla,
    bg: Hsla,
    bold: bool,
    italic: bool,
    strike: bool,
    hidden: bool,
}

/// The underline styles SGR 4 and 4:x ask for.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Under {
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

fn under_of(flags: Flags) -> Option<Under> {
    if flags.contains(Flags::UNDERCURL) {
        Some(Under::Curly)
    } else if flags.contains(Flags::DOTTED_UNDERLINE) {
        Some(Under::Dotted)
    } else if flags.contains(Flags::DASHED_UNDERLINE) {
        Some(Under::Dashed)
    } else if flags.contains(Flags::DOUBLE_UNDERLINE) {
        Some(Under::Double)
    } else if flags.contains(Flags::UNDERLINE) {
        Some(Under::Single)
    } else {
        None
    }
}

/// A run of text being collected for one shaped line.
struct Seg {
    col: usize,
    text: String,
    runs: Vec<TextRun>,
}

struct Rows<'a> {
    m: &'a Metrics,
    origin: Point<Pixels>,
    font: Font,
    frame: Frame,
    seg: Option<Seg>,
    bg_run: Option<(usize, usize, usize, Hsla)>,
}

impl<'a> Rows<'a> {
    fn cell(&self, line: usize, col: usize, cells: usize) -> Bounds<Pixels> {
        bounds(
            point(self.origin.x + px(col as f32 * self.m.cell_w), self.origin.y + px(line as f32 * self.m.line_h)),
            size(px(cells as f32 * self.m.cell_w), px(self.m.line_h)),
        )
    }

    fn flush_bg(&mut self) {
        if let Some((line, c0, c1, color)) = self.bg_run.take() {
            let b = self.cell(line, c0, c1 - c0);
            self.frame.bgs.push((b, color));
        }
    }

    fn bg(&mut self, line: usize, col: usize, cells: usize, color: Hsla) {
        if let Some((l, _, c1, c)) = &mut self.bg_run {
            if *l == line && *c1 == col && *c == color {
                *c1 = col + cells;
                return;
            }
        }
        self.flush_bg();
        self.bg_run = Some((line, col, col + cells, color));
    }

    fn flush_seg(&mut self, window: &Window, line: usize) {
        let Some(seg) = self.seg.take() else { return };
        if seg.text.is_empty() {
            return;
        }
        let at = point(self.origin.x + px(seg.col as f32 * self.m.cell_w), self.origin.y + px(line as f32 * self.m.line_h));
        let text = SharedString::from(seg.text);
        let shaped = window.text_system().shape_line(text.clone(), px(self.m.size), &seg.runs, Some(px(self.m.cell_w)));
        if shaped.runs.iter().map(|r| r.glyphs.len()).sum::<usize>() != text.len() {
            // The font joined characters anyway (a ligature the features did not turn off): place each on its own cell.
            let mut i = 0;
            for r in &seg.runs {
                for k in 0..r.len {
                    let at = point(at.x + px((i + k) as f32 * self.m.cell_w), at.y);
                    let one = TextRun { len: 1, ..r.clone() };
                    let shaped = window.text_system().shape_line(SharedString::from(text[i + k..i + k + 1].to_string()), px(self.m.size), &[one], None);
                    self.frame.lines.push((at, shaped));
                }
                i += r.len;
            }
            return;
        }
        self.frame.lines.push((at, shaped));
    }

    fn push_text(&mut self, window: &Window, line: usize, col: usize, s: &str, look: &Look) {
        let joins = self.seg.as_ref().is_some_and(|g| g.col + g.text.chars().count() == col);
        if !joins {
            self.flush_seg(window, line);
            self.seg = Some(Seg { col, text: String::new(), runs: Vec::new() });
        }
        let font = self.styled(look);
        let seg = self.seg.as_mut().expect("segment");
        seg.text.push_str(s);
        match seg.runs.last_mut() {
            Some(r) if r.color == look.fg && r.font.weight == font.weight && r.font.style == font.style => r.len += s.len(),
            _ => seg.runs.push(TextRun { len: s.len(), font, color: look.fg, background_color: None, underline: None, strikethrough: None }),
        }
    }

    /// The face for a cell: bold and italic are the font's own.
    fn styled(&self, look: &Look) -> Font {
        let mut f = self.font.clone();
        if look.bold {
            f.weight = FontWeight::BOLD;
        }
        if look.italic {
            f.style = FontStyle::Italic;
        }
        f
    }

    /// A character on its own (wide, a symbol from a fallback font, or with combining marks).
    fn single(&mut self, window: &Window, line: usize, col: usize, s: &str, look: &Look) {
        self.flush_seg(window, line);
        let run = TextRun { len: s.len(), font: self.styled(look), color: look.fg, background_color: None, underline: None, strikethrough: None };
        let at = point(self.origin.x + px(col as f32 * self.m.cell_w), self.origin.y + px(line as f32 * self.m.line_h));
        let shaped = window.text_system().shape_line(SharedString::from(s.to_string()), px(self.m.size), std::slice::from_ref(&run), None);
        self.frame.lines.push((at, shaped));
    }
}

/// Builds the frame for `t` laid out from `origin`.
/// How the cursor looks: the user's style (used while the program leaves the shape alone) and whether the blink has it
/// showing right now.
#[derive(Clone, Copy)]
pub struct CursorLook {
    pub shape: CursorShape,
    pub visible: bool,
}

pub fn build(t: &TermTab, m: &Metrics, pal: &Palette, look: CursorLook, clip: Bounds<Pixels>, window: &Window) -> Frame {
    let origin = m.origin(clip);
    let content = t.term.renderable_content();
    let offset = content.display_offset;
    let colors = content.colors;
    let sel = content.selection;
    let mode = content.mode;
    let mut cursor = content.cursor;
    // A program that sets its own shape (vim's bar in insert mode) wins; the default block takes the user's style.
    if cursor.shape == CursorShape::Block {
        cursor.shape = look.shape;
    }
    let cols = t.term.columns();
    let mut rows = Rows { m, origin, font: font(), frame: Frame { clip: Some(clip), line_h: m.line_h, ..Default::default() }, seg: None, bg_run: None };
    let mut cur_line = usize::MAX;
    let mut s = String::new();
    for cell in content.display_iter {
        let p = cell.point;
        let Some(vp) = term::point_to_viewport(offset, p) else { continue };
        let (line, col) = (vp.line, vp.column.0);
        if col >= cols {
            continue;
        }
        if line != cur_line {
            if cur_line != usize::MAX {
                rows.flush_seg(window, cur_line);
            }
            rows.flush_bg();
            cur_line = line;
        }
        let flags = cell.flags;
        if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let wide = flags.contains(Flags::WIDE_CHAR);
        let cells = if wide { 2 } else { 1 };
        let mut fg = pal.resolve(cell.fg, colors);
        let mut bg = pal.resolve(cell.bg, colors);
        if flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
        }
        if flags.contains(Flags::DIM) {
            fg = mix(fg, bg, 0.42);
        }
        let selected = sel.is_some_and(|r| r.contains(p));
        if selected {
            bg = mix(bg, pal.sel, 0.85);
        }
        let look = Look {
            fg,
            bg,
            bold: flags.contains(Flags::BOLD),
            italic: flags.contains(Flags::ITALIC),
            strike: flags.contains(Flags::STRIKEOUT),
            hidden: flags.contains(Flags::HIDDEN),
        };
        if bg != pal.bg {
            rows.bg(line, col, cells, bg);
        } else {
            rows.flush_bg();
        }
        let r = rows.cell(line, col, cells);
        // The hovered link is underlined; a link the program set (OSC 8) has a faint dotted line until then.
        let hovered = t.hover.as_ref().is_some_and(|(l, f)| *l == line && col >= f.start && col < f.end);
        let (u, faint) = match under_of(flags) {
            Some(u) => (Some(u), false),
            None if hovered => (Some(Under::Single), false),
            None if cell.hyperlink().is_some() => (Some(Under::Dotted), true),
            None => (None, false),
        };
        if let Some(u) = u.filter(|_| !look.hidden) {
            let color = cell.underline_color().map(|c| pal.resolve(c, colors)).unwrap_or(if faint { mix(fg, bg, 0.5) } else { fg });
            underline(if hovered { Under::Single } else { u }, r, color, m, &mut rows.frame);
        }
        if look.strike {
            let y = m.snap(f32::from(r.origin.y) + m.line_h * 0.52);
            rows.frame.shapes.push((bounds(point(r.origin.x, px(y)), size(r.size.width, px(m.light()))), fg));
        }
        let c = cell.c;
        if look.hidden || c == ' ' || c == '\t' || c == '\0' {
            rows.flush_seg(window, line);
            continue;
        }
        if !wide && draw_box(c, r, fg, bg, m, &mut rows.frame) {
            rows.flush_seg(window, line);
            continue;
        }
        s.clear();
        s.push(c);
        let zw = cell.zerowidth();
        if let Some(z) = zw {
            s.extend(z.iter());
        }
        if c.is_ascii() && zw.is_none() {
            rows.push_text(window, line, col, &s, &look);
        } else {
            rows.single(window, line, col, &s, &look);
        }
    }
    if cur_line != usize::MAX {
        rows.flush_seg(window, cur_line);
    }
    rows.flush_bg();

    // The cursor: a block when focused (the character redrawn in the background color), an outline when not.
    let show = t.phase == Phase::Open && cursor.shape != CursorShape::Hidden && mode.contains(TermMode::SHOW_CURSOR) && (look.visible || !t.focused);
    if show {
        if let Some(vp) = term::point_to_viewport(offset, cursor.point) {
            let cc = &t.term.grid()[cursor.point];
            let cells = if cc.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
            let r = rows.cell(vp.line, vp.column.0, cells);
            let color = pal.index(258, colors);
            let th = m.snap(2.0_f32.max(m.light()));
            match (t.focused, cursor.shape) {
                (true, CursorShape::Beam) => rows.frame.cursor.push((bounds(r.origin, size(px(th), r.size.height)), color)),
                (true, CursorShape::Underline) => rows
                    .frame
                    .cursor
                    .push((bounds(point(r.origin.x, r.origin.y + r.size.height - px(th)), size(r.size.width, px(th))), color)),
                (true, _) => {
                    rows.frame.cursor.push((r, color));
                    if cc.c != ' ' && cc.c != '\0' && !cc.flags.contains(Flags::HIDDEN) {
                        let mut txt = cc.c.to_string();
                        if let Some(z) = cc.zerowidth() {
                            txt.extend(z.iter());
                        }
                        let run = TextRun { len: txt.len(), font: rows.font.clone(), color: pal.bg, background_color: None, underline: None, strikethrough: None };
                        let shaped = window.text_system().shape_line(SharedString::from(txt), px(m.size), &[run], None);
                        rows.frame.top.push((r.origin, shaped));
                    }
                }
                (false, _) => rows.frame.hollow.push((r, color)),
            }
            // Text being composed in an input method sits on the cursor, underlined.
            if let Some(marked) = t.marked.as_ref().filter(|s| !s.is_empty()) {
                let run = TextRun { len: marked.len(), font: rows.font.clone(), color: pal.fg, background_color: None, underline: None, strikethrough: None };
                let shaped = window.text_system().shape_line(SharedString::from(marked.clone()), px(m.size), &[run], None);
                let w = shaped.width().max(px(m.cell_w));
                rows.frame.cursor.push((bounds(r.origin, size(w, r.size.height)), pal.bg));
                rows.frame.cursor.push((bounds(point(r.origin.x, r.origin.y + r.size.height - px(m.light())), size(w, px(m.light()))), pal.fg));
                rows.frame.top.push((r.origin, shaped));
            }
        }
    }
    rows.frame
}

pub fn paint(frame: Frame, window: &mut Window, cx: &mut App) {
    let mask = frame.clip.map(|bounds| ContentMask { bounds });
    let lh = px(frame.line_h);
    window.with_content_mask(mask, |window| {
        for (b, c) in frame.bgs {
            window.paint_quad(fill(b, c));
        }
        for (at, l) in &frame.lines {
            let _ = l.paint(*at, lh, TextAlign::Left, None, window, cx);
        }
        for (b, c) in frame.shapes {
            window.paint_quad(fill(b, c));
        }
        for (p, c) in frame.paths {
            window.paint_path(p, c);
        }
        for (b, c) in frame.cursor {
            window.paint_quad(fill(b, c));
        }
        for (b, c) in frame.hollow {
            window.paint_quad(outline(b, c, BorderStyle::Solid));
        }
        for (at, l) in &frame.top {
            let _ = l.paint(*at, lh, TextAlign::Left, None, window, cx);
        }
    });
}

/// An underline under one cell. Neighbouring cells line up: the dots and dashes keep to a grid of their own, and
/// the curl is one whole wave per cell, so a word's curl is one smooth line.
fn underline(u: Under, r: Bounds<Pixels>, color: Hsla, m: &Metrics, f: &mut Frame) {
    let th = m.light();
    let (x, w) = (f32::from(r.origin.x), f32::from(r.size.width));
    let base = m.snap(f32::from(r.origin.y) + m.line_h - m.line_h * 0.14);
    let line = |x0: f32, x1: f32, y: f32, f: &mut Frame| f.shapes.push((bounds(point(px(x0), px(y)), size(px(x1 - x0), px(th))), color));
    match u {
        Under::Single => line(x, x + w, base, f),
        Under::Double => {
            line(x, x + w, base, f);
            line(x, x + w, m.snap(base - th * 2.), f);
        }
        Under::Dotted => {
            let step = m.snap((th * 2.).max(2.));
            let mut d = m.snap(x / step).ceil() * step;  // on a grid of its own, so the dots never bunch at a cell edge
            while d + th <= x + w + 0.01 {
                line(d, d + th, base, f);
                d += step;
            }
        }
        Under::Dashed => {
            let (a, gap) = (m.snap(w * 0.6), w - m.snap(w * 0.6));
            line(x + gap / 2., x + gap / 2. + a, base, f);
        }
        Under::Curly => {
            let amp = m.snap((m.line_h * 0.07).max(1.5));
            let mid = base - amp / 2.;
            let mut b = PathBuilder::stroke(px(th));
            b.move_to(point(px(x), px(mid)));
            b.curve_to(point(px(x + w / 2.), px(mid)), point(px(x + w / 4.), px(mid - amp)));
            b.curve_to(point(px(x + w), px(mid)), point(px(x + w * 0.75), px(mid + amp)));
            if let Ok(p) = b.build() {
                f.paths.push((p, color));
            }
        }
    }
}

// ---------- box drawing and blocks ----------

/// Arms of U+2500..U+257F as up, right, down, left: 0 none, 1 light, 2 heavy, 3 double. "x" marks the dashed,
/// rounded and diagonal ones, drawn separately.
const ARMS: [&str; 128] = [
    "0101", "0202", "1010", "2020", "x", "x", "x", "x", "x", "x", "x", "x", "0110", "0210", "0120", "0220", // 2500
    "0011", "0012", "0021", "0022", "1100", "1200", "2100", "2200", "1001", "1002", "2001", "2002", "1110", "1210", "2110", "1120", // 2510
    "2120", "2210", "1220", "2220", "1011", "1012", "2011", "1021", "2021", "2012", "1022", "2022", "0111", "0112", "0211", "0212", // 2520
    "0121", "0122", "0221", "0222", "1101", "1102", "1201", "1202", "2101", "2102", "2201", "2202", "1111", "1112", "1211", "1212", // 2530
    "2111", "1121", "2121", "2112", "2211", "1122", "1221", "2212", "1222", "2122", "2221", "2222", "x", "x", "x", "x", // 2540
    "0303", "3030", "0310", "0130", "0330", "0013", "0031", "0033", "1300", "3100", "3300", "1003", "3001", "3003", "1310", "3130", // 2550
    "3330", "1013", "3031", "3033", "0313", "0131", "0333", "1303", "3101", "3303", "1313", "3131", "3333", "x", "x", "x", // 2560
    "x", "x", "x", "x", "0001", "1000", "0100", "0010", "0002", "2000", "0200", "0020", "0201", "1020", "0102", "2010", // 2570
];

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds<Pixels> {
    bounds(point(px(x0.min(x1)), px(y0.min(y1))), size(px((x1 - x0).abs()), px((y1 - y0).abs())))
}

/// Draws `c` into cell `r` when it is box drawing or a block element. False means "let the font draw it".
fn draw_box(c: char, r: Bounds<Pixels>, fg: Hsla, bg: Hsla, m: &Metrics, f: &mut Frame) -> bool {
    let u = c as u32;
    if !(0x2500..=0x259F).contains(&u) {
        return false;
    }
    let (x, y) = (f32::from(r.origin.x), f32::from(r.origin.y));
    let (w, h) = (f32::from(r.size.width), f32::from(r.size.height));
    let (x1, y1) = (x + w, y + h);
    if u >= 0x2580 {
        let q = |a: bool, b: bool, c: bool, d: bool, f: &mut Frame| {
            let (mx, my) = (m.snap(x + w / 2.), m.snap(y + h / 2.));
            if a {
                f.shapes.push((rect(x, y, mx, my), fg));
            }
            if b {
                f.shapes.push((rect(mx, y, x1, my), fg));
            }
            if c {
                f.shapes.push((rect(x, my, mx, y1), fg));
            }
            if d {
                f.shapes.push((rect(mx, my, x1, y1), fg));
            }
        };
        let lower = |n: f32, f: &mut Frame| f.shapes.push((rect(x, m.snap(y1 - h * n / 8.), x1, y1), fg));
        let left = |n: f32, f: &mut Frame| f.shapes.push((rect(x, y, m.snap(x + w * n / 8.), y1), fg));
        match u {
            0x2580 => f.shapes.push((rect(x, y, x1, m.snap(y + h / 2.)), fg)),
            0x2581..=0x2588 => lower((u - 0x2580) as f32, f),
            0x2589..=0x258F => left((0x2590 - u) as f32, f),
            0x2590 => f.shapes.push((rect(m.snap(x + w / 2.), y, x1, y1), fg)),
            0x2591 => f.shapes.push((r, mix(bg, fg, 0.25))),
            0x2592 => f.shapes.push((r, mix(bg, fg, 0.5))),
            0x2593 => f.shapes.push((r, mix(bg, fg, 0.75))),
            0x2594 => f.shapes.push((rect(x, y, x1, m.snap(y + h / 8.)), fg)),
            0x2595 => f.shapes.push((rect(m.snap(x1 - w / 8.), y, x1, y1), fg)),
            0x2596 => q(false, false, true, false, f),
            0x2597 => q(false, false, false, true, f),
            0x2598 => q(true, false, false, false, f),
            0x2599 => q(true, false, true, true, f),
            0x259A => q(true, false, false, true, f),
            0x259B => q(true, true, true, false, f),
            0x259C => q(true, true, false, true, f),
            0x259D => q(false, true, false, false, f),
            0x259E => q(false, true, true, false, f),
            0x259F => q(false, true, true, true, f),
            _ => return false,
        }
        return true;
    }

    let lt = m.light();
    let thick = |wgt: u8| match wgt {
        2 => lt * 2.,
        _ => lt,
    };
    let cx = m.snap(x + w / 2. - lt / 2.) + lt / 2.;
    let cy = m.snap(y + h / 2. - lt / 2.) + lt / 2.;
    let code = ARMS[(u - 0x2500) as usize];
    if code == "x" {
        return special(u, x, y, w, h, cx, cy, lt, fg, f);
    }
    let a: Vec<u8> = code.bytes().map(|b| b - b'0').collect();
    let (up, right, down, left) = (a[0], a[1], a[2], a[3]);
    let g = lt; // half the gap between double lines, to their centers
    let doubles = a.iter().filter(|v| **v == 3).count();
    let two = a.iter().filter(|v| **v > 0).count() == 2;
    // A corner of two double lines: the outer and inner lines meet like a frame.
    if doubles == 2 && two && (up == 3 || down == 3) && (left == 3 || right == 3) {
        let sh = if right == 3 { 1. } else { -1. };
        let sv = if down == 3 { 1. } else { -1. };
        let hx = |from: f32| if sh > 0. { (from - lt / 2., x1) } else { (x, from + lt / 2.) };
        let vy = |from: f32| if sv > 0. { (from - lt / 2., y1) } else { (y, from + lt / 2.) };
        for k in [-1., 1.] {
            let (a0, a1) = hx(cx + k * sh * g);
            let yy = cy + k * sv * g;
            f.shapes.push((rect(a0, yy - lt / 2., a1, yy + lt / 2.), fg));
            let (b0, b1) = vy(cy + k * sv * g);
            let xx = cx + k * sh * g;
            f.shapes.push((rect(xx - lt / 2., b0, xx + lt / 2., b1), fg));
        }
        return true;
    }
    let vmax = [up, down].iter().map(|v| if *v == 3 { lt * 3. } else if *v > 0 { thick(*v) } else { 0. }).fold(0., f32::max);
    let hmax = [left, right].iter().map(|v| if *v == 3 { lt * 3. } else if *v > 0 { thick(*v) } else { 0. }).fold(0., f32::max);
    // Horizontal arms.
    for (wgt, dir) in [(right, 1.), (left, -1.)] {
        if wgt == 0 {
            continue;
        }
        let (a0, a1) = if dir > 0. { (cx - vmax / 2., x1) } else { (x, cx + vmax / 2.) };
        if wgt == 3 {
            for k in [-1., 1.] {
                let yy = cy + k * g;
                f.shapes.push((rect(a0, yy - lt / 2., a1, yy + lt / 2.), fg));
            }
        } else {
            let th = thick(wgt);
            f.shapes.push((rect(a0, cy - th / 2., a1, cy + th / 2.), fg));
        }
    }
    // Vertical arms.
    for (wgt, dir) in [(down, 1.), (up, -1.)] {
        if wgt == 0 {
            continue;
        }
        let (b0, b1) = if dir > 0. { (cy - hmax / 2., y1) } else { (y, cy + hmax / 2.) };
        if wgt == 3 {
            for k in [-1., 1.] {
                let xx = cx + k * g;
                f.shapes.push((rect(xx - lt / 2., b0, xx + lt / 2., b1), fg));
            }
        } else {
            let th = thick(wgt);
            f.shapes.push((rect(cx - th / 2., b0, cx + th / 2., b1), fg));
        }
    }
    true
}

/// Dashed lines, rounded corners and diagonals.
#[allow(clippy::too_many_arguments)]
fn special(u: u32, x: f32, y: f32, w: f32, h: f32, cx: f32, cy: f32, lt: f32, fg: Hsla, f: &mut Frame) -> bool {
    let (x1, y1) = (x + w, y + h);
    let dashes = |n: usize, heavy: bool, vertical: bool, f: &mut Frame| {
        let th = if heavy { lt * 2. } else { lt };
        let len = if vertical { h } else { w };
        let step = len / n as f32;
        let gap = step * 0.35;
        for i in 0..n {
            let s0 = i as f32 * step + gap / 2.;
            let s1 = s0 + step - gap;
            if vertical {
                f.shapes.push((rect(cx - th / 2., y + s0, cx + th / 2., y + s1), fg));
            } else {
                f.shapes.push((rect(x + s0, cy - th / 2., x + s1, cy + th / 2.), fg));
            }
        }
    };
    match u {
        0x2504 => dashes(3, false, false, f),
        0x2505 => dashes(3, true, false, f),
        0x2506 => dashes(3, false, true, f),
        0x2507 => dashes(3, true, true, f),
        0x2508 => dashes(4, false, false, f),
        0x2509 => dashes(4, true, false, f),
        0x250A => dashes(4, false, true, f),
        0x250B => dashes(4, true, true, f),
        0x254C => dashes(2, false, false, f),
        0x254D => dashes(2, true, false, f),
        0x254E => dashes(2, false, true, f),
        0x254F => dashes(2, true, true, f),
        0x256D..=0x2570 => {
            // Rounded corners: a straight arm into a quarter curve. ╭ down+right, ╮ down+left, ╯ up+left, ╰ up+right.
            let (down, right) = match u {
                0x256D => (true, true),
                0x256E => (true, false),
                0x256F => (false, false),
                _ => (false, true),
            };
            let r = (w / 2.).min(h / 2.);
            let sv = if down { 1. } else { -1. };
            let sh = if right { 1. } else { -1. };
            let mut b = PathBuilder::stroke(px(lt));
            b.move_to(point(px(cx), px(if down { y1 } else { y })));
            b.line_to(point(px(cx), px(cy + sv * r)));
            b.curve_to(point(px(cx + sh * r), px(cy)), point(px(cx), px(cy)));
            b.line_to(point(px(if right { x1 } else { x }), px(cy)));
            match b.build() {
                Ok(p) => f.paths.push((p, fg)),
                Err(_) => return false,
            }
        }
        0x2571..=0x2573 => {
            let mut lines = Vec::new();
            if u != 0x2572 {
                lines.push(((x, y1), (x1, y)));
            }
            if u != 0x2571 {
                lines.push(((x, y), (x1, y1)));
            }
            for (a, z) in lines {
                let mut b = PathBuilder::stroke(px(lt));
                b.move_to(point(px(a.0), px(a.1)));
                b.line_to(point(px(z.0), px(z.1)));
                if let Ok(p) = b.build() {
                    f.paths.push((p, fg));
                }
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::ARMS;

    #[test]
    fn arms_table_is_whole() {
        assert_eq!(ARMS.len(), 128);
        assert!(ARMS.iter().all(|a| *a == "x" || (a.len() == 4 && a.bytes().all(|b| (b'0'..=b'3').contains(&b)))));
        // Spot checks against the Unicode names.
        assert_eq!(ARMS[0x0C], "0110", "┌ down and right");
        assert_eq!(ARMS[0x3C], "1111", "┼");
        assert_eq!(ARMS[0x54], "0330", "╔ double down and right");
        assert_eq!(ARMS[0x6C], "3333", "╬");
        assert_eq!(ARMS[0x7F], "2010", "╿ heavy up, light down");
    }
}
