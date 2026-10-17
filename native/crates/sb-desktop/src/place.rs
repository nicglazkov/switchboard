//! Where a chat's terminal lives: a tab in the header (the default), a window of its own, or a floating sheet over the
//! board. Each choice is visible where you would look for it: the inspector's header, the right-click menu, a
//! terminal tab's own menu (right-click), the floating sheet's header, and dragging a tab down out of the tab row
//! makes it a window. The shortcuts are shown next to each.

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement, Styled, Subscription,
    TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowOptions, div,
    prelude::*, px, relative, size,
};

use crate::term::{Start, TermEvent, TermTab};
use crate::theme::Theme;
use crate::ui::{self, MAC};
use crate::view::{BoardView, Ctx};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Tab,
    Window,
    Float,
}

impl Place {
    pub fn title(self) -> &'static str {
        match self {
            Place::Tab => "Open terminal in a tab",
            Place::Window => "Open terminal in a new window",
            Place::Float => "Open terminal as a floating sheet",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Place::Tab => "tab",
            Place::Window => "popout",
            Place::Float => "window",
        }
    }
    /// The shortcut shown next to it (Enter on the board opens a tab).
    pub fn keys(self) -> String {
        match self {
            Place::Tab => "↵".into(),
            Place::Window => {
                if MAC {
                    "⇧↵".into()
                } else {
                    "Shift ↵".into()
                }
            }
            Place::Float => {
                if MAC {
                    "⌥↵".into()
                } else {
                    "Alt ↵".into()
                }
            }
        }
    }
}

/// A terminal in a window of its own. Closing the window detaches; the chat keeps running.
pub struct TermWindow {
    tab: Entity<TermTab>,
    /// The window's title, for the title bar a Linux window draws when the system draws none.
    title: String,
    _subs: Vec<Subscription>,
}

impl TermWindow {
    pub fn open(tab: Entity<TermTab>, cx: &mut App) {
        let title = {
            let t = tab.read(cx);
            format!(
                "{}, {}",
                sb_core::names::split(&t.title).0,
                if t.label.is_empty() { t.machine.clone() } else { t.label.clone() }
            )
        };
        let bounds = gpui::Bounds::centered(None, size(px(980.), px(640.)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(title.clone().into()),
                appears_transparent: false,
                traffic_light_position: None,
            }),
            window_min_size: Some(size(px(480.), px(320.))),
            app_id: Some("switchboard".into()),
            window_background: WindowBackgroundAppearance::Opaque,
            window_decorations: Some(WindowDecorations::Server),
            ..Default::default()
        };
        let res = cx.open_window(opts, move |window, cx| {
            cx.new(|cx| {
                let sub = cx.subscribe_in(&tab, window, |_, _, ev: &TermEvent, window, cx| {
                    match ev {
                        TermEvent::Close => window.remove_window(),
                        // A terminal in its own window opens the flags drawer in the board window.
                        TermEvent::Flags(k) => crate::state::open_flags_in_board(k.clone(), cx),
                        TermEvent::Changed => {}
                    }
                });
                let rel = cx.on_release(|this: &mut TermWindow, cx| this.tab.update(cx, |t, _| t.detach()));
                let f = tab.read(cx).focus.clone();
                window.focus(&f, cx);
                TermWindow { tab, title, _subs: vec![sub, rel] }
            })
        });
        if let Err(e) = res {
            log::error!("could not open a terminal window: {e:#}");
        }
    }
}

impl Render for TermWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let body = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .font_family(crate::theme::UI)
            .text_color(t.ink)
            .child(self.tab.clone());
        crate::frame::framed(window, &t, &self.title, body)
    }
}

