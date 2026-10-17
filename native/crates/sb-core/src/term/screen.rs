//! A terminal's screen as bytes (spec 2026-10-14, terminal pipeline). The hub keeps a headless grid per terminal and
//! sends a viewer that joins, reconnects or asks for a redraw this snapshot first, so it has the picture at once,
//! with no resize nudge. The desktop app uses the same writer to carry its own scrollback across a reconnect.
//!
//! A snapshot is meant for a fresh terminal of the same size: the scrollback (up to `history` lines) and the screen,
//! cell by cell with their colors, attributes and links, soft wraps kept so a later resize reflows the same way;
//! the primary screen under an alternate one; the saved and the live cursor with its pen; the modes a program set;
//! the cursor style and palette changes; and a synchronized update still waiting for its end.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::{Dimensions, Grid};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags, Hyperlink};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, StdSyncHandler};

/// Scrollback lines a snapshot carries at most.
pub const HISTORY: usize = 1000;

/// Attributes that change how a cell is drawn (not the layout flags: wraps and wide-character halves).
const LOOK: Flags = Flags::INVERSE
    .union(Flags::BOLD)
    .union(Flags::ITALIC)
    .union(Flags::ALL_UNDERLINES)
    .union(Flags::DIM)
    .union(Flags::HIDDEN)
    .union(Flags::STRIKEOUT);

/// The pen the writer has set, so SGR goes out only where it changes.
#[derive(Default)]
struct Pen {
    look: Option<(Color, Color, Flags, Option<Color>)>,
    link: Option<Hyperlink>,
}

fn default_look() -> (Color, Color, Flags, Option<Color>) {
    (Color::Named(NamedColor::Foreground), Color::Named(NamedColor::Background), Flags::empty(), None)
}

fn look_of(c: &Cell) -> (Color, Color, Flags, Option<Color>) {
    (c.fg, c.bg, c.flags & LOOK, c.underline_color())
}

fn color(out: &mut String, c: Color, fg: bool) {
    use std::fmt::Write;
    let base = if fg { 30 } else { 40 };
    match c {
        Color::Named(n) => {
            let i = n as usize;
            if i < 8 {
                let _ = write!(out, ";{}", base + i);
            } else if i < 16 {
                let _ = write!(out, ";{}", base + 60 + i - 8);
            } // the default and the dim or bright defaults: already 39 or 49 after the reset
        }
        Color::Indexed(i) => {
            let _ = write!(out, ";{};5;{i}", base + 8);
        }
        Color::Spec(rgb) => {
            let _ = write!(out, ";{};2;{};{};{}", base + 8, rgb.r, rgb.g, rgb.b);
        }
    }
}

/// SGR for a look, from a reset: "\x1b[0;1;38;5;3m".
fn sgr(l: &(Color, Color, Flags, Option<Color>)) -> String {
    use std::fmt::Write;
    let (fg, bg, f, ul) = l;
    let mut s = String::from("\x1b[0");
    for (flag, code) in [
        (Flags::BOLD, "1"),
        (Flags::DIM, "2"),
        (Flags::ITALIC, "3"),
        (Flags::INVERSE, "7"),
        (Flags::HIDDEN, "8"),
        (Flags::STRIKEOUT, "9"),
    ] {
        if f.contains(flag) {
            s.push(';');
            s.push_str(code);
        }
    }
    if f.contains(Flags::DOUBLE_UNDERLINE) {
        s.push_str(";21");
    } else if f.contains(Flags::UNDERCURL) {
        s.push_str(";4:3");
    } else if f.contains(Flags::DOTTED_UNDERLINE) {
        s.push_str(";4:4");
    } else if f.contains(Flags::DASHED_UNDERLINE) {
        s.push_str(";4:5");
    } else if f.contains(Flags::UNDERLINE) {
        s.push_str(";4");
    }
    color(&mut s, *fg, true);
    color(&mut s, *bg, false);
    match ul {
        Some(Color::Indexed(i)) => {
            let _ = write!(s, ";58;5;{i}");
        }
        Some(Color::Spec(rgb)) => {
            let _ = write!(s, ";58;2;{};{};{}", rgb.r, rgb.g, rgb.b);
        }
        Some(Color::Named(n)) if (*n as usize) < 16 => {
            let _ = write!(s, ";58;5;{}", *n as usize);
        }
        _ => {}
    }
    s.push('m');
    s
}

