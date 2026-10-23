//! Tab groups in the tab row, Chrome style. Starred tabs sit at the far left, outside any group; then each group
//! starts with a chip in its color (its name, or just a dot when it has none) and its tabs carry a 2 px line in that
//! color; then the tabs in no group. Clicking a chip folds the group (its tabs hide, the chip shows how many);
//! right-clicking it opens its menu. Membership, names, colors and the groups' order come from the server (every Mac
//! sees the same groups); which tabs are open, and which groups are folded, stays on this Mac.

use gpui::{
    AnyElement, ClickEvent, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::*, px, relative, rgb,
};
use sb_core::groups::{Group, GroupColor, Op};

use crate::theme::Theme;
use crate::ui;
use crate::view::{BoardView, Ctx};

/// Chrome's tab group colors: deep in light mode (white text), soft in dark mode (dark text), so a chip reads on
/// either header and the 2 px line under a tab stays visible.
pub fn group_color(t: &Theme, c: GroupColor) -> Hsla {
    let hex = if t.dark {
        match c {
            GroupColor::Grey => 0xBDC1C6,
            GroupColor::Blue => 0x8AB4F8,
            GroupColor::Red => 0xF28B82,
            GroupColor::Yellow => 0xFDD663,
            GroupColor::Green => 0x81C995,
            GroupColor::Pink => 0xFF8BCB,
            GroupColor::Purple => 0xD7AEFB,
            GroupColor::Cyan => 0x78D9EC,
        }
    } else {
        match c {
            GroupColor::Grey => 0x5F6368,
            GroupColor::Blue => 0x1A73E8,
            GroupColor::Red => 0xD93025,
            GroupColor::Yellow => 0xB06000,
            GroupColor::Green => 0x1E8E3E,
            GroupColor::Pink => 0xD01884,
            GroupColor::Purple => 0x9334E6,
            GroupColor::Cyan => 0x007B83,
        }
    };
    rgb(hex).into()
}

/// The text on a chip of that color.
pub fn group_ink(t: &Theme, c: GroupColor) -> Hsla {
    let _ = c;
    if t.dark { rgb(0x16181B).into() } else { rgb(0xFFFFFF).into() }
}

/// A small dot in a group's color (board cards, the palette).
pub fn group_dot(t: &Theme, c: GroupColor, size: f32) -> gpui::Div {
    div().flex_none().size(px(size)).rounded_full().bg(group_color(t, c))
}

/// What the tab row needs to know about one terminal tab.
#[derive(Clone, Debug, Default)]
pub struct TabInfo {
    pub star: bool,
    /// The chat's thread (its own sid when it has none); None while a tab does not know its chat yet.
    pub thread: Option<String>,
    /// The group it shows in (an index of `groups`); a starred tab is in none.
    pub group: Option<usize>,
}

/// One place in the tab row: a group's chip, or a terminal tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Chip(usize),
    Tab(usize),
}

/// The order tabs show in: starred first, then the groups in their order (each in its members' order), then the
/// rest; ties keep the order they have.
pub fn sort_key(infos: &[TabInfo], groups: &[Group], i: usize) -> (u8, usize, usize, usize) {
    let x = &infos[i];
    if x.star {
        return (0, 0, 0, i);
    }
    match x.group {
        Some(g) => {
            let m = x.thread.as_ref().and_then(|t| groups[g].members.iter().position(|m| m == t)).unwrap_or(usize::MAX);
            (1, g, m, i)
        }
        None => (2, 0, 0, i),
    }
}

/// The row: each group's chip before its first tab, and a folded group's tabs left out.
pub fn slots(infos: &[TabInfo], groups: &[Group], folded: &[String]) -> Vec<Slot> {
    let mut out = Vec::new();
    let mut last = None;
    for (i, x) in infos.iter().enumerate() {
        if let Some(g) = x.group {
            if last != Some(g) {
                out.push(Slot::Chip(g));
            }
            last = Some(g);
            if folded.contains(&groups[g].id) {
                continue;
            }
        } else {
            last = None;
        }
        out.push(Slot::Tab(i));
    }
    out
}

/// The note beside "Close group", from whether closing each of its tabs stops that tab's chat.
pub fn close_group_note(stops: &[bool]) -> &'static str {
    match (stops.iter().any(|s| *s), stops.iter().all(|s| *s)) {
        (false, _) => "chats keep running",
        (true, true) => "stops its chats",
        (true, false) => "stops some chats",
    }
}

