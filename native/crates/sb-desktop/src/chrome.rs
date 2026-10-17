//! The Lanes header: the window controls, the Board tab and one tab per open terminal, a clear "+" to start or
//! attach, then the plan meter (it opens the usage panel) and the Settings gear. Everything sits on one 40 px row,
//! centered on the traffic lights, with the same left edge as the footer below. Terminal tabs drag to reorder;
//! dropped below the row, one opens in its own window.

use gpui::{
    AnyElement, ClickEvent, Decorations, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::*, px,
};
use sb_core::fmt;

use crate::theme::{BRAND, Mode, Theme};
use crate::ui::{self, MAC};
use crate::view::{BoardView, Ctx};

/// Where the header's content starts: the same inset as every footer (ui::lead).
pub fn lead() -> f32 {
    ui::lead()
}

impl BoardView {
    pub fn titlebar(&self, c: &Ctx, decorations: Decorations, active: bool, maxed: bool) -> impl IntoElement {
        let t = c.t;
        let app = c.app;
        let cx = c.cx;
        let csd = matches!(decorations, Decorations::Client { .. });
        let ink = if active || !MAC { t.ink2 } else { t.faint };
        let updated: (String, gpui::Hsla) = if app.synced <= 0.0 {
            (if app.error.is_some() { "Not connected".into() } else { "Connecting".into() }, if app.error.is_some() { t.cold } else { t.faint })
        } else if app.error.is_some() {
            (format!("Offline, from {}", fmt::ago(app.synced, c.now)), t.cold)
        } else if !app.live {
            (format!("Saved {}", fmt::ago(app.synced, c.now)), t.faint)
        } else {
            let s = (c.now - app.synced).max(0.0).round() as i64;
            (if s < 15 { "Updated just now".into() } else if s < 90 { format!("Updated {s}s ago") } else { format!("Updated {}", fmt::ago(app.synced, c.now)) }, t.faint)
        };
        div()
            .flex_none()
            .h(px(ui::header()))
            .flex()
            .items_center()
            .bg(t.bg2)
            .border_b_1()
            .border_color(t.rule)
            .pl(px(lead()))
            .pr(px(if ui::win() { 0. } else if csd { 6. } else { ui::EDGE - 4. }))
            .child(
                div()
                    .id("drag")
                    .flex_1()
                    .h_full()
                    // Board, History and "+" always fit: "Updated ..." gives way first on a narrow window.
                    .min_w(px(if self.solo.is_some() { 330. } else { 440. }))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .when(ui::win(), |d| d.window_control_area(gpui::WindowControlArea::Drag))
                    .on_mouse_down(MouseButton::Left, |ev, window, _| {
                        if ev.click_count >= 2 {
                            if MAC { window.titlebar_double_click() } else { window.zoom_window() }
                        } else {
                            window.start_window_move();
                        }
                    })
                    // The logo and name: a button to Settings, as the gear is. Both sit on the tabs' 30 px line, the
                    // name on a line as tall as the mark so their centers meet.
                    .child({
                        let (hover, press) = (t.hover, t.press);
                        div()
                            .id("hdr-logo")
                            .flex_none()
                            .h(px(30.))
                            .ml(px(-6.))
                            .pl(px(6.))
                            .pr(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .rounded(px(ui::R_CTRL))
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .active(move |s| s.bg(press))
                            .tooltip(ui::tip("Settings"))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.plan_open = false;
                                this.add_open = false;
                                cx.notify();
                                crate::settings::open(cx);
                            }))
                            .child(ui::mark(&t, 16.))
                            .child(
                                div()
                                    .h(px(16.))
                                    .line_height(px(16.))
                                    .flex()
                                    .items_center()
                                    .font_family(BRAND)
                                    .font_weight(FontWeight::BOLD)
                                    .text_size(px(13.5))
                                    // Manrope sits high in its line: nudged down so its baseline meets Board's and its caps center on the mark.
                                    .relative()
                                    .top(px(1.))
                                    .text_color(ink)
                                    .child("Switchboard"),
                            )
                    })
                    .child(div().flex_none().w(px(1.)).h(px(18.)).bg(t.line))
                    .child(self.tab_strip(c)),
            )
            // Hidden while the tab row is crowded, unless it says the connection is down.
            .when(!self.tabs_crowded.get() || app.error.is_some(), |d| {
                d.child(div().flex_shrink(1.).min_w_0().mx(px(12.)).truncate().text_size(px(12.)).text_color(updated.1).child(updated.0))
            })
            .when(self.tabs_crowded.get() && app.error.is_none(), |d| d.child(div().w(px(12.))))
            .children(self.update_chip(c))
            .child(self.week_chip(c))
            .child(div().w(px(8.)))
            .child(settings_btn(c))
            .children(self.app_menu_btn(c))
            .when(csd, |d| d.child(div().w(px(10.))).child(crate::frame::buttons(&t, maxed)))
            .when(ui::win(), |d| {
                use gpui::WindowControlArea as A;
                d.child(div().w(px(10.)))
                    .child(ui::caption_btn(&t, "w-min", "min", A::Min, false))
                    .child(ui::caption_btn(&t, "w-max", if maxed { "restore" } else { "max" }, A::Max, false))
                    .child(ui::caption_btn(&t, "w-close", "close", A::Close, true))
            })
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                if this.add_open {
                    this.add_open = false;
                    cx.notify();
                }
            }))
    }

    /// Windows and Linux: the menu button at the end of the header, where the Mac has its menu bar.
    fn app_menu_btn(&self, c: &Ctx) -> Option<AnyElement> {
        if cfg!(target_os = "macos") {
            return None;
        }
        let t = c.t;
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .child(div().w(px(8.)))
                .child(
                    ui::icon_btn(&t, "app-menu-btn", "dots", 28.)
                        .when(self.app_menu_open, |d| d.bg(t.press))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.app_menu_open = !this.app_menu_open;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    /// The menu under that button: what the Mac's menu bar holds (Settings, History, light and dark, the terminal's
    /// text size, Quit), each with its keys.
    pub fn app_menu(&self, c: &Ctx, decorations: Decorations) -> Option<AnyElement> {
        if cfg!(target_os = "macos") || !self.app_menu_open {
            return None;
        }
        let t = c.t;
        let cx = c.cx;
        let rule = || div().my(px(4.)).mx(px(4.)).h(px(1.)).bg(t.rule);
        let item = |id: &'static str, ic: &'static str, title: &'static str, keys: String, act: fn() -> Box<dyn gpui::Action>| {
            let hover = t.hover;
            div()
                .id(id)
                .h(px(30.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(6.))
                .text_size(px(13.))
                .text_color(t.ink)
                .cursor_pointer()
                .hover(move |st| st.bg(hover))
                .child(ui::icon(ic, 15., t.faint))
                .child(div().flex_1().child(title))
                .when(!keys.is_empty(), |d| d.child(ui::kbd(&t, &keys)))
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.app_menu_open = false;
                    cx.notify();
                    window.dispatch_action(act(), cx);
                }))
        };
        let menu = div()
            .id("app-menu")
            .absolute()
            .top(px(ui::header() + 4.))
            .right(px(app_menu_right(decorations)))
            .w(px(280.))
            .p(px(6.))
            .rounded(px(10.))
            .border_1()
            .border_color(t.line2)
            .bg(t.panel2)
            .shadow(t.drop())
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .flex()
            .flex_col()
            .child(item("am-settings", "gear", "Settings", ui::mod_cap(","), || Box::new(crate::view::Settings)))
            .child(item("am-history", "history", "History", ui::mod_cap("B"), || Box::new(crate::view::Browse)))
            .child(item("am-theme", "contrast", "Switch light and dark", ui::mod_cap("Shift L"), || Box::new(crate::view::ToggleTheme)))
            .child(rule())
            .child(item("am-bigger", "zoomin", "Bigger terminal text", ui::mod_cap("+"), || Box::new(crate::term::TermBigger)))
            .child(item("am-smaller", "zoomout", "Smaller terminal text", ui::mod_cap("-"), || Box::new(crate::term::TermSmaller)))
            .child(item("am-actual", "fit", "Actual size", ui::mod_cap("0"), || Box::new(crate::term::TermSizeReset)))
            .child(rule())
            .child(item("am-about", "info", "About Switchboard", String::new(), || Box::new(crate::view::About)))
            .child(item("am-quit", "out", "Quit Switchboard", ui::mod_cap("Q"), || Box::new(crate::view::Quit)));
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.app_menu_open = false;
                    cx.notify();
                }))
                .child(menu)
                .into_any_element(),
        )
    }

    /// Board, then a tab per terminal, then "+". A terminal tab says its chat's name and machine, with its state lamp,
    /// a dot when its bell rang out of sight, and a close button (which detaches; the chat keeps running).
    fn tab_strip(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let board_on = self.tab == 0 && self.main == crate::view::Main::Board;
        let hist_on = self.tab == 0 && self.main == crate::view::Main::History;
        // Parked chats (waiting more than a day) do not count.
        let needs = c.app.snap.sessions().iter().filter(|s| !s.stale && !c.app.is_hidden(&s.key) && s.needs_you(c.now)).count();
        let view = match &self.solo {
            Some(m) => format!("{} lane", c.app.snap.label_of(m).unwrap_or_else(|| m.clone())),
            None => "Board".into(),
        };
        let mut strip = div().flex().items_center().gap(px(2.)).min_w_0().overflow_hidden().child(
            tab(&t, "tab-board", board_on)
                .flex_none()
                .pr(px(10.))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.set_tab(0, cx);
                    this.view_board(cx);
                }))
                .child(ui::icon("grid", 14., if board_on { t.ink2 } else { t.faint }))
                .child(view)
                .when(needs > 0, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .min_w(px(18.))
                            .h(px(18.))
                            .px(px(5.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .bg(t.need)
                            .text_size(px(11.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(gpui::white())
                            .child(needs.to_string()),
                    )
                }),
        );
        if self.solo.is_none() {
            strip = strip.child(
                tab(&t, "tab-history", hist_on)
                    .flex_none()
                    .pr(px(10.))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.view_history(cx)))
                    .child(ui::icon("history", 14., if hist_on { t.ink2 } else { t.faint }))
                    .child("History"),
            );
        }
        // Tab groups: each tab's star, thread and group, and the row with a chip before each group (tabgroups.rs).
        let (infos, groups) = self.tab_infos(cx);
        let folded = self.folded(cx);
        let slots = crate::tabgroups::slots(&infos, &groups, &folded);
        // A tab being dragged along the row: which one, the gap it would drop into, and the group it would join.
        // The gap is marked unless the drop would change nothing (the same place, the same group).
        let dragging = self.tab_drop.map(|d| d.0);
        let target_group = self.drop_group.as_ref().and_then(|id| groups.iter().position(|g| g.id == *id));
        let drop_at = self
            .tab_drop
            .filter(|(from, to)| {
                let same_group = infos.get(*from).is_none_or(|x| x.star || x.group == target_group);
                !same_group || (*to != *from && *to != *from + 1)
            })
            .map(|d| d.1);
        let n_tabs = self.tabs.len();
        // Where the marker goes: left of the tab at the gap when that tab is where the drop lands, else right of the
        // tab before it (the end of a group, or of the row).
        let mark: Option<(usize, bool)> = drop_at.and_then(|to| {
            let from = dragging?;
            let zone = |i: usize| infos.get(i).map(|x| if x.star { (0u8, None) } else { (1, x.group) });
            let target = if infos.get(from).is_some_and(|x| x.star) { Some((0, None)) } else { Some((1, target_group)) };
            if to < n_tabs && to != from && zone(to) == target {
                Some((to, true))
            } else {
                (0..to).rev().find(|j| *j != from).map(|j| (j, false))
            }
        });
        // A dragged chip: the group it would go before (left of its chip), or after the last group.
        let chip_mark: Option<(usize, bool)> = self.group_drop.as_ref().and_then(|(id, at)| {
            let cur = groups.iter().position(|g| g.id == *id)?;
            if *at == cur {
                return None;
            }
            let others: Vec<usize> = (0..groups.len()).filter(|g| *g != cur).collect();
            match others.get(*at) {
                Some(g) => Some((*g, true)),
                None => others.last().map(|g| (*g, false)),
            }
        });
        let accent = t.accent;
        // In the middle of the gap between two tabs (2 px, the 1 px divider, 2 px).
        let marker = move |left: bool| {
            div()
                .absolute()
                .top(px(4.))
                .bottom(px(4.))
                .when(left, |d| d.left(px(-3.5)))
                .when(!left, |d| d.right(px(-3.5)))
                .w(px(2.))
                .rounded(px(1.))
                .bg(accent)
        };
        // Chrome's separators: a thin line between neighbours, hidden beside the tab that shows and the one under the
        // pointer (and where a dragged tab would drop, which has its own marker). Gap `g` is the one before tab `g`.
        let hover = self.tab_hover;
        let shown = self.tab.checked_sub(1);
        let line2 = t.line2;
        let divider = move |g: usize| {
            let hide = [g.checked_sub(1), Some(g)].into_iter().flatten().any(|i| shown == Some(i) || hover == Some(i)) || drop_at == Some(g);
            div().flex_none().w(px(1.)).h(px(16.)).rounded(px(0.5)).when(!hide, |d| d.bg(line2))
        };
        // About the room each terminal tab gets (the window less the brand, Board, History, "+", the group chips and
        // the chips on the right). Many tabs give way in this order: "Updated ..." in the header goes, then the
        // machine labels (under 150 px), then the titles shrink (the hover star goes under 120, and under 100 the
        // close button shows only on the tab that shows and the one under the pointer), down to 64 px, which still
        // shows 4 or 5 letters. Group names shrink to about 6 letters. Past that the row scrolls sideways, with the
        // tab that shows kept in sight.
        // (With 40 px to spare: the Week chip and the header's own padding vary, and an estimate a little short scrolls the
        // first group's chip half out of sight.)
        let fixed = if self.solo.is_some() { 730. } else { 810. };
        let n_vis = slots.iter().filter(|s| matches!(s, crate::tabgroups::Slot::Tab(_))).count();
        let n_star = infos.iter().filter(|x| x.star).count() as f32;
        let chip_room = |max: f32| -> f32 {
            slots
                .iter()
                .filter_map(|s| match s {
                    crate::tabgroups::Slot::Chip(g) => Some(crate::tabgroups::chip_width(&groups[*g], folded.contains(&groups[*g].id), max, self.group_edit.as_deref() == Some(groups[*g].id.as_str())) + 5.),
                    _ => None,
                })
                .sum()
        };
        // A tab being renamed takes RENAME_W for its field; the others share what is left.
        const RENAME_W: f32 = 200.;
        let renaming = slots.iter().any(|s| matches!(s, crate::tabgroups::Slot::Tab(i) if self.renaming_tab(*i)));
        let (n_share, held) = if renaming { (n_vis.saturating_sub(1), RENAME_W + 5.) } else { (n_vis, 0.) };
        let room = |fixed: f32, max: f32| if n_share == 0 { 240. } else { (self.win_w - fixed - 16. * n_star - chip_room(max) - held) / n_share as f32 - 5. };
        let is_crowded = room(fixed, 120.) < 120.;
        self.tabs_crowded.set(is_crowded);
        let fixed = if is_crowded { fixed - 100. } else { fixed };
        let chip_max = if room(fixed, 120.) >= 120. { 120. } else { 60. };
        let per = room(fixed, chip_max);
        let slim = per < 100.;
        // Tiny: past 64 px a tab keeps only its dot (and its close button where wanted), like Chrome's pinned-size
        // tabs, down to 40 px, before the row has to scroll. The tab that shows keeps its title.
        let tiny = per < 64.;
        let tab_min = per.clamp(40., 120.);
        let show_label = per >= 150.;
        // Room above and below for the shown tab's ring, which the clip would cut.
        let mut terms = div()
            .id("tab-row")
            .flex()
            .items_center()
            .gap(px(2.))
            .py(px(2.))
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&self.tab_scroll);
        // Children of the row so far, to find the shown tab's place for keeping it in sight.
        let mut n_child = 0usize;
        let mut shown_child = None;
        strip = strip
            .on_drag_move::<TabDrag>(cx.listener(|this, ev: &gpui::DragMoveEvent<TabDrag>, _, cx| {
                // Off the row (down toward the window drop, or above it): no gap is marked.
                let (b, p) = (ev.bounds, ev.event.position);
                if p.y > b.bottom() + px(6.) || p.y < b.top() - px(6.) {
                    this.set_tab_drop(None, cx);
                    this.drop_group = None;
                }
            }))
            .on_drop::<TabDrag>(cx.listener(|this, d: &TabDrag, _, cx| this.drop_tab(d.0, cx)))
            .on_drop::<crate::tabgroups::GroupDrag>(cx.listener(|this, d: &crate::tabgroups::GroupDrag, _, cx| this.drop_group_chip(&d.id, cx)));
        let mut prev_tab = false;
        for (si, slot) in slots.iter().enumerate() {
            let i = match *slot {
                crate::tabgroups::Slot::Chip(g) => {
                    let gr = &groups[g];
                    let count = infos.iter().filter(|x| x.group == Some(g)).count();
                    let is_folded = folded.contains(&gr.id);
                    let ring = is_folded && target_group == Some(g) && dragging.is_some();
                    // A little room between the tab before and the chip, which needs no divider.
                    if prev_tab {
                        terms = terms.child(div().flex_none().w(px(3.)));
                        n_child += 1;
                    }
                    let cm = chip_mark.filter(|(x, left)| *x == g && (*left || !next_tab_in(&slots, si, &infos, Some(g))));
                    n_child += 1;
                    terms = terms.child(
                        div()
                            .relative()
                            .flex_none()
                            .child(self.group_chip(c, g, gr, count, is_folded, chip_max, ring))
                            .when_some(cm, |d, (_, left)| d.child(marker(left))),
                    );
                    prev_tab = false;
                    continue;
                }
                crate::tabgroups::Slot::Tab(i) => i,
            };
            let starred = infos[i].star;
            let grp = infos[i].group.map(|g| groups[g].color);
            let first_in_group = infos[i].group.is_some() && !prev_tab_in(&slots, si, &infos, infos[i].group);
            let hovered = self.tab_hover == Some(i);
            let tb = self.tabs[i].read(cx);
            let on = self.tab == i + 1;
            let can_star = tb.key.is_some();
            let press = t.press;
            let topic = sb_core::names::split(&tb.title).0;
            let editing = self.renaming_tab(i);
            if prev_tab {
                terms = terms.child(divider(i));
                n_child += 1;
            }
            if on {
                shown_child = Some(n_child);
            }
            n_child += 1;
            // The end of the groups for a dragged chip: right of the last group's last tab.
            let end_mark = chip_mark.is_some_and(|(g, left)| !left && infos[i].group == Some(g) && !next_tab_in(&slots, si, &infos, Some(g)));
            // Slim: the close button only where it is wanted, so the title keeps its letters.
            let no_x = slim && !on && !hovered;
            terms = terms.child(
                term_tab(&t, ("tab", i), on)
                    .min_w(px(if editing { RENAME_W } else if tiny && on { 64. } else { tab_min } + if starred { 16. } else { 0. } + if slim && on && !editing { 25. } else { 0. }))
                    .when(slim, |d| d.pl(px(10.)).pr(px(6.)).gap(px(6.)))
                    .when(tiny && !on && !editing, |d| d.justify_center().px(px(6.)))
                    .on_hover(cx.listener(move |this, h: &bool, _, cx| {
                        let next = if *h { Some(i) } else if this.tab_hover == Some(i) { None } else { this.tab_hover };
                        if next != this.tab_hover {
                            this.tab_hover = next;
                            cx.notify();
                        }
                    }))
                    // A double-click on a tab names its chat, as on a Chrome tab group.
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        this.set_tab(i + 1, cx);
                        if ev.click_count() >= 2 {
                            this.rename_tab(i, window, cx);
                        }
                    }))
                    .on_mouse_down(MouseButton::Right, cx.listener(move |this, ev: &gpui::MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.open_tab_menu(i, ev.position, cx);
                    }))
                    .on_drag(TabDrag(i), |d, _, _, cx| cx.new(|_| d.clone()))
                    // Over a tab, the drop goes before it (its left half) or after it (its right half); over a
                    // group's tabs it joins that group, over the tabs in no group it leaves its group.
                    .on_drag_move::<TabDrag>(cx.listener(move |this, ev: &gpui::DragMoveEvent<TabDrag>, _, cx| {
                        let (b, p) = (ev.bounds, ev.event.position);
                        if p.x >= b.left() && p.x < b.right() && p.y >= b.top() - px(6.) && p.y <= b.bottom() + px(6.) {
                            let from = ev.drag(cx).0;
                            this.tab_drag_over(from, i, p.x >= b.left() + b.size.width / 2., cx);
                        }
                    }))
                    .on_drag_move::<crate::tabgroups::GroupDrag>(cx.listener(move |this, ev: &gpui::DragMoveEvent<crate::tabgroups::GroupDrag>, _, cx| {
                        let (b, p) = (ev.bounds, ev.event.position);
                        if p.x >= b.left() && p.x < b.right() && p.y >= b.top() - px(6.) && p.y <= b.bottom() + px(6.) {
                            let id = ev.drag(cx).id.clone();
                            this.group_drag_over(&id, crate::tabgroups::Slot::Tab(i), true, cx);
                        }
                    }))
                    .when(dragging == Some(i), |d| d.opacity(0.45))
                    .when_some(mark.filter(|(j, _)| *j == i), |d, (_, left)| d.child(marker(left)))
                    .when(end_mark, |d| d.child(marker(false)))
                    // The group's line under its tabs, joined across the gaps between them.
                    .when_some(grp, |d, gc| {
                        d.child(
                            div()
                                .absolute()
                                .left(px(if first_in_group { 0. } else { -3. }))
                                .right(px(-3.))
                                .bottom(px(-2.))
                                .h(px(2.))
                                .rounded(px(1.))
                                .bg(crate::tabgroups::group_color(&t, gc)),
                        )
                    })
                    // The live state's dot (the connection's color until the chat is known), then the star.
                    .child(match tb.live_state(cx) {
                        Some(l) => live_dot(&t, &l, ("tab-live", i), 7.),
                        None => div().flex_none().size(px(7.)).rounded_full().bg(tb.phase.color(&t)).into_any_element(),
                    })
                    // A starred tab shows its star once, here; clicking it unstars (no second star on hover).
                    .when(starred, |d| {
                        d.child(
                            div()
                                .id(("tab-star-on", i))
                                .flex_none()
                                .ml(px(-2.))
                                .when(can_star, |d| {
                                    d.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation()).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.star_tab(i, cx);
                                    }))
                                })
                                .child(ui::icon("star-on", 11., t.cold)),
                        )
                    })
                    // The title takes the room and is what gives way (an ellipsis); the machine label keeps its size
                    // (up to 72 px) until the title is down to a few letters, and the dot, star and close button never shrink.
                    // Being renamed: the name field in its place (rename.rs), and the machine label steps aside for it.
                    // A tiny tab (its dot only) shows no title unless it is the one that shows.
                    .when(editing || !tiny || on, |d| {
                        d.child(if editing {
                            self.chat_name_field(c, "tab-rename", 22., 13.).font_weight(FontWeight::NORMAL).flex_grow(1.).flex_shrink(1.).min_w(px(150.)).into_any_element()
                        } else {
                            ui::line(topic).flex_grow(1.).flex_shrink(1.).min_w(px(12.)).into_any_element()
                        })
                    })
                    .when(!editing && show_label && !tb.label.is_empty(), |d| d.child(ui::line(tb.label.clone()).flex_shrink(0.05).min_w_0().max_w(px(72.)).text_color(t.faint).text_size(px(12.5))))
                    .when(tb.bell && !on, |d| d.child(div().flex_none().size(px(6.)).rounded_full().bg(t.need)))
                    // Under the pointer: a star button just before the close button (both fixed, so neither covers
                    // the other; the title gives way).
                    .when(hovered && can_star && !starred && per >= 120., |d| {
                        d.child(
                            div()
                                .id(("tab-star", i))
                                .flex_none()
                                .size(px(18.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(ui::R_CHIP))
                                .hover(move |s| s.bg(press))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.star_tab(i, cx);
                                }))
                                .child(ui::icon("star", 12., t.faint)),
                        )
                    })
                    .when(!no_x, |d| d.child(
                        div()
                            .id(("tab-x", i))
                            .flex_none()
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(ui::R_CHIP))
                            .hover(move |s| s.bg(press))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.close_tab(i, cx);
                            }))
                            .child(ui::icon("x", 11., if on { t.dim } else { t.faint })),
                    )),
            );
            prev_tab = true;
        }
        if prev_tab {
            terms = terms.child(divider(n_tabs));
        }
        // The terminal tabs take what room there is; "+" stays after them and is never pushed off.
        // The tab that shows is brought into sight when it changes (or the row or window does). gpui can only scroll
        // to it once the row has been laid out as a scroller, so it asks again each frame until the tab is in sight.
        if let Some(ix) = shown_child {
            let key = (self.tab, n_tabs, self.win_w.round() as i32);
            if self.tab_scrolled.get() != key {
                self.tab_scrolled.set(key);
                self.tab_scroll_pending.set(true);
            }
            if self.tab_scroll_pending.get() {
                let seen = self.tab_scroll.bounds_for_item(ix).is_some_and(|b| {
                    let view = self.tab_scroll.bounds();
                    let x = self.tab_scroll.offset().x;
                    view.size.width > px(0.) && b.left() + x >= view.left() - px(1.) && b.right() + x <= view.right() + px(1.)
                });
                if seen {
                    self.tab_scroll_pending.set(false);
                } else {
                    self.tab_scroll.scroll_to_item(ix);
                }
            }
        }
        let off = f32::from(self.tab_scroll.offset().x);
        let max = f32::from(self.tab_scroll.max_offset().x);
        let bg = t.bg2;
        let fade = |left: bool| {
            let (from, to) = if left { (bg, bg.opacity(0.)) } else { (bg.opacity(0.), bg) };
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .w(px(28.))
                .when(left, |d| d.left_0())
                .when(!left, |d| d.right_0())
                .bg(gpui::linear_gradient(90., gpui::linear_color_stop(from, 0.), gpui::linear_color_stop(to, 1.)))
        };
        let row = div()
            .relative()
            .flex()
            .min_w_0()
            .child(terms)
            .when(off < -1., |d| d.child(fade(true)))
            .when(max > 1. && off > -max + 1., |d| d.child(fade(false)));
        strip.child(row).child(
            ui::icon_btn(&t, "tab-add", "plus", 28.)
                .ml(px(2.))
                .shadow(vec![Theme::ring(t.line2)])
                .when(self.add_open, |d| d.bg(t.press))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.add_open = !this.add_open;
                    this.plan_open = false;
                    cx.notify();
                })),
        )
    }

    fn week_chip(&self, c: &Ctx) -> AnyElement {
        let t = c.t;
        let plan = &c.app.snap.plan;
        let Some(week) = plan.week else { return div().into_any_element() };
        // Old numbers stay readable but quieter, with how old they are next to them.
        let old = plan.limits_old(c.now);
        let week = if plan.week_label(c.now) == "--" { 0.0 } else { week };
        // Over the daily budget turns the chip amber; the hard limit stopping chats turns it red.
        let st = c.app.snap.budget.as_ref().map(|b| b.status()).unwrap_or_default();
        let alarm = if st.blocking() { Some(t.bad) } else if st.over_budget() { Some(t.cold) } else { None };
        let tone = match (alarm, plan.week_tone()) {
            (Some(a), _) => a,
            (None, sb_core::Tone::Ok) => t.fill,
            (None, sb_core::Tone::Warn) => t.cold,
            (None, sb_core::Tone::Bad) => t.bad,
        };
        div()
            .id("week")
            .flex_none()
            .h(px(28.))
            .px(px(10.))
            .gap(px(8.))
            .flex()
            .items_center()
            .rounded(px(ui::R_CTRL))
            .shadow(vec![Theme::ring(alarm.map(|a| a.alpha(0.7)).unwrap_or(t.line))])
            .when_some(alarm, |d, a| d.bg(a.alpha(if t.dark { 0.12 } else { 0.08 })))
            .cursor_pointer()
            .when(self.plan_open, |d| d.bg(t.press))
            .hover(move |s| s.bg(t.hover))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| {
                if this.plan_open {
                    this.plan_open = false;
                    cx.notify();
                } else {
                    this.open_plan(cx);
                }
            }))
            .child(div().text_size(px(13.)).text_color(t.ink2).child("Week"))
            .child(div().w(px(44.)).child(ui::meter(&t, (week / 100.0) as f32, tone, plan.week_projected.map(|p| (p / 100.0) as f32), 4.)))
            .child(div().text_size(px(13.)).font_weight(FontWeight::BOLD).text_color(match alarm {
                Some(a) => a,
                None if old => t.dim,
                None => t.ink,
            }).child(plan.week_label(c.now)))
            .children(plan.as_of(c.now).map(|a| div().text_size(px(12.)).text_color(t.faint).child(a)))
            .into_any_element()
    }

    /// The "+" menu under the header: a new session (a terminal on any machine and folder), or attach to a chat.
    pub fn add_menu(&self, c: &Ctx, win_w: f32) -> Option<AnyElement> {
        if !self.add_open {
            return None;
        }
        let t = c.t;
        let cx = c.cx;
        let row = |id: &'static str, ic: &'static str, title: &'static str, sub: &'static str, keys: String| {
            let hover = t.hover;
            div()
                .id(id)
                .px(px(10.))
                .py(px(8.))
                .flex()
                .items_center()
                .gap(px(12.))
                .rounded(px(ui::R_CTRL))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .child(ui::icon(ic, 16., t.dim))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(title))
                        .child(div().text_size(px(12.)).text_color(t.dim).child(sub)),
                )
                .child(ui::kbd(&t, &keys))
        };
        // Kept inside the window when many tabs push "+" toward the right edge.
        let x = (lead() + self.tab_strip_width(c) - 16.).min(win_w - 348.).max(8.);
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.add_open = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("add-menu")
                        .absolute()
                        .top(px(ui::header() + 4.))
                        .left(px(x))
                        .w(px(340.))
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
                        .child(row("add-new", "plus", "New session", "Any machine and folder, with a terminal", ui::mod_cap("N")).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                this.add_open = false;
                                this.open_new(None, window, cx);
                            },
                        )))
                        .child(row("add-attach", "term", "Open a chat in a terminal", "Attach to one that is running", ui::mod_cap("T")).on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| {
                                this.add_open = false;
                                this.open_chat_palette(window, cx);
                            },
                        )))
                        .child(row("add-history", "history", "Find an older chat", "Every chat on every machine", ui::mod_cap("B")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.add_open = false;
                                this.view_history(cx);
                            },
                        ))),
                )
                .into_any_element(),
        )
    }

    /// About where "+" sits, for its menu: the Board tab, the brand and the terminal tabs before it.
    fn tab_strip_width(&self, c: &Ctx) -> f32 {
        let brand = 16. + 7. + 92. + 10. + 1. + 10.;
        let board = 110. + if self.solo.is_some() { 60. } else { 100. };
        let tabs: f32 = self
            .tabs
            .iter()
            .map(|tb| {
                let tb = tb.read(c.cx);
                let n = sb_core::names::split(&tb.title).0.chars().count() + tb.label.chars().count();
                (n as f32 * 7.2 + 64.).clamp(120., 240.) + 5.
            })
            .sum();
        brand + board + tabs
    }
}