impl Pen {
    fn set(&mut self, out: &mut Vec<u8>, cell: &Cell) {
        let look = look_of(cell);
        if self.look.as_ref() != Some(&look) {
            out.extend_from_slice(sgr(&look).as_bytes());
            self.look = Some(look);
        }
        let link = cell.hyperlink();
        if link != self.link {
            match &link {
                Some(h) => out.extend_from_slice(format!("\x1b]8;id={};{}\x1b\\", h.id(), h.uri()).as_bytes()),
                None => out.extend_from_slice(b"\x1b]8;;\x1b\\"),
            }
            self.link = link;
        }
    }

    /// Back to the default pen and no link, before a line feed (a new line takes the pen's background).
    fn plain(&mut self, out: &mut Vec<u8>) {
        if self.look.is_some_and(|l| l != default_look()) {
            out.extend_from_slice(b"\x1b[0m");
        }
        self.look = Some(default_look());
        if self.link.take().is_some() {
            out.extend_from_slice(b"\x1b]8;;\x1b\\");
        }
    }
}

fn blank(c: &Cell) -> bool {
    c.c == ' ' && look_of(c) == default_look() && c.hyperlink().is_none() && c.zerowidth().is_none()
}

fn push_cell(out: &mut Vec<u8>, cell: &Cell) {
    let mut b = [0u8; 4];
    out.extend_from_slice(cell.c.encode_utf8(&mut b).as_bytes());
    if let Some(zw) = cell.zerowidth() {
        for z in zw {
            out.extend_from_slice(z.encode_utf8(&mut b).as_bytes());
        }
    }
}

/// Where a row's content ends (cells after it are default blanks), and whether it wraps softly into the next.
fn extent(grid: &Grid<Cell>, line: Line, cols: usize) -> (usize, bool) {
    let row = &grid[line];
    let wrapped = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
    let end = (0..cols).rev().find(|&c| !blank(&row[Column(c)])).map_or(0, |c| c + 1);
    (end, wrapped)
}

/// Writes rows `from..to` of `grid` from the cursor on, joined by line feeds or soft wraps. Returns nothing; the
/// cursor ends on the last row written.
fn rows(out: &mut Vec<u8>, grid: &Grid<Cell>, from: i32, to: i32, pen: &mut Pen) {
    let cols = grid.columns();
    let lines: Vec<(Line, usize, bool)> = (from..to)
        .map(|l| {
            let (end, wrapped) = extent(grid, Line(l), cols);
            (Line(l), end, wrapped)
        })
        .collect();
    for (i, &(line, end, wrapped)) in lines.iter().enumerate() {
        let next_empty = lines.get(i + 1).is_none_or(|n| n.1 == 0);
        // a soft wrap is kept by writing the whole row and letting the next character wrap it
        let soft = wrapped && !next_empty;
        let end = if soft { cols } else { end };
        let row = &grid[line];
        for c in 0..end {
            let cell = &row[Column(c)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue; // the wide character takes both cells, or wraps by itself
            }
            pen.set(out, cell);
            push_cell(out, cell);
        }
        if i + 1 < lines.len() && !soft {
            pen.plain(out);
            out.extend_from_slice(b"\r\n");
        }
    }
}

fn cup(out: &mut Vec<u8>, line: i32, col: usize) {
    out.extend_from_slice(format!("\x1b[{};{}H", line + 1, col + 1).as_bytes());
}