impl BoardView {
    /// Opens a chat's terminal where asked. An open tab for it moves there instead of opening a second one.
    pub fn open_terminal_in(&mut self, key: String, place: Place, cx: &mut Context<Self>) {
        match place {
            Place::Tab => self.open_terminal(key, false, cx),
            _ => {
                let alive = self.app.read(cx).snap.session(&key).map(|s| s.alive).unwrap_or(true);
                let found = self.tabs.iter().position(|t| t.read(cx).key.as_deref() == Some(key.as_str()));
                let tab = match found {
                    Some(i) => {
                        let tab = self.tabs.remove(i);
                        self.after_remove(i, cx);
                        tab
                    }
                    None => cx.new(|cx| {
                        TermTab::new(if alive { Start::Chat(key.clone()) } else { Start::Resume(key.clone()) }, cx)
                    }),
                };
                self.place(tab, place, cx);
            }
        }
        self.menu = None;
        cx.notify();
    }

    fn place(&mut self, tab: Entity<TermTab>, place: Place, cx: &mut Context<Self>) {
        match place {
            Place::Window => TermWindow::open(tab, cx),
            Place::Float => {
                if let Some(old) = self.float.replace(tab) {
                    old.update(cx, |t, _| t.detach());
                }
                self.focus_float = true;
            }
            Place::Tab => {}
        }
    }

    fn after_remove(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.tab > i {
            self.tab -= 1;
        }
        self.tab = self.tab.min(self.tabs.len());
        self.remember_tabs(cx);
    }

    /// Moves terminal tab `i` (of `tabs`) to a window or the floating sheet, still attached.
    pub fn move_tab(&mut self, i: usize, place: Place, cx: &mut Context<Self>) {
        self.tab_menu = None;
        if i >= self.tabs.len() || place == Place::Tab {
            cx.notify();
            return;
        }
        let tab = self.tabs.remove(i);
        self.after_remove(i, cx);
        self.place(tab, place, cx);
        cx.notify();
    }

    /// The floating sheet back into the tab row.
    pub fn float_to_tab(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.float.take() {
            self.tabs.push(tab);
            self.set_tab(self.tabs.len(), cx);
        }
    }

    pub fn float_to_window(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.float.take() {
            TermWindow::open(tab, cx);
            cx.notify();
        }
    }

    pub fn close_float(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.float.take() {
            tab.update(cx, |t, _| t.detach());
            cx.notify();
        }
    }

    pub fn open_tab_menu(&mut self, i: usize, at: Point<Pixels>, cx: &mut Context<Self>) {
        self.tab_menu = Some((i, at));
        self.tab_menu_sub = false;
        self.group_menu = None;
        self.add_open = false;
        cx.notify();
    }