/// Where a dragged group chip would move, counted among the other groups (see BoardView::group_drag_over). `over`
/// was drawn from an earlier frame: a group gone since (ungrouped elsewhere, or by the server) marks no place.
pub fn group_drop_index(infos: &[TabInfo], groups: &[Group], gid: &str, over: Slot, right: bool) -> Option<usize> {
    let others: Vec<&Group> = groups.iter().filter(|g| g.id != gid).collect();
    let at = |g: usize, after: bool| {
        let id = &groups.get(g)?.id;
        others.iter().position(|o| o.id == *id).map(|p| p + after as usize)
    };
    let own = || groups.iter().position(|x| x.id == gid);
    match over {
        Slot::Chip(g) if groups.get(g).is_some_and(|x| x.id == gid) => own(),
        Slot::Chip(g) => at(g, right),
        Slot::Tab(i) => match infos.get(i) {
            Some(x) if x.star => Some(0),
            Some(TabInfo { group: Some(g), .. }) if groups.get(*g).is_some_and(|x| x.id == gid) => own(),
            Some(TabInfo { group: Some(g), .. }) => at(*g, true),
            _ => Some(others.len()),
        },
    }
}

/// Where tab `from` (chat `thread`) goes among the members of group `g` (index `gi`) when dropped at gap `to`: after
/// the group's tab before the gap, or else before its tab after it. `to` was drawn from an earlier frame, so it may lie
/// past tabs closed since.
pub fn group_add_index(infos: &[TabInfo], g: &Group, gi: usize, thread: &str, from: usize, to: usize) -> usize {
    let to = to.min(infos.len());
    let members: Vec<&String> = g.members.iter().filter(|m| *m != thread).collect();
    let pos = |j: usize| infos[j].thread.as_ref().and_then(|t| members.iter().position(|m| *m == t));
    let prev = (0..to).rev().find(|j| *j != from).filter(|j| infos[*j].group == Some(gi));
    let next = (to..infos.len()).find(|j| *j != from).filter(|j| infos[*j].group == Some(gi));
    prev.and_then(pos).map(|p| p + 1).or_else(|| next.and_then(pos)).unwrap_or(0)
}

/// About how wide a chip draws (for the room the tabs get).
pub fn chip_width(g: &Group, folded: bool, max: f32, editing: bool) -> f32 {
    // The name field while it is being named (group_chip: 168 px plus the chip's padding).
    if editing {
        return 178.;
    }
    let name = g.name.chars().count() as f32 * 6.8;
    let count = if folded { 22. } else { 0. };
    // Under 40 px a name would be a letter or two: the chip shows the color alone, as for a group with no name.
    if g.name.is_empty() || max < 40. {
        if folded { 34. } else { 22. }
    } else {
        (name + 18. + count).min(max + count) + 4.
    }
}

/// What a group chip carries while it is dragged.
#[derive(Clone)]
pub struct GroupDrag {
    pub id: String,
    pub name: String,
    pub color: GroupColor,
}

impl gpui::Render for GroupDrag {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let name = if self.name.is_empty() { "Group".to_string() } else { self.name.clone() };
        div()
            .px(px(10.))
            .h(px(22.))
            .flex()
            .items_center()
            .rounded(px(11.))
            .bg(group_color(&t, self.color))
            .shadow(t.drop())
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(group_ink(&t, self.color))
            .child(name)
    }
}

impl BoardView {
    /// Each tab's star, thread and group, and the groups as they show now.
    pub fn tab_infos(&self, cx: &gpui::App) -> (Vec<TabInfo>, Vec<Group>) {
        let app = self.app.read(cx);
        let groups = app.snap.groups.clone();
        let infos = self
            .tabs
            .iter()
            .map(|tb| {
                let Some(s) = tb.read(cx).key.clone().and_then(|k| app.snap.session(&k).cloned()) else {
                    return TabInfo::default();
                };
                let thread = s.thread.clone().unwrap_or_else(|| s.sid.clone());
                let group = if s.star { None } else { groups.iter().position(|g| g.members.contains(&thread)) };
                TabInfo { star: s.star, thread: Some(thread), group }
            })
            .collect();
        (infos, groups)
    }

