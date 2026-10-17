//! History: the tab next to Board. Everything older, by day and by thread, with search and filters for machine,
//! folder and state; each chat's summary and thread; Resume, Start fresh and Open terminal in one click. It is the
//! session browser (browser.rs) drawn inside the main window.

use gpui::{AnyElement, AppContext, Context, Entity, IntoElement, ParentElement, Styled, Window, div};

use crate::browser::BrowserView;
use crate::view::{BoardView, Ctx, Main};

#[derive(Default)]
pub struct Hist {
    pub view: Option<Entity<BrowserView>>,
}

impl Hist {
    pub fn new(_: &mut Window, _: &mut Context<BoardView>) -> Hist {
        Hist::default()
    }
}

impl BoardView {
    /// Shows History in the first tab (Cmd+B, the "+" menu, the footer).
    pub fn view_history(&mut self, cx: &mut Context<Self>) {
        self.main = Main::History;
        self.tab = 0;
        self.add_open = false;
        cx.notify();
    }

    pub fn view_board(&mut self, cx: &mut Context<Self>) {
        self.main = Main::Board;
        self.tab = 0;
        cx.notify();
    }

    /// Makes the History view the first time it is shown (it needs the window).
    pub fn ensure_history(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.main == Main::History && self.hist.view.is_none() {
            self.hist.view = Some(cx.new(|cx| BrowserView::embedded(window, cx)));
        }
    }

    pub fn history_view(&self, _c: &Ctx, _w: f32) -> AnyElement {
        match &self.hist.view {
            Some(v) => div().flex_1().min_h_0().flex().flex_col().child(v.clone()).into_any_element(),
            None => div().flex_1().into_any_element(),
        }
    }
}