    /// A terminal tab's own menu: where else it can live, and close.
    pub fn tab_menu_el(&self, c: &Ctx, i: usize, w: f32) -> Option<AnyElement> {
        let (_, at) = self.tab_menu?;
        let t = c.t;
        let cx = c.cx;
        let x = f32::from(at.x).min(w - 300.).max(8.);
        let starred = self.tab_starred(i, cx);
        let can_star = self.tabs.get(i).is_some_and(|tb| tb.read(cx).key.is_some());
        // Rename chat: once the tab knows its chat, and that chat's machine can keep a name.
        let can_rename = self
            .tabs
            .get(i)
            .and_then(|tb| tb.read(cx).chat_name(cx))
            .is_some_and(|(k, _)| c.app.snap.can_rename(k.split(':').next().unwrap_or("")));
        // Tab groups: a starred tab stays outside them; a tab that does not know its chat yet cannot join one.
        let (infos, groups) = self.tab_infos(cx);
        let info = infos.get(i).cloned().unwrap_or_default();
        let can_group = !starred && info.thread.is_some();
        let own = info.group.map(|g| groups[g].id.clone());
        let others = groups.iter().filter(|g| Some(&g.id) != own.as_ref()).count();
        // Where "Add to group" sits, for its list beside the menu: the menu's top and padding, the star and rename
        // rows and their rule, then "Add to new group".
        let top_rows = can_star as u8 + can_rename as u8;
        let sub_top = ui::header() + 2. + 7. + if top_rows > 0 { 32. * top_rows as f32 + 9. } else { 0. } + 32. - 6.;
        let sub = if can_group && others > 0 { self.group_submenu(c, i, x, sub_top, w, own.as_deref()) } else { None };
        let row = |id: &'static str, ic: &'static str, title: &str, keys: String| {
            let hover = t.hover;
            div()
                .id(id)
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
                .child(ui::icon(ic, 15., t.dim))
                .child(div().flex_1().child(title.to_string()))
                .when(!keys.is_empty(), |d| d.child(ui::kbd(&t, &keys)))
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.tab_menu = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .id("tab-menu")
                        .absolute()
                        .left(px(x))
                        .top(px(ui::header() + 2.))
                        .w(px(292.))
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
                        .when(can_star, |d| {
                            d.child(
                                row(
                                    "tm-star",
                                    if starred { "star-on" } else { "star" },
                                    if starred { "Unstar tab" } else { "Star tab" },
                                    String::new(),
                                )
                                .on_hover(cx.listener(|this, h: &bool, _, cx| sub_off(this, *h, cx)))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.star_tab(i, cx))),
                            )
                        })
                        .when(can_rename, |d| {
                            d.child(
                                row("tm-rename", "pencil", "Rename chat", String::new())
                                    .on_hover(cx.listener(|this, h: &bool, _, cx| sub_off(this, *h, cx)))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        this.rename_tab(i, window, cx)
                                    })),
                            )
                        })
                        .when(top_rows > 0, |d| d.child(div().my(px(4.)).mx(px(4.)).h(px(1.)).bg(t.rule)))
                        .when(can_group, |d| {
                            d.child(
                                row("tm-newgroup", "group-add", "Add to new group", String::new())
                                    .on_hover(cx.listener(|this, h: &bool, _, cx| sub_off(this, *h, cx)))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                        this.new_group_from_tab(i, window, cx)
                                    })),
                            )
                            .when(others > 0, |d| {
                                let on = self.tab_menu_sub;
                                d.child(
                                    row("tm-group", "group", "Add to group", String::new())
                                        .when(on, |d| d.bg(t.hover))
                                        .child(ui::icon("chev", 13., t.faint))
                                        .on_hover(cx.listener(|this, h: &bool, _, cx| {
                                            if *h && !this.tab_menu_sub {
                                                this.tab_menu_sub = true;
                                                cx.notify();
                                            }
                                        }))
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.tab_menu_sub = !this.tab_menu_sub;
                                            cx.notify();
                                        })),
                                )
                            })
                            .when(own.is_some(), |d| {
                                d.child(
                                    row("tm-ungroup", "ungroup", "Remove from group", String::new())
                                        .on_hover(cx.listener(|this, h: &bool, _, cx| sub_off(this, *h, cx)))
                                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            this.remove_tab_from_group(i, cx)
                                        })),
                                )
                            })
                            .child(div().my(px(4.)).mx(px(4.)).h(px(1.)).bg(t.rule))
                        })
                        .child(
                            row("tm-win", "popout", "Move to a new window", String::new())
                                .on_hover(cx.listener(|this, h: &bool, _, cx| sub_off(this, *h, cx)))
                                .on_click(
                                    cx.listener(move |this, _: &ClickEvent, _, cx| this.move_tab(i, Place::Window, cx)),
                                ),
                        )
                        .child(
                            row("tm-float", "window", "Show as a floating sheet", String::new())
                                .on_hover(cx.listener(|this, h: &bool, _, cx| sub_off(this, *h, cx)))
                                .on_click(
                                    cx.listener(move |this, _: &ClickEvent, _, cx| this.move_tab(i, Place::Float, cx)),
                                ),
                        )
                        .child(div().my(px(4.)).mx(px(4.)).h(px(1.)).bg(t.rule))
                        .child(
                            row("tm-close", "x", "Close tab (the chat keeps running)", ui::mod_cap("W"))
                                .on_hover(cx.listener(|this, h: &bool, _, cx| sub_off(this, *h, cx)))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.tab_menu = None;
                                    this.close_tab(i, cx);
                                })),
                        )
                        .child(
                            div()
                                .px(px(10.))
                                .pt(px(6.))
                                .pb(px(4.))
                                .text_size(px(12.))
                                .line_height(relative(1.4))
                                .text_color(t.faint)
                                .child("Tip: drag a tab down out of the tab row to open it in a window."),
                        ),
                )
                .children(sub)
                .into_any_element(),
        )
    }

    /// The floating sheet: a terminal over the board, with a header that says where else it can go.
    pub fn float_layer(&self, c: &Ctx, w: f32, h: f32) -> Option<AnyElement> {
        let tab = self.float.clone()?;
        let t = c.t;
        let cx = c.cx;
        let (title, label) = {
            let tb = tab.read(cx);
            (sb_core::names::split(&tb.title).0, tb.label.clone())
        };
        let fw = (w * 0.78).clamp(560., 1180.);
        let fh = (h * 0.74).clamp(360., 820.);
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(t.scrim)
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.close_float(cx)))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("float-term")
                        .w(px(fw))
                        .h(px(fh))
                        .rounded(px(ui::R_SHEET))
                        .border_1()
                        .border_color(t.line2)
                        .bg(t.panel)
                        .shadow(t.drop())
                        .overflow_hidden()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .flex_none()
                                .h(px(44.))
                                .pl(px(16.))
                                .pr(px(8.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .border_b_1()
                                .border_color(t.line)
                                .child(ui::icon("term", 15., t.dim))
                                .child(
                                    ui::wrap(title)
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(px(14.))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(t.ink),
                                )
                                .child(div().text_size(px(13.)).text_color(t.faint).child(label))
                                .child(div().w(px(8.)))
                                .child(
                                    ui::ghost(&t, "fl-tab", Some("Make it a tab"), Some("tab"))
                                        .h(px(28.))
                                        .px(px(9.))
                                        .text_size(px(12.5))
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.float_to_tab(cx))),
                                )
                                .child(
                                    ui::ghost(&t, "fl-win", Some("Open in a window"), Some("popout"))
                                        .h(px(28.))
                                        .px(px(9.))
                                        .text_size(px(12.5))
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.float_to_window(cx))),
                                )
                                .child(
                                    ui::icon_btn(&t, "fl-close", "x", 28.)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_float(cx))),
                                ),
                        )
                        .child(div().flex_1().min_h_0().flex().flex_col().child(tab)),
                )
                .into_any_element(),
        )
    }

    /// The three places, as rows for a menu (the right-click menu and the inspector use them).
    pub fn place_rows(&self, c: &Ctx, key: &str, prefix: &str) -> Vec<AnyElement> {
        let t = c.t;
        [Place::Tab, Place::Window, Place::Float]
            .into_iter()
            .map(|p| {
                let k = key.to_string();
                let hover = t.hover;
                div()
                    .id(SharedString::from(format!("{prefix}-{p:?}")))
                    .h(px(30.))
                    .px(px(8.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(ui::R_ITEM))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .text_size(px(13.))
                    .text_color(t.ink)
                    .child(ui::icon(p.icon(), 15., t.dim))
                    .child(div().flex_1().child(p.title()))
                    .child(ui::kbd(&t, &p.keys()))
                    .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| this.open_terminal_in(k.clone(), p, cx)))
                    .into_any_element()
            })
            .collect()
    }
}

/// Another row of the tab menu under the pointer: the "Add to group" list closes.
fn sub_off(this: &mut BoardView, hovered: bool, cx: &mut Context<BoardView>) {
    if hovered && this.tab_menu_sub {
        this.tab_menu_sub = false;
        cx.notify();
    }
}