/// The cursor (and the one saved with ESC 7) where the grid has them, with their pens.
fn cursor(out: &mut Vec<u8>, grid: &Grid<Cell>, origin_mode: bool, pen: &mut Pen) {
    let saved = &grid.saved_cursor;
    if saved.point.line.0 != 0 || saved.point.column.0 != 0 || look_of(&saved.template) != default_look() {
        cup(out, saved.point.line.0, saved.point.column.0);
        out.extend_from_slice(sgr(&look_of(&saved.template)).as_bytes());
        out.extend_from_slice(b"\x1b7");
        pen.look = Some(look_of(&saved.template));
    }
    if origin_mode {
        out.extend_from_slice(b"\x1b[?6h");
    }
    let c = &grid.cursor;
    let cols = grid.columns();
    if c.input_needs_wrap && c.point.column.0 + 1 >= cols {
        // the last character went into the last column and the next one wraps: write that cell again to get there
        let row = &grid[c.point.line];
        let mut col = cols - 1;
        if row[Column(col)].flags.contains(Flags::WIDE_CHAR_SPACER) && col > 0 {
            col -= 1;
        }
        cup(out, c.point.line.0, col);
        let cell = &row[Column(col)];
        pen.set(out, cell);
        push_cell(out, cell);
    } else {
        cup(out, c.point.line.0, c.point.column.0);
    }
    pen.look = None;
    out.extend_from_slice(sgr(&look_of(&c.template)).as_bytes());
    let link = c.template.hyperlink();
    match (&link, &pen.link) {
        (Some(h), _) => out.extend_from_slice(format!("\x1b]8;id={};{}\x1b\\", h.id(), h.uri()).as_bytes()),
        (None, Some(_)) => out.extend_from_slice(b"\x1b]8;;\x1b\\"),
        (None, None) => {}
    }
}

/// The modes that differ from a fresh terminal's, set or reset.
fn modes(out: &mut Vec<u8>, mode: TermMode) {
    let fresh = TermMode::default();
    let private = [
        (TermMode::SHOW_CURSOR, 25),
        (TermMode::APP_CURSOR, 1),
        (TermMode::MOUSE_REPORT_CLICK, 1000),
        (TermMode::MOUSE_DRAG, 1002),
        (TermMode::MOUSE_MOTION, 1003),
        (TermMode::FOCUS_IN_OUT, 1004),
        (TermMode::UTF8_MOUSE, 1005),
        (TermMode::SGR_MOUSE, 1006),
        (TermMode::ALTERNATE_SCROLL, 1007),
        (TermMode::URGENCY_HINTS, 1042),
        (TermMode::BRACKETED_PASTE, 2004),
        (TermMode::LINE_WRAP, 7),
    ];
    for (m, n) in private {
        if mode.contains(m) != fresh.contains(m) {
            out.extend_from_slice(format!("\x1b[?{n}{}", if mode.contains(m) { 'h' } else { 'l' }).as_bytes());
        }
    }
    if mode.contains(TermMode::APP_KEYPAD) {
        out.extend_from_slice(b"\x1b=");
    }
    if mode.contains(TermMode::INSERT) {
        out.extend_from_slice(b"\x1b[4h");
    }
    if mode.contains(TermMode::LINE_FEED_NEW_LINE) {
        out.extend_from_slice(b"\x1b[20h");
    }
}

/// One grid: its last `history` scrollback lines and the screen, then its cursors.
fn grid_bytes(out: &mut Vec<u8>, grid: &Grid<Cell>, history: usize, origin_mode: bool) {
    let mut pen = Pen::default();
    let hist = history.min(grid.history_size()) as i32;
    out.extend_from_slice(b"\x1b[H");
    rows(out, grid, -hist, grid.screen_lines() as i32, &mut pen);
    cursor(out, grid, origin_mode, &mut pen);
}

/// The snapshot of `term` for a fresh terminal of its size, with up to `history` lines of scrollback. `sync` is what
/// the parser holds of a synchronized update that has not ended (see `Screen`). `term` is left as it was.
pub fn snapshot<T: EventListener>(term: &mut Term<T>, history: usize, sync: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(64 * 1024);
    let mode = *term.mode();
    let origin = mode.contains(TermMode::ORIGIN);
    if mode.contains(TermMode::ALT_SCREEN) {
        // The primary screen is under the alternate one: look at it (swap, write, swap back) and put back the
        // alternate grid, which swapping in clears.
        let alt = term.grid().clone();
        term.swap_alt();
        grid_bytes(&mut out, term.grid(), history, false);
        term.swap_alt();
        *term.grid_mut() = alt;
        out.extend_from_slice(b"\x1b[0m\x1b[?1049h");
        grid_bytes(&mut out, term.grid(), 0, origin);
    } else {
        grid_bytes(&mut out, term.grid(), history, origin);
    }
    modes(&mut out, mode);
    let style = term.cursor_style();
    let n = match (style.shape, style.blinking) {
        (CursorShape::Block, false) => 0, // the default: nothing to say
        (CursorShape::Block, true) => 1,
        (CursorShape::Underline, true) => 3,
        (CursorShape::Underline, false) => 4,
        (CursorShape::Beam, true) => 5,
        (CursorShape::Beam, false) => 6,
        _ => 0,
    };
    if n != 0 {
        out.extend_from_slice(format!("\x1b[{n} q").as_bytes());
    }
    let colors = term.colors();
    for i in 0..259 {
        if let Some(c) = colors[i] {
            let rgb = format!("rgb:{:02x}/{:02x}/{:02x}", c.r, c.g, c.b);
            let osc = match i {
                0..=255 => format!("\x1b]4;{i};{rgb}\x1b\\"),
                256 => format!("\x1b]10;{rgb}\x1b\\"),
                257 => format!("\x1b]11;{rgb}\x1b\\"),
                _ => format!("\x1b]12;{rgb}\x1b\\"),
            };
            out.extend_from_slice(osc.as_bytes());
        }
    }
    if !sync.is_empty() {
        out.extend_from_slice(b"\x1b[?2026h");
        out.extend_from_slice(sync);
    }
    out
}

