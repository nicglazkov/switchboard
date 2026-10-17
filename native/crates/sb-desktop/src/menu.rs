//! The right-click menu on a chat: the inspector's actions where the pointer is, with Continue spelled out per
//! machine. A click anywhere else, a right click elsewhere or Escape closes it.

use gpui::{ClickEvent, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point, SharedString, StatefulInteractiveElement, Styled, div, prelude::*, px};
use sb_core::actions::{self as acts, ActionId};

use crate::ui;
use crate::view::{BoardView, Ctx};

const W: f32 = 304.;
const ROW: f32 = 28.;

fn keycap(k: &str) -> Option<&'static str> {
    match k {
        "Enter" => Some("↵"),
        "F" => Some("F"),
        "R" => Some("R"),
        "C" => Some("C"),
        _ => None,
    }
}

enum Item {
    Act(ActionId),
    To(String),
}

struct Row {
    item: Item,
    title: String,
    icon: String,
    cap: Option<&'static str>,
    disabled: bool,
    danger: bool,
    /// v2: "This continues..." and "Not the same topic" sit apart, after a rule.
    thread: bool,
    /// Start fresh and Resume: what it costs right now ("about 91k").
    cost: Option<String>,
}

impl BoardView {
    pub fn open_menu(&mut self, key: String, at: Point<Pixels>, cx: &mut gpui::Context<Self>) {
        self.plan_open = false;
        self.select(Some(key.clone()), cx);
        self.menu = Some((key, at));
        cx.notify();
    }

    pub fn ctx_menu(&self, c: &Ctx, w: f32, h: f32) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let close = |this: &mut BoardView, cx: &mut gpui::Context<BoardView>| {
            this.menu = None;
            cx.notify();
        };
        let scrim = div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, _, cx| close(this, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(move |this, _, _, cx| close(this, cx)));
        let Some((key, at)) = &self.menu else { return scrim };
        let Some(s) = c.app.snap.session(key) else { return scrim };

        // Where its terminal opens comes first (a tab, a window, a floating sheet), then the rest.
        let places = (s.background() || !s.alive) && !c.app.snap.machine(&s.machine).is_some_and(|m| m.is_offline());
        let mut rows: Vec<Row> = Vec::new();
        for a in acts::for_session(s, &c.app.snap, c.now) {
            if places && matches!(a.id, ActionId::Terminal | ActionId::ResumeOpen) {
                continue;
            }
            if a.id == ActionId::Continue {
                for m in c.app.snap.other_machines(&s.machine) {
                    let title = format!("Continue on {}", m.label);
                    rows.push(Row { item: Item::To(m.name.clone()), title, icon: "cont".into(), cap: None, disabled: false, danger: false, thread: false, cost: None });
                }
                continue;
            }
            let cap = a.key.as_deref().and_then(keycap).filter(|_| !a.disabled);
            let warm = s.cache_left(c.now) > 0;
            let cost = s.cost.as_ref().filter(|_| !a.disabled).and_then(|k| match a.id {
                ActionId::Fresh => Some(k.fresh.total),
                ActionId::Resume | ActionId::ResumeOpen => Some(if warm { k.resume.warm } else { k.resume.cold }),
                _ => None,
            });
            let cost = cost.map(|n| format!("about {}", sb_core::fmt::k(n.max(0.).round() as u64)));
            rows.push(Row { item: Item::Act(a.id), title: a.title.clone(), icon: a.icon.clone(), cap, disabled: a.disabled, danger: a.danger, thread: a.id.is_thread(), cost });
        }
        let costs = rows.iter().any(|r| r.cost.is_some());
        // A rule before the first danger row and before the thread rows (never at the top).
        let rule_at = |rows: &[Row], f: fn(&Row) -> bool| rows.iter().position(f).filter(|i| *i > 0);
        let danger_at = rule_at(&rows, |r| r.danger);
        let thread_at = rule_at(&rows, |r| r.thread);
        let seps = danger_at.iter().chain(thread_at.iter()).collect::<std::collections::HashSet<_>>().len();
        let menu_h = rows.len() as f32 * ROW + seps as f32 * 9. + 12. + if places { 3. * 30. + 9. } else { 0. } + ROW + 9. + if costs { 48. } else { 0. };
        let x = f32::from(at.x).min(w - W - 8.).max(8.);
        let y = f32::from(at.y).min(h - menu_h - 8.).max(8.);