    /// The groups folded on this Mac.
    pub fn folded(&self, cx: &gpui::App) -> Vec<String> {
        self.app.read(cx).settings.folded_groups.clone()
    }

    /// Starred tabs first, then each group together in its order, then the rest (each part keeping its order), the
    /// tab that shows staying shown. Saved when it changed. A change made anywhere (another Mac, the iPhone) moves
    /// the tabs on the next frame.
    pub fn order_tabs(&mut self, cx: &mut gpui::Context<Self>) {
        let (infos, groups) = self.tab_infos(cx);
        let keys: Vec<_> = (0..infos.len()).map(|i| sort_key(&infos, &groups, i)).collect();
        if keys.windows(2).all(|w| w[0] <= w[1]) {
            return;
        }
        let mut idx: Vec<usize> = (0..infos.len()).collect();
        idx.sort_by_key(|i| keys[*i]);
        let shown = self.tab.checked_sub(1).and_then(|i| self.tabs.get(i)).cloned();
        let old = std::mem::take(&mut self.tabs);
        self.tabs = idx.into_iter().map(|i| old[i].clone()).collect();
        if let Some(s) = shown
            && let Some(i) = self.tabs.iter().position(|x| *x == s)
        {
            self.tab = i + 1;
        }
        self.tab_drop = None;
        self.remember_tabs(cx);
    }

    // ---------- changes ----------

    /// "Add to new group": a group with this tab, in the next unused color, its name typed on the chip.
    pub fn new_group_from_tab(&mut self, i: usize, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        self.tab_menu = None;
        self.tab_menu_sub = false;
        let (infos, groups) = self.tab_infos(cx);
        let Some(thread) = infos.get(i).and_then(|x| x.thread.clone()) else { return };
        let color = GroupColor::next_unused(&groups);
        let id = self
            .app
            .update(cx, |s, cx| s.group_op(Op::Create { name: String::new(), color, members: vec![thread] }, cx));
        self.edit_group_name(id, window, cx);
    }

    pub fn add_tab_to_group(&mut self, i: usize, gid: String, cx: &mut gpui::Context<Self>) {
        self.tab_menu = None;
        self.tab_menu_sub = false;
        let (infos, _) = self.tab_infos(cx);
        let Some(thread) = infos.get(i).and_then(|x| x.thread.clone()) else { return };
        self.app.update(cx, |s, cx| {
            s.group_op(Op::Add { id: gid, thread, index: None }, cx);
        });
        cx.notify();
    }

    pub fn remove_tab_from_group(&mut self, i: usize, cx: &mut gpui::Context<Self>) {
        self.tab_menu = None;
        self.tab_menu_sub = false;
        let (infos, _) = self.tab_infos(cx);
        let Some(thread) = infos.get(i).and_then(|x| x.thread.clone()) else { return };
        // It goes to the front of the tabs in no group, next to where it was.
        self.app.update(cx, |s, cx| {
            s.group_op(Op::Remove { thread }, cx);
        });
        cx.notify();
    }

    /// Folds or unfolds a group (on this Mac only). Folding the group of the tab that shows moves to the nearest tab
    /// still in sight, else the board.
    pub fn toggle_fold(&mut self, gid: &str, cx: &mut gpui::Context<Self>) {
        let fold = !self.folded(cx).iter().any(|g| g == gid);
        self.app.update(cx, |s, _| {
            s.settings.folded_groups.retain(|g| g != gid);
            if fold {
                s.settings.folded_groups.push(gid.to_string());
            }
            s.save_settings();
        });
        if fold {
            let (infos, groups) = self.tab_infos(cx);
            let gi = groups.iter().position(|g| g.id == gid);
            if let Some(shown) = self.tab.checked_sub(1)
                && gi.is_some()
                && infos.get(shown).is_some_and(|x| x.group == gi)
            {
                let folded = self.folded(cx);
                let visible = |j: usize| infos[j].group.is_none_or(|g| !folded.contains(&groups[g].id));
                let next =
                    (shown + 1..infos.len()).find(|j| visible(*j)).or_else(|| (0..shown).rev().find(|j| visible(*j)));
                self.set_tab(next.map(|j| j + 1).unwrap_or(0), cx);
            }
        }
        cx.notify();
    }