/// Carries `old`'s scrollback into `fresh` (a new, empty terminal, any size): written in, rewrapped to the new width,
/// then scrolled up off the screen, so the screen is blank for a snapshot and the history is all there.
/// `old` is being replaced: it is left on its primary screen.
pub fn carry_history<T: EventListener, U: EventListener>(old: &mut Term<T>, fresh: &mut Term<U>) {
    if old.mode().contains(TermMode::ALT_SCREEN) {
        old.swap_alt();
    }
    let grid = old.grid();
    let hist = grid.history_size() as i32;
    if hist == 0 {
        return;
    }
    let mut out = Vec::new();
    let mut pen = Pen::default();
    rows(&mut out, grid, -hist, 0, &mut pen);
    pen.plain(&mut out);
    let mut parser: Processor<StdSyncHandler> = Processor::new();
    parser.advance(fresh, &out);
    let used = fresh.grid().cursor.point.line.0 + 1;
    let mut push = Vec::new();
    cup(&mut push, fresh.screen_lines() as i32 - 1, 0);
    push.extend(std::iter::repeat_n(b'\n', used.max(0) as usize));
    push.extend_from_slice(b"\x1b[H");
    parser.advance(fresh, &push);
}

/// A headless terminal: the hub's copy of a screen. It parses everything the program writes, keeps up to
/// `HISTORY` lines of scrollback, and remembers the raw bytes of a synchronized update still open, so a snapshot
/// taken in the middle of one replays it whole.
pub struct Screen<T: EventListener> {
    pub term: Term<T>,
    parser: Processor<StdSyncHandler>,
    /// The bytes the parser holds for the open synchronized update: always the tail of the stream.
    sync: Vec<u8>,
}

struct Size(usize, usize);

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.1
    }
    fn screen_lines(&self) -> usize {
        self.1
    }
    fn columns(&self) -> usize {
        self.0
    }
}

/// The size type alacritty wants, for callers that only have columns and rows.
pub fn dims(cols: u16, rows: u16) -> impl Dimensions {
    Size(cols.max(2) as usize, rows.max(1) as usize)
}