/// The color of a live state, the same on the board, the inspector, the lists and a terminal's tab and strip:
/// working the run color, warm green (amber in its last minute), cold amber when a big chat would re-read a lot and a
/// muted blue-gray when it is small, waiting the need color, stalled its own, parked and quiet muted. Data older than
/// LIVE_STALE (or a dropped link) is grey whatever it says.
pub fn live_color(t: &Theme, l: &sb_core::live::Live) -> gpui::Hsla {
    use sb_core::live::LiveKind as K;
    if l.stale.is_some() {
        return t.faint;
    }
    match l.kind {
        _ if l.hint => t.run,
        K::Run => t.run,
        K::Warm if l.urgent => t.cold,
        K::Warm => t.warm,
        K::Cold if l.expensive.is_some() => t.cold,
        K::Cold => crate::theme::mix(t.run, t.faint, 0.6),
        K::Wait | K::Held => t.need,
        K::Stall => t.stall,
        K::Parked | K::Idle => t.faint,
    }
}

/// The pill's color: what going back costs, or what it is doing.
pub fn pill_color(t: &Theme, l: &sb_core::live::Live) -> gpui::Hsla {
    use sb_core::live::LiveKind as K;
    if l.stale.is_some() {
        return t.faint;
    }
    match l.pill_kind {
        K::Run => t.run,
        K::Stall => t.stall,
        K::Parked => t.dim,
        K::Warm if l.urgent => t.cold,
        K::Warm => t.warm,
        K::Cold if l.expensive.is_some() => t.cold,
        K::Cold => t.dim,
        _ => t.faint,
    }
}