    /// Types a group's name on its chip.
    pub fn edit_group_name(&mut self, gid: String, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        self.group_menu = None;
        let name =
            self.app.read(cx).snap.groups.iter().find(|g| g.id == gid).map(|g| g.name.clone()).unwrap_or_default();
        self.group_edit = Some(gid);
        self.focus_tab = false;
        self.group_name.update(cx, |n, cx| {
            n.set_value(name, window, cx);
            n.focus(window, cx);
            n.select_all(window, cx);
        });
        cx.notify();
    }

    /// Enter, or the field losing focus: the name typed is saved (unchanged: nothing is sent).
    pub fn commit_group_name(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(gid) = self.group_edit.take() else { return };
        let name = sb_core::groups::clean_name(&self.group_name.read(cx).value());
        let gid = self.app.read(cx).group_id(&gid);
        let same = self.app.read(cx).snap.groups.iter().find(|g| g.id == gid).is_none_or(|g| g.name == name);
        if !same {
            self.app.update(cx, |s, cx| {
                s.group_op(Op::Update { id: gid, name: Some(name), color: None }, cx);
            });
        }
        self.focus_tab = true;
        cx.notify();
    }

    /// Escape: the name goes back to what it was.
    pub fn cancel_group_name(&mut self, cx: &mut gpui::Context<Self>) {
        self.group_edit = None;
        self.focus_tab = true;
        cx.notify();
    }

    fn group_do(&mut self, op: Op, cx: &mut gpui::Context<Self>) {
        self.group_menu = None;
        self.app.update(cx, |s, cx| {
            s.group_op(op, cx);
        });
        cx.notify();
    }

    /// "Close group": its tabs close here, each as its own Close would (a running chat stops unless kept running), and
    /// the group stays (other Macs keep theirs).
    pub fn close_group(&mut self, gid: &str, cx: &mut gpui::Context<Self>) {
        self.group_menu = None;
        let (infos, groups) = self.tab_infos(cx);
        let Some(gi) = groups.iter().position(|g| g.id == gid) else { return };
        for i in (0..infos.len()).rev() {
            if infos[i].group == Some(gi) {
                self.close_tab(i, cx);
            }
        }
        cx.notify();
    }

    // ---------- dragging ----------

    /// A tab dragged over tab `over` (`right`: its right half): the gap it would drop into, and the group it would
    /// join (a tab over a group's tabs joins that group; over the tabs in no group, it leaves its group). A starred
    /// tab moves only among starred ones, and nothing else drops among them.
    pub fn tab_drag_over(&mut self, from: usize, over: usize, right: bool, cx: &mut gpui::Context<Self>) {
        let (infos, groups) = self.tab_infos(cx);
        let (Some(f), Some(o)) = (infos.get(from), infos.get(over)) else { return };
        let gap = if right { over + 1 } else { over };
        let n_star = infos.iter().take_while(|x| x.star).count();
        let (gap, into) = if f.star {
            (gap.min(n_star), None)
        } else if o.star {
            (n_star, infos.get(n_star).and_then(|x| x.group).map(|g| groups[g].id.clone()))
        } else {
            (gap, o.group.map(|g| groups[g].id.clone()))
        };
        if self.drop_group != into {
            self.drop_group = into;
            cx.notify();
        }
        self.set_tab_drop(Some((from, gap)), cx);
    }

    /// A tab dragged over a group's chip: it would join that group, before its first tab.
    pub fn tab_drag_over_chip(&mut self, from: usize, g: usize, cx: &mut gpui::Context<Self>) {
        let (infos, groups) = self.tab_infos(cx);
        if infos.get(from).is_none_or(|x| x.star) {
            return;
        }
        let first = infos.iter().position(|x| x.group == Some(g)).unwrap_or(infos.len());
        let into = groups.get(g).map(|g| g.id.clone());
        if self.drop_group != into {
            self.drop_group = into;
            cx.notify();
        }
        self.set_tab_drop(Some((from, first)), cx);
    }