        let mut menu = div()
            .id("ctx-menu")
            .absolute()
            .left(px(x))
            .top(px(y))
            .w(px(W))
            .p(px(6.))
            .rounded(px(ui::R_POP))
            .border_1()
            .border_color(t.line2)
            .bg(t.panel2)
            .shadow(t.drop())
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .flex()
            .flex_col();
        if places {
            menu = menu.children(self.place_rows(c, key, "ctx-place")).child(div().my(px(4.)).mx(px(4.)).h(px(1.)).bg(t.rule));
        }
        let fk = key.clone();
        let hover = t.hover;
        menu = menu
            .child(
                div()
                    .id("ctx-flags")
                    .h(px(ROW))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(ui::R_ITEM))
                    .text_size(px(13.))
                    .text_color(t.ink)
                    .cursor_pointer()
                    .hover(move |st| st.bg(hover))
                    .child(ui::icon("flag", 15., t.faint))
                    .child(ui::line("Flags").flex_1())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_flags(fk.clone(), cx))),
            )
            .child(div().my(px(4.)).mx(px(4.)).h(px(1.)).bg(t.rule));
        for (i, Row { item, title, icon, cap, disabled, danger, cost, .. }) in rows.into_iter().enumerate() {
            if Some(i) == danger_at || Some(i) == thread_at {
                menu = menu.child(div().my(px(4.)).mx(px(4.)).h(px(1.)).bg(t.rule));
            }
            let ink = if disabled { t.faint } else if danger { t.bad } else { t.ink };
            let ic = if danger { t.bad } else { t.faint };
            let hover = t.hover;
            let k = key.clone();
            let icon_el = match &item {
                Item::To(m) => ui::machine_icon(m, 15., ic),
                Item::Act(_) => ui::icon(&icon, 15., ic),
            };
            menu = menu.child(
                div()
                    .id(SharedString::from(format!("ctx-{i}")))
                    .h(px(ROW))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(ui::R_ITEM))
                    .text_size(px(13.))
                    .text_color(ink)
                    .when(!disabled, |d| d.cursor_pointer().hover(move |st| st.bg(hover)))
                    .child(icon_el)
                    .child(ui::line(title).flex_1())
                    .when_some(cost, |d, c| d.child(div().flex_none().text_size(px(12.)).text_color(t.faint).child(c)))
                    .when_some(cap, |d, k| d.child(ui::kbd(&t, k)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if disabled {
                            return;
                        }
                        this.menu = None;
                        match &item {
                            Item::To(to) => {
                                let (k, to) = (k.clone(), to.clone());
                                this.app.update(cx, |s, cx| s.continue_on(&k, &to, cx));
                            }
                            Item::Act(id) => this.run_action(*id, k.clone(), cx),
                        }
                        cx.notify();
                    })),
            );
        }
        if costs {
            // Where the numbers come from: the detail sheet (open beside the menu) has "How is this estimated?".
            menu = menu.child(div().mt(px(4.)).mx(px(4.)).h(px(1.)).bg(t.rule)).child(
                ui::wrap("Costs in tokens, for the cache as it is now. How they are estimated is in the detail sheet.")
                    .px(px(8.))
                    .pt(px(6.))
                    .pb(px(2.))
                    .text_size(px(11.5))
                    .line_height(gpui::relative(1.35))
                    .text_color(t.faint),
            );
        }
        scrim.child(menu)
    }
}