/// A live state's dot; working breathes. Old data draws a hollow ring: it says what was, not what is.
pub fn live_dot(t: &Theme, l: &sb_core::live::Live, id: impl Into<gpui::ElementId>, size: f32) -> AnyElement {
    let c = live_color(t, l);
    if l.stale.is_some() {
        return div().flex_none().size(px(size)).rounded_full().border_1().border_color(c).into_any_element();
    }
    let dot = div().flex_none().size(px(size)).rounded_full().bg(c);
    if l.working() {
        gpui::AnimationExt::with_animation(dot, id, gpui::Animation::new(std::time::Duration::from_millis(1600)).repeat().with_easing(gpui::pulsating_between(0.35, 1.0)), |d, k| d.opacity(k))
            .into_any_element()
    } else {
        dot.into_any_element()
    }
}

/// A big chat's cost badge, "expensive" or "re-reads 312k": a hairline pill in amber with the gauge, the same height
/// and radius as the state pill beside it, so the two read as a pair. Grey with the rest when the data is old.
pub fn cost_badge(t: &Theme, text: &str, stale: bool) -> gpui::Div {
    let c = if stale { t.faint } else { t.cold };
    div()
        .flex_none()
        .h(px(18.))
        .pl(px(5.))
        .pr(px(7.))
        .gap(px(4.))
        .flex()
        .items_center()
        .rounded(px(9.))
        .border_1()
        .border_color(c.alpha(if t.dark { 0.45 } else { 0.4 }))
        .text_size(px(11.))
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .font_features(ui::tnum())
        .text_color(c)
        .child(ui::icon("gauge", 10., c))
        .child(text.to_string())
}