    /// A tab dropped on the tab row: it moves to the marked gap, joining or leaving a group on the way.
    pub fn drop_tab(&mut self, from: usize, cx: &mut gpui::Context<Self>) {
        let target = self.drop_group.take();
        let Some((_, to)) = self.tab_drop.take() else {
            cx.notify();
            return;
        };
        let (infos, groups) = self.tab_infos(cx);
        let Some(f) = infos.get(from).cloned() else { return };
        if f.star {
            self.reorder_tab(from, to, cx);
            return;
        }
        let cur = f.group.map(|g| groups[g].id.clone());
        match (target, f.thread) {
            (Some(gid), Some(thread)) => {
                let Some(gi) = groups.iter().position(|g| g.id == gid) else { return };
                let index = group_add_index(&infos, &groups[gi], gi, &thread, from, to);
                self.reorder_tab(from, to, cx);
                self.app.update(cx, |s, cx| {
                    s.group_op(Op::Add { id: gid, thread, index: Some(index) }, cx);
                });
            }
            (None, Some(thread)) if cur.is_some() => {
                self.reorder_tab(from, to, cx);
                self.app.update(cx, |s, cx| {
                    s.group_op(Op::Remove { thread }, cx);
                });
            }
            _ => self.reorder_tab(from, to, cx),
        }
        cx.notify();
    }

    /// A group chip dragged over chip `over` (`right`: its right half) or over a tab: where the group would move,
    /// counted among the other groups (before or after the group under the pointer; past the tabs in no group, last;
    /// over the starred tabs, first).
    pub fn group_drag_over(&mut self, gid: &str, over: Slot, right: bool, cx: &mut gpui::Context<Self>) {
        let (infos, groups) = self.tab_infos(cx);
        let d = group_drop_index(&infos, &groups, gid, over, right).map(|i| (gid.to_string(), i));
        if self.group_drop != d {
            self.group_drop = d;
            cx.notify();
        }
    }

    pub fn drop_group_chip(&mut self, gid: &str, cx: &mut gpui::Context<Self>) {
        let Some((id, index)) = self.group_drop.take() else { return };
        let now = self.app.read(cx).snap.groups.iter().position(|g| g.id == gid);
        if id == gid && now != Some(index) {
            self.app.update(cx, |s, cx| {
                s.group_op(Op::Move { id, index }, cx);
            });
        }
        cx.notify();
    }

    // ---------- drawing ----------