impl<T: EventListener> Screen<T> {
    pub fn new(cols: u16, rows: u16, history: usize, listener: T) -> Self {
        let config = alacritty_terminal::term::Config { scrolling_history: history, ..Default::default() };
        Screen { term: Term::new(config, &dims(cols, rows), listener), parser: Processor::new(), sync: Vec::new() }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        let held = self.parser.sync_bytes_count();
        if held == 0 {
            self.sync.clear();
        } else {
            self.sync.extend_from_slice(bytes);
            let cut = self.sync.len().saturating_sub(held);
            self.sync.drain(..cut);
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(dims(cols, rows));
    }

    /// When an open synchronized update times out (then `flush`).
    pub fn sync_deadline(&self) -> Option<std::time::Instant> {
        self.parser.sync_timeout().sync_timeout()
    }

    pub fn flush(&mut self) {
        self.parser.stop_sync(&mut self.term);
        self.sync.clear();
    }

    pub fn snapshot(&mut self, history: usize) -> Vec<u8> {
        snapshot(&mut self.term, history, &self.sync)
    }

    pub fn size(&self) -> (u16, u16) {
        (self.term.columns() as u16, self.term.screen_lines() as u16)
    }
}

/// Every cell of a screen (and `history` lines above it) as text with its look, for tests and the replay tool to
/// compare two grids.
pub fn dump<T>(term: &Term<T>, history: usize) -> Vec<String> {
    let grid = term.grid();
    let hist = history.min(grid.history_size()) as i32;
    let cols = grid.columns();
    (-hist..grid.screen_lines() as i32)
        .map(|l| {
            let row = &grid[Line(l)];
            let mut s = String::new();
            let mut last = None;
            for c in 0..cols {
                let cell = &row[Column(c)];
                let look = look_of(cell);
                if last.as_ref() != Some(&look) {
                    s.push_str(&format!("{{{:?}}}", look).replace(' ', ""));
                    last = Some(look);
                }
                s.push(cell.c);
                if let Some(zw) = cell.zerowidth() {
                    s.extend(zw);
                }
            }
            let wrap = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
            format!("{}{}", s.trim_end(), if wrap { "⏎" } else { "" })
        })
        .collect()
}

/// The cursor and the modes, as text, to compare beside `dump`.
pub fn state<T>(term: &Term<T>) -> String {
    let c = &term.grid().cursor;
    format!(
        "cursor {}:{} wrap={} mode={:?} style={:?}",
        c.point.line.0,
        c.point.column.0,
        c.input_needs_wrap,
        term.mode(),
        term.cursor_style()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;

    fn screen(cols: u16, rows: u16, bytes: &[u8]) -> Screen<VoidListener> {
        let mut s = Screen::new(cols, rows, 2000, VoidListener);
        s.feed(bytes);
        s
    }

    /// A snapshot fed to a fresh terminal gives the same cells, cursor and modes.
    fn round_trip(s: &mut Screen<VoidListener>, history: usize) {
        let (c, r) = s.size();
        let snap = s.snapshot(history);
        let mut copy = Screen::new(c, r, 2000, VoidListener);
        copy.feed(&snap);
        assert_eq!(dump(&copy.term, history), dump(&s.term, history), "cells");
        assert_eq!(state(&copy.term), state(&s.term), "cursor and modes");
        // and the snapshot changed nothing on the original
        let again = s.snapshot(history);
        assert_eq!(again, snap, "a snapshot is read-only");
    }

    #[test]
    fn plain_text_and_colors() {
        let mut s = screen(40, 8, b"hello \x1b[1;31mred bold\x1b[0m and \x1b[38;5;208morange\x1b[0m\r\n\x1b[48;2;10;20;30m bg \x1b[0m\r\n\x1b[3;4mital\x1b[0m");
        round_trip(&mut s, 0);
    }

    #[test]
    fn attributes_links_and_wide_characters() {
        let bytes = "\x1b[2mdim\x1b[22m \x1b[7minv\x1b[27m \x1b[9mstrike\x1b[29m \x1b[4:3;58;2;255;0;0mcurl\x1b[0m \x1b[21mdbl\x1b[0m\r\n\
                     \x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\ 漢字 e\u{301} 🙂 end"
            .as_bytes();
        let mut s = screen(30, 6, bytes);
        round_trip(&mut s, 0);
    }

    #[test]
    fn soft_wraps_survive_and_reflow_the_same() {
        let long = "0123456789".repeat(5);
        let mut s = screen(16, 6, format!("{long}\r\nshort\r\n{long}").as_bytes());
        round_trip(&mut s, 100);
        // after a resize both reflow the same way
        let snap = s.snapshot(100);
        let mut copy = Screen::new(16, 6, 2000, VoidListener);
        copy.feed(&snap);
        s.resize(25, 6);
        copy.resize(25, 6);
        assert_eq!(dump(&copy.term, 100), dump(&s.term, 100));
    }

    #[test]
    fn a_wide_character_at_the_edge_wraps_the_same() {
        let mut s = screen(9, 4, "abcdefgh漢x".as_bytes());
        round_trip(&mut s, 10);
    }

    #[test]
    fn scrollback_up_to_the_limit() {
        let mut bytes = Vec::new();
        for i in 0..50 {
            bytes.extend_from_slice(format!("line {i}\r\n").as_bytes());
        }
        let mut s = screen(20, 5, &bytes);
        round_trip(&mut s, 30);
        round_trip(&mut s, 1000);
        let snap = s.snapshot(30);
        let mut copy = Screen::new(20, 5, 2000, VoidListener);
        copy.feed(&snap);
        assert_eq!(copy.term.grid().history_size(), 30);
    }

    #[test]
    fn cursor_pen_and_saved_cursor() {
        // the pen set before the cursor moved is what the next character gets
        let mut s = screen(20, 6, b"abc\x1b[3;5H\x1b[32mX\x1b7\x1b[5;2H\x1b[1;44m");
        round_trip(&mut s, 0);
        let snap = s.snapshot(0);
        let mut copy = Screen::new(20, 6, 2000, VoidListener);
        copy.feed(&snap);
        copy.feed(b"Y\x1b8Z");
        s.feed(b"Y\x1b8Z");
        assert_eq!(dump(&copy.term, 0), dump(&s.term, 0));
    }

    #[test]
    fn pending_wrap_at_the_last_column() {
        let mut s = screen(10, 4, b"0123456789");
        round_trip(&mut s, 0);
        let snap = s.snapshot(0);
        let mut copy = Screen::new(10, 4, 2000, VoidListener);
        copy.feed(&snap);
        copy.feed(b"next");
        s.feed(b"next");
        assert_eq!(dump(&copy.term, 0), dump(&s.term, 0));
    }

    #[test]
    fn modes_and_cursor_style() {
        let mut s = screen(
            20,
            4,
            b"\x1b[?1h\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[?25l\x1b[5 q\x1b=\x1b]4;1;rgb:12/34/56\x1b\\hi",
        );
        round_trip(&mut s, 0);
        assert!(s.term.mode().contains(TermMode::BRACKETED_PASTE));
    }

    #[test]
    fn the_alternate_screen_and_the_primary_under_it() {
        let mut s = screen(20, 5, b"primary 1\r\nprimary 2\r\n\x1b[?1049h\x1b[2;3Halt text\x1b[31m");
        round_trip(&mut s, 10);
        let snap = s.snapshot(10);
        let mut copy = Screen::new(20, 5, 2000, VoidListener);
        copy.feed(&snap);
        // leaving the alternate screen shows the same primary on both
        copy.feed(b"\x1b[?1049lback");
        s.feed(b"\x1b[?1049lback");
        assert_eq!(dump(&copy.term, 10), dump(&s.term, 10));
        assert_eq!(state(&copy.term), state(&s.term));
    }

    #[test]
    fn an_open_synchronized_update_is_replayed_whole() {
        let mut s = screen(40, 4, b"before\x1b[?2026h\x1b[Hin the middle");
        assert!(s.sync_deadline().is_some());
        let snap = s.snapshot(0);
        let mut copy = Screen::new(40, 4, 2000, VoidListener);
        copy.feed(&snap);
        assert!(copy.sync_deadline().is_some(), "the copy waits for the same end");
        s.feed(b" and the end\x1b[?2026l");
        copy.feed(b" and the end\x1b[?2026l");
        assert_eq!(dump(&copy.term, 0), dump(&s.term, 0));
        assert!(dump(&s.term, 0)[0].contains("in the middle and the end"));
    }

    #[test]
    fn history_carries_into_a_fresh_terminal() {
        let mut bytes = Vec::new();
        for i in 0..40 {
            bytes.extend_from_slice(format!("\x1b[3{}mrow {i}\x1b[0m\r\n", i % 8).as_bytes());
        }
        let mut old = screen(30, 6, &bytes);
        let want: Vec<String> = dump(&old.term, 1000);
        let want_hist = &want[..old.term.grid().history_size()];
        let mut fresh = Screen::new(30, 6, 2000, VoidListener);
        carry_history(&mut old.term, &mut fresh.term);
        let got = dump(&fresh.term, 1000);
        let hist = fresh.term.grid().history_size();
        assert_eq!(&got[..hist], want_hist, "the scrollback is the same lines");
        assert!(
            got[hist..].iter().all(|l| l.trim_start_matches(|c| c != '}').trim_start_matches('}').is_empty()),
            "the screen is blank: {got:?}"
        );
        assert_eq!(fresh.term.grid().cursor.point.line.0, 0);
    }
}