/// What a terminal tab carries while it is dragged: along the tab row it reorders, dropped below it becomes a window.
#[derive(Clone)]
pub struct TabDrag(pub usize);

impl gpui::Render for TabDrag {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        div()
            .px(px(12.))
            .py(px(6.))
            .rounded(px(ui::R_CTRL))
            .bg(t.panel2)
            .shadow(t.drop())
            .text_size(px(12.5))
            .text_color(t.ink)
            .flex()
            .items_center()
            .gap(px(6.))
            .child(ui::icon("tab", 13., t.dim))
            .child("Drop in the tabs to reorder, below them for a window")
    }
}

impl BoardView {
    /// Where a dragged tab would land: (the tab dragged, the gap before tab `to`; `to` = the count for the end).
    pub fn set_tab_drop(&mut self, d: Option<(usize, usize)>, cx: &mut gpui::Context<Self>) {
        if self.tab_drop != d {
            self.tab_drop = d;
            cx.notify();
        }
    }

    /// Is terminal tab `i` (of `tabs`) starred?
    pub fn tab_starred(&self, i: usize, cx: &gpui::App) -> bool {
        let app = self.app.read(cx);
        self.tabs.get(i).and_then(|t| t.read(cx).key.clone()).is_some_and(|k| app.snap.session(&k).is_some_and(|s| s.star))
    }