    /// A group's chip: its name in a pill of its color (or a dot when it has none), with the count of its tabs when
    /// folded. The name gives way to an ellipsis at `max` px; while it is being named, the field sits in the pill.
    #[allow(clippy::too_many_arguments)]
    pub fn group_chip(
        &self,
        c: &Ctx,
        gi: usize,
        g: &Group,
        count: usize,
        folded: bool,
        max: f32,
        ring: bool,
    ) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let col = group_color(&t, g.color);
        let ink = group_ink(&t, g.color);
        let editing = self.group_edit.as_deref() == Some(g.id.as_str());
        let gid = g.id.clone();
        let gid2 = g.id.clone();
        let gid3 = g.id.clone();
        let drag = GroupDrag { id: g.id.clone(), name: g.name.clone(), color: g.color };
        let count_el = |on_pill: bool| {
            div()
                .flex_none()
                .min_w(px(16.))
                .h(px(16.))
                .px(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(8.))
                .bg(if on_pill { gpui::hsla(0., 0., if t.dark { 0. } else { 1. }, 0.22) } else { col })
                .text_size(px(10.5))
                .font_weight(FontWeight::BOLD)
                .text_color(ink)
                .child(count.to_string())
        };
        let mut chip = div()
            .id(("group-chip", gi))
            .flex_none()
            .relative()
            .h(px(30.))
            .px(px(3.))
            .flex()
            .items_center()
            .cursor_pointer()
            .rounded(px(ui::R_CTRL))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .hover(move |s| s.bg(t.hover));
        // While naming: a light field outlined in the group's color, its dot in front. The input draws in the
        // app's own ink and placeholder colors, which vanish on a solid colored pill.
        let pill = if editing {
            div()
                .flex_none()
                .w(px(168.))
                .h(px(24.))
                .pl(px(8.))
                .pr(px(10.))
                .gap(px(6.))
                .flex()
                .items_center()
                .rounded(px(12.))
                .bg(t.panel)
                .border_2()
                .border_color(col)
                .text_size(px(12.5))
                .text_color(t.ink)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(div().flex_none().size(px(8.)).rounded_full().bg(col))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .flex()
                        .items_center()
                        .child(gpui_base::Input::new(&self.group_name)),
                )
                .into_any_element()
        } else if (g.name.is_empty() || max < 40.) && !folded {
            div().flex_none().mx(px(4.)).size(px(12.)).rounded_full().bg(col).into_any_element()
        } else if g.name.is_empty() || max < 40. {
            div()
                .flex_none()
                .h(px(22.))
                .pl(px(7.))
                .pr(px(3.))
                .gap(px(5.))
                .flex()
                .items_center()
                .rounded(px(11.))
                .bg(col)
                .child(div().flex_none().size(px(6.)).rounded_full().bg(ink))
                .child(count_el(true))
                .into_any_element()
        } else {
            div()
                .flex_none()
                .max_w(px(max + if folded { 22. } else { 0. }))
                .h(px(22.))
                .pl(px(9.))
                .pr(px(if folded { 3. } else { 9. }))
                .gap(px(5.))
                .flex()
                .items_center()
                .rounded(px(11.))
                .bg(col)
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ink)
                .child(ui::line(g.name.clone()).flex_shrink(1.).line_height(relative(1.)))
                .when(folded, |d| d.child(count_el(true)))
                .into_any_element()
        };
        chip = chip.child(pill).when(ring, |d| d.shadow(vec![Theme::ring(t.accent)]));
        if editing {
            return chip.into_any_element();
        }
        chip.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_fold(&gid, cx)))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    this.tab_menu = None;
                    this.add_open = false;
                    this.group_menu = Some((gid2.clone(), ev.position));
                    cx.notify();
                }),
            )
            .on_drag(drag, |d, _, _, cx| cx.new(|_| d.clone()))
            .on_drag_move::<crate::chrome::TabDrag>(cx.listener(
                move |this, ev: &gpui::DragMoveEvent<crate::chrome::TabDrag>, _, cx| {
                    let (b, p) = (ev.bounds, ev.event.position);
                    if b.contains(&p)
                        && let Some(from) = this.tab_index(ev.drag(cx).0)
                    {
                        this.tab_drag_over_chip(from, gi, cx);
                    }
                },
            ))
            .on_drag_move::<GroupDrag>(cx.listener(move |this, ev: &gpui::DragMoveEvent<GroupDrag>, _, cx| {
                let (b, p) = (ev.bounds, ev.event.position);
                if b.contains(&p) {
                    let id = ev.drag(cx).id.clone();
                    this.group_drag_over(&id, Slot::Chip(gi), p.x >= b.left() + b.size.width / 2., cx);
                }
            }))
            .on_drop::<GroupDrag>(cx.listener(move |this, d: &GroupDrag, _, cx| {
                let _ = &gid3;
                this.drop_group_chip(&d.id, cx);
            }))
            .into_any_element()
    }

    /// A chip's own menu: rename, color, ungroup, close its tabs here, move it left or right.
    pub fn group_menu_el(&self, c: &Ctx, w: f32) -> Option<AnyElement> {
        let (gid, at) = self.group_menu.clone()?;
        let t = c.t;
        let cx = c.cx;
        let groups = c.app.snap.groups.clone();
        let gi = groups.iter().position(|g| g.id == gid)?;
        let g = groups[gi].clone();
        let x = f32::from(at.x).min(w - 268.).max(8.);
        let row = |id: &'static str, ic: &'static str, title: &str, sub: Option<&str>| {
            let hover = t.hover;
            div()
                .id(id)
                .min_h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(ui::R_ITEM))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .text_size(px(13.))
                .text_color(t.ink)
                .child(ui::icon(ic, 15., t.dim))
                .child(div().flex_1().child(title.to_string()))
                .when_some(sub.map(str::to_string), |d, s| {
                    d.child(div().text_size(px(12.)).text_color(t.faint).child(s))
                })
        };
        let rule = || div().my(px(4.)).mx(px(4.)).h(px(1.)).bg(t.rule);
        let swatches = GroupColor::ALL.iter().enumerate().fold(
            div().px(px(10.)).py(px(8.)).flex().items_center().gap(px(8.)),
            |d, (k, col)| {
                let col = *col;
                let on = col == g.color;
                let id = g.id.clone();
                d.child(
                    div()
                        .id(("grp-color", k))
                        .flex_none()
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .cursor_pointer()
                        .when(on, |d| d.shadow(vec![Theme::ring(t.ink2)]))
                        .child(
                            div()
                                .size(px(16.))
                                .rounded_full()
                                .bg(group_color(&t, col))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(on, |d| d.child(ui::icon("check", 11., group_ink(&t, col)))),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.group_do(Op::Update { id: id.clone(), name: None, color: Some(col) }, cx);
                        })),
                )
            },
        );
        let (id1, id2, id3, id4, id5) = (g.id.clone(), g.id.clone(), g.id.clone(), g.id.clone(), g.id.clone());
        // What closing the group does to its chats, as each tab's own Close says it.
        let (infos, _) = self.tab_infos(cx);
        let stops: Vec<bool> = (0..infos.len())
            .filter(|i| infos[*i].group == Some(gi))
            .map(|i| {
                let t = &self.tabs[i];
                t.read(cx).key.as_deref().is_some_and(|k| self.close_stops(k, t.entity_id(), cx))
            })
            .collect();
        let close_note = close_group_note(&stops);
        let n = groups.len();
        let title = if g.name.is_empty() { "Unnamed group".to_string() } else { g.name.clone() };
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.group_menu = None;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("group-menu")
                        .absolute()
                        .left(px(x))
                        .top(px(ui::header() + 2.))
                        .w(px(260.))
                        .p(px(6.))
                        .rounded(px(ui::R_POP))
                        .border_1()
                        .border_color(t.line2)
                        .bg(t.panel2)
                        .shadow(t.drop())
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .px(px(10.))
                                .pt(px(6.))
                                .pb(px(2.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(group_dot(&t, g.color, 9.))
                                .child(ui::line(title).flex_1().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink2)),
                        )
                        .child(swatches)
                        .child(rule())
                        .child(row("grp-rename", "dots", if g.name.is_empty() { "Name group" } else { "Rename group" }, None).on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| this.edit_group_name(id1.clone(), window, cx),
                        )))
                        .when(gi > 0, |d| {
                            d.child(row("grp-left", "back", "Move group left", None).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.group_do(Op::Move { id: id2.clone(), index: gi - 1 }, cx)
                            })))
                        })
                        .when(gi + 1 < n, |d| {
                            d.child(row("grp-right", "chev", "Move group right", None).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.group_do(Op::Move { id: id3.clone(), index: gi + 1 }, cx)
                            })))
                        })
                        .child(rule())
                        .child(row("grp-ungroup", "ungroup", "Ungroup", None).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.group_do(Op::Delete { id: id4.clone() }, cx)
                        })))
                        .child(row("grp-close", "x", "Close group", Some(close_note)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.close_group(&id5, cx)
                        })))
                        .child(
                            div()
                                .px(px(10.))
                                .pt(px(6.))
                                .pb(px(4.))
                                .text_size(px(12.))
                                .line_height(relative(1.4))
                                .text_color(t.faint)
                                .child("Groups are the same on every Mac. Which tabs are open, and folding, stay on this one."),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The tab menu's "Add to group" list, beside the menu (left of it when the window has no room on the right), for
    /// the tab with this id.
    pub fn group_submenu(
        &self,
        c: &Ctx,
        tab: gpui::EntityId,
        menu_x: f32,
        top: f32,
        w: f32,
        own: Option<&str>,
    ) -> Option<AnyElement> {
        if !self.tab_menu_sub {
            return None;
        }
        let t = c.t;
        let cx = c.cx;
        let groups: Vec<Group> = c.app.snap.groups.iter().filter(|g| Some(g.id.as_str()) != own).cloned().collect();
        if groups.is_empty() {
            return None;
        }
        let x = if menu_x + 292. + 236. <= w { menu_x + 288. } else { (menu_x - 236.).max(8.) };
        let mut list = div()
            .id("tab-menu-groups")
            .absolute()
            .left(px(x))
            .top(px(top))
            .w(px(240.))
            .max_h(px(320.))
            .overflow_y_scroll()
            .p(px(6.))
            .rounded(px(ui::R_POP))
            .border_1()
            .border_color(t.line2)
            .bg(t.panel2)
            .shadow(t.drop())
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .flex_col();
        for (k, g) in groups.into_iter().enumerate() {
            let hover = t.hover;
            let id = g.id.clone();
            let name = if g.name.is_empty() { format!("{} group", g.color.title()) } else { g.name.clone() };
            list = list.child(
                div()
                    .id(("tm-group", k))
                    .h(px(32.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(ui::R_ITEM))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .text_size(px(13.))
                    .text_color(t.ink)
                    .child(div().flex_none().w(px(15.)).flex().justify_center().child(group_dot(&t, g.color, 10.)))
                    .child(ui::line(name).flex_1())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(i) = this.tab_index(tab) {
                            this.add_tab_to_group(i, id.clone(), cx)
                        }
                    })),
            );
        }
        Some(list.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(id: &str, members: &[&str]) -> Group {
        Group {
            id: id.into(),
            name: String::new(),
            color: GroupColor::Blue,
            members: members.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn info(star: bool, thread: &str, group: Option<usize>) -> TabInfo {
        TabInfo { star, thread: Some(thread.into()), group }
    }

    #[test]
    fn stars_then_groups_in_order_then_the_rest() {
        let groups = vec![g("a", &["t3", "t1"]), g("b", &["t2"])];
        let infos = vec![
            info(false, "t9", None),
            info(false, "t1", Some(0)),
            info(false, "t2", Some(1)),
            info(true, "t5", None),
            info(false, "t3", Some(0)),
        ];
        let mut idx: Vec<usize> = (0..infos.len()).collect();
        idx.sort_by_key(|i| sort_key(&infos, &groups, *i));
        assert_eq!(idx, vec![3, 4, 1, 2, 0]);
    }

    #[test]
    fn chips_come_before_each_group_and_folding_hides_its_tabs() {
        let groups = vec![g("a", &["t1", "t2"]), g("b", &["t3"])];
        let infos = vec![
            info(true, "s", None),
            info(false, "t1", Some(0)),
            info(false, "t2", Some(0)),
            info(false, "t3", Some(1)),
            info(false, "x", None),
        ];
        assert_eq!(
            slots(&infos, &groups, &[]),
            vec![Slot::Tab(0), Slot::Chip(0), Slot::Tab(1), Slot::Tab(2), Slot::Chip(1), Slot::Tab(3), Slot::Tab(4)]
        );
        assert_eq!(
            slots(&infos, &groups, &["a".into()]),
            vec![Slot::Tab(0), Slot::Chip(0), Slot::Chip(1), Slot::Tab(3), Slot::Tab(4)]
        );
    }

    #[test]
    fn a_group_gone_during_a_drag_marks_no_place() {
        let groups = vec![g("a", &["t1"]), g("b", &["t2"])];
        let infos = vec![info(false, "t1", Some(0)), info(false, "t2", Some(1)), info(false, "t9", Some(5))];
        assert_eq!(group_drop_index(&infos, &groups, "a", Slot::Chip(1), true), Some(1));
        assert_eq!(group_drop_index(&infos, &groups, "a", Slot::Chip(0), false), Some(0), "over itself: stays");
        assert_eq!(group_drop_index(&infos, &groups, "a", Slot::Chip(7), true), None, "a chip from before");
        assert_eq!(group_drop_index(&infos, &groups, "a", Slot::Tab(2), true), None, "a tab of a group gone");
        assert_eq!(group_drop_index(&infos, &groups, "a", Slot::Tab(9), true), Some(1), "past the tabs: last");
    }

    #[test]
    fn a_drop_past_tabs_closed_during_the_drag_lands_last() {
        let grp = g("a", &["t1", "t2"]);
        let infos = vec![info(false, "t9", None), info(false, "t1", Some(0)), info(false, "t2", Some(0))];
        assert_eq!(group_add_index(&infos, &grp, 0, "t9", 0, 2), 1, "between t1 and t2");
        assert_eq!(group_add_index(&infos, &grp, 0, "t9", 0, 5), 2, "a gap from before two tabs closed");
    }

    #[test]
    fn close_group_says_what_happens_to_its_chats() {
        assert_eq!(close_group_note(&[]), "chats keep running");
        assert_eq!(close_group_note(&[false, false]), "chats keep running");
        assert_eq!(close_group_note(&[true, true]), "stops its chats");
        assert_eq!(close_group_note(&[true, false]), "stops some chats");
    }

    #[test]
    fn chips_stay_small() {
        let mut a = g("a", &["t"]);
        assert_eq!(chip_width(&a, false, 120., false), 22.);
        a.name = "A very long group name that goes on and on".into();
        assert!(chip_width(&a, false, 120., false) <= 124.);
        assert!(chip_width(&a, true, 60., false) <= 86.);
    }
}
