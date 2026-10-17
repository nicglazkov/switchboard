//! The details popped out of the board into their own window. It shows whatever chat is selected on the board, so it
//! keeps following as you click around; Dock (or closing the window) brings the floating sheet back.

use gpui::{
    AnyElement, Context, IntoElement, ParentElement, Render, Styled, Subscription, WeakEntity, Window, div, prelude::*,
    px, relative,
};

use crate::inspector::InsOpts;
use crate::state::AppState;
use crate::theme::{Theme, UI};
use crate::ui;
use crate::view::{BoardView, Ctx};

pub struct DetachedView {
    board: WeakEntity<BoardView>,
    _subs: Vec<Subscription>,
}

impl DetachedView {
    fn new(
        board: gpui::Entity<BoardView>,
        app: gpui::Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let weak = board.downgrade();
        let gone = weak.clone();
        let subs = vec![
            // a new selection or new data on the board repaints this window too
            cx.observe(&board, |_, _, cx| cx.notify()),
            cx.observe(&app, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
            crate::wake::watch(window),
            // closed with its own close button: the board shows the sheet again
            cx.on_release(move |_, cx| {
                cx.defer(move |cx| {
                    if let Some(b) = gone.upgrade() {
                        b.update(cx, |b, cx| {
                            b.detached = None;
                            cx.notify();
                        });
                    }
                })
            }),
        ];
        Self { board: weak, _subs: subs }
    }
}

impl Render for DetachedView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let body = match self.board.upgrade() {
            Some(b) => b.update(cx, |b, cx| b.detached_panel(window, cx)),
            None => div().into_any_element(),
        };
        div().size_full().flex().flex_col().bg(t.panel).text_color(t.ink).font_family(UI).text_size(px(13.)).child(body)
    }
}

impl BoardView {
    /// Pop out: the details leave the board for their own window, which follows the selection from now on.
    pub fn pop_out(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = self.detached
            && h.update(cx, |_, window, _| window.activate_window()).is_ok()
        {
            return;
        }
        let (me, app) = (cx.entity(), self.app.clone());
        // Opened after this click is handled: the new window paints the board's details at once, and the board
        // cannot be read while it is still being updated here (that aborted the app).
        cx.defer(move |cx| {
            let opts = crate::files::options("Details, Switchboard", 452., 760., (360., 420.), cx);
            let board = me.clone();
            match cx.open_window(opts, move |window, cx| cx.new(|cx| DetachedView::new(board, app, window, cx))) {
                Ok(h) => me.update(cx, |b, cx| {
                    b.detached = Some(h.into());
                    cx.notify();
                }),
                Err(e) => log::error!("could not pop out the details: {e:#}"),
            }
        });
    }

    /// Dock: close the popped-out window; the floating sheet shows the selection again.
    pub fn dock(&mut self, cx: &mut Context<Self>) {
        if let Some(h) = self.detached.take() {
            // after this click: Dock is clicked inside that window, which is busy handling the click right now
            cx.defer(move |cx| {
                let _ = h.update(cx, |_, window, _| window.remove_window());
            });
        }
        cx.notify();
    }

    /// What the popped-out window paints: the selected chat in full, or a short note when nothing is selected.
    pub fn detached_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.ask_inputs(self.sel.clone(), window, cx);
        let t = Theme::get(cx);
        let app_e = self.app.clone();
        let cx = &*cx;
        let app = app_e.read(cx);
        let now = AppState::now();
        let selected =
            self.sel.as_ref().filter(|k| app.snap.session(k).is_some() || app.snap.replaced_row(k).is_some()).cloned();
        let c = Ctx { t, app, now, cx, card_w: 200. };
        match selected {
            Some(k) => div()
                .size_full()
                .min_h_0()
                .flex()
                .flex_col()
                .child(self.inspector(&c, &k, InsOpts { close: true, radius: 0., detached: true }))
                .into_any_element(),
            None => div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.))
                .px(px(32.))
                .child(ui::icon("inspector", 22., t.faint))
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(t.dim)
                        .line_height(relative(1.5))
                        .child("Click a chat on the board to see it here."),
                )
                .child(
                    ui::icon_btn(&t, "ins-dock-empty", "sidebar", 26.)
                        .on_click(c.cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.dock(cx))),
                )
                .into_any_element(),
        }
    }
}