    /// Star or unstar terminal tab `i`'s chat (its tab moves to or out of the starred group at once).
    pub fn star_tab(&mut self, i: usize, cx: &mut gpui::Context<Self>) {
        self.tab_menu = None;
        let Some(key) = self.tabs.get(i).and_then(|t| t.read(cx).key.clone()) else { return };
        let on = !self.tab_starred(i, cx);
        self.app.update(cx, |s, cx| s.star(&key, on, cx));
        cx.notify();
    }

    /// Moves terminal tab `from` (of `tabs`) into the gap before `to`, keeping the tab that shows. The new order is
    /// saved (it is the order tabs reopen in), and Cmd+2 to 8 follow it.
    pub fn reorder_tab(&mut self, from: usize, to: usize, cx: &mut gpui::Context<Self>) {
        let n = self.tabs.len();
        if from >= n {
            return;
        }
        let to = if to > from { to - 1 } else { to }.min(n - 1);
        if to == from {
            return;
        }
        let shown = self.tab.checked_sub(1).and_then(|i| self.tabs.get(i)).cloned();
        let moved = self.tabs.remove(from);
        self.tabs.insert(to, moved);
        if let Some(s) = shown {
            if let Some(i) = self.tabs.iter().position(|x| *x == s) {
                self.tab = i + 1;
            }
        }
        self.tab_menu = None;
        self.remember_tabs(cx);
        cx.notify();
    }
}

/// A header tab: text on the bar, the open one underlined in the accent. Pressing it never starts a window drag.
fn tab(t: &Theme, id: impl Into<gpui::ElementId>, on: bool) -> gpui::Stateful<gpui::Div> {
    let hover = t.hover;
    let accent = t.accent;
    div()
        .id(id.into())
        .relative()
        .h(px(30.))
        .pl(px(10.))
        .pr(px(4.))
        .gap(px(7.))
        .flex()
        .items_center()
        .rounded(px(ui::R_CTRL))
        .text_size(px(13.5))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .hover(move |s| s.bg(hover))
        .when(on, |d| {
            d.text_color(t.ink)
                .font_weight(FontWeight::SEMIBOLD)
                .child(div().absolute().left(px(10.)).right(px(10.)).bottom(px(-5.)).h(px(2.)).rounded(px(1.)).bg(accent))
        })
        .when(!on, |d| d.text_color(t.dim))
}

/// A terminal tab, Chrome style: its own padding, 120 to 240 px wide, a raised fill when it shows and a soft one
/// under the pointer. Board and History keep `tab` (fixed tabs with an underline).
fn term_tab(t: &Theme, id: impl Into<gpui::ElementId>, on: bool) -> gpui::Stateful<gpui::Div> {
    let hover = t.hover;
    div()
        .id(id.into())
        .relative()
        .flex_shrink(1.)
        .flex_basis(px(240.))
        .min_w(px(120.))
        .max_w(px(240.))
        .h(px(30.))
        .pl(px(12.))
        .pr(px(7.))
        .gap(px(7.))
        .flex()
        .items_center()
        .rounded(px(ui::R_CTRL))
        .text_size(px(13.))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .when(on, |d| d.bg(t.panel).shadow(vec![Theme::ring(t.line)]).text_color(t.ink).font_weight(FontWeight::SEMIBOLD))
        .when(!on, |d| d.text_color(t.dim).hover(move |s| s.bg(hover)))
}

/// How far the window buttons push the app menu button in from the right edge (its menu lines up with it).
fn app_menu_right(decorations: Decorations) -> f32 {
    if ui::win() {
        10. + 3. * 46.
    } else if matches!(decorations, Decorations::Client { .. }) {
        6. + 10. + 3. * 28. + 2. * 2.
    } else {
        ui::EDGE - 4.
    }
}

/// The room the app menu button takes in the header on Windows and Linux, so popovers under the chips to its left
/// still line up with them. None on a Mac.
pub fn app_menu_room() -> f32 {
    if cfg!(target_os = "macos") { 0. } else { 8. + 28. }
}

/// The gear at the end of the header: Settings. Light and dark live in Settings > Appearance, the View menu and
/// Shift Cmd L.
fn settings_btn(c: &Ctx) -> impl IntoElement {
    let t = c.t;
    ui::icon_btn(&t, "hdr-settings", "gear", 28.)
        .flex_none()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| {
            this.plan_open = false;
            this.add_open = false;
            cx.notify();
            crate::settings::open(cx);
        }))
}

impl BoardView {
    /// The theme from the header, Shift Cmd L or Settings: saved, synced to the server, and applied to every window.
    pub fn set_mode(&mut self, m: Mode, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        crate::theme::set_mode(m, window, cx);
        cx.notify();
    }
}

/// The slot before `si` is a tab in group `g`.
fn prev_tab_in(slots: &[crate::tabgroups::Slot], si: usize, infos: &[crate::tabgroups::TabInfo], g: Option<usize>) -> bool {
    si.checked_sub(1).and_then(|k| slots.get(k)).is_some_and(|s| matches!(s, crate::tabgroups::Slot::Tab(j) if infos[*j].group == g))
}

/// The slot after `si` is a tab in group `g`.
fn next_tab_in(slots: &[crate::tabgroups::Slot], si: usize, infos: &[crate::tabgroups::TabInfo], g: Option<usize>) -> bool {
    slots.get(si + 1).is_some_and(|s| matches!(s, crate::tabgroups::Slot::Tab(j) if infos[*j].group == g))
}
