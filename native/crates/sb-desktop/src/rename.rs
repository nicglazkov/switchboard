//! Renaming a chat. One field, opened where the chat is in front of you: in its terminal tab (the tab's menu, or a
//! double-click on its title, as Chrome names a tab group), in the details sheet (its pencil, a double-click on the
//! title, a card's right-click menu), or from the palette. Enter saves, Escape puts the name back, clicking away saves.
//! An empty name goes back to the automatic one. The new name shows at once on every view (AppState::rename), and the
//! server tells the other devices; the machine writes it where Claude Code's own /resume list reads it.

use gpui::{AnyElement, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled, div, px, relative};
use sb_core::names;

use crate::ui;
use crate::view::{BoardView, Ctx, Main};

/// Where the field shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAt {
    /// In terminal tab i, in place of its title.
    Tab(usize),
    /// At the top of the details sheet, in place of the title.
    Details,
    /// A small sheet under the title bar, when neither of those is on screen (the details popped out, History).
    Sheet,
}

#[derive(Clone, Debug)]
pub struct ChatEdit {
    /// "machine:sid"
    pub key: String,
    pub at: EditAt,
    /// The name as the server had it when the field opened: its tag ("part 2") stays on the new name.
    pub raw: String,
}

impl BoardView {
    /// Tab i's chat, in the tab itself (its menu's Rename chat, or a double-click on its title).
    pub fn rename_tab(&mut self, i: usize, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        self.tab_menu = None;
        let Some((key, raw)) = self.tabs.get(i).and_then(|t| t.read(cx).chat_name(cx)) else { return };
        if !self.app.read(cx).snap.can_rename(key.split(':').next().unwrap_or("")) {
            return;
        }
        self.begin_rename(ChatEdit { key, at: EditAt::Tab(i), raw }, window, cx);
    }

    /// From the palette, a card's menu or the details: in the tab showing the chat if that tab is in front, else in
    /// the details sheet (selected for it), else in a small sheet of its own.
    pub fn rename_chat(&mut self, key: String, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        self.menu = None;
        let shown = self.tab.checked_sub(1).filter(|i| self.tabs.get(*i).and_then(|t| t.read(cx).chat_name(cx)).is_some_and(|(k, _)| k == key));
        if let Some(i) = shown {
            return self.rename_tab(i, window, cx);
        }
        let Some(raw) = self.app.read(cx).snap.session(&key).map(|s| s.raw_title.clone()) else { return };
        let at = if self.tab == 0 && self.main == Main::Board && self.detached.is_none() { EditAt::Details } else { EditAt::Sheet };
        if at == EditAt::Details && self.sel.as_deref() != Some(key.as_str()) {
            self.select(Some(key.clone()), cx);
        }
        self.begin_rename(ChatEdit { key, at, raw }, window, cx);
    }

    fn begin_rename(&mut self, e: ChatEdit, window: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        let topic = names::split(&e.raw).0;
        self.chat_edit = Some(e);
        self.focus_tab = false;
        self.chat_name.update(cx, |n, cx| {
            n.set_value(topic, window, cx);
            n.focus(window, cx);
            n.select_all(window, cx);
        });
        cx.notify();
    }

    /// Enter (`enter`), or the field losing focus. An unchanged name sends nothing; an emptied one goes back to the
    /// automatic name on Enter only (clicking away from an empty field leaves the name as it was).
    pub fn commit_chat_name(&mut self, enter: bool, cx: &mut gpui::Context<Self>) {
        let Some(e) = self.chat_edit.take() else { return };
        self.focus_tab = matches!(e.at, EditAt::Tab(_));
        cx.notify();
        let typed = self.chat_name.read(cx).value().to_string();
        if typed.trim().is_empty() && !enter {
            return;
        }
        let name = names::given(&typed, &e.raw);
        if name.is_empty() && !typed.trim().is_empty() {
            self.app.update(cx, |s, cx| s.toast("A name needs letters or digits.", Some("It may use letters, digits, spaces and . , : ' + # ( ) - _".into()), true, cx));
            return;
        }
        if name == e.raw || (!name.is_empty() && names::split(&name).0 == names::split(&e.raw).0) {
            return;
        }
        let Some((machine, sid)) = e.key.split_once(':') else { return };
        let (machine, sid, cleared) = (machine.to_string(), sid.to_string(), name.is_empty());
        self.app.update(cx, |s, cx| {
            s.rename(
                machine,
                sid,
                name,
                move |ok, cx| {
                    if ok && cleared {
                        crate::state::app(cx).update(cx, |s, cx| s.toast("Back to its automatic name", None, false, cx));
                    }
                },
                cx,
            )
        });
    }

    /// Escape: the name stays as it was.
    pub fn cancel_chat_name(&mut self, cx: &mut gpui::Context<Self>) {
        if let Some(e) = self.chat_edit.take() {
            self.focus_tab = matches!(e.at, EditAt::Tab(_));
        }
        cx.notify();
    }

    /// Whether the field is open for this chat in this place.
    pub fn renaming(&self, key: &str, at: EditAt) -> bool {
        self.chat_edit.as_ref().is_some_and(|e| e.key == key && e.at == at)
    }

    pub fn renaming_tab(&self, i: usize) -> bool {
        self.chat_edit.as_ref().is_some_and(|e| e.at == EditAt::Tab(i))
    }

    /// The field: light (the panel's own color, a border in the accent) so the text and caret read on any header,
    /// the input centered in its height. `size` is the text size.
    pub fn chat_name_field(&self, c: &Ctx, id: &'static str, h: f32, size: f32) -> gpui::Stateful<gpui::Div> {
        let t = c.t;
        div()
            .id(id)
            .h(px(h))
            .px(px(8.))
            .flex()
            .items_center()
            .rounded(px(ui::R_CTRL))
            .bg(t.panel)
            .border_1()
            .border_color(t.accent)
            // a soft focus ring outside the border, as text fields have on the Mac
            .shadow(vec![gpui::BoxShadow { color: t.accent.opacity(0.22), offset: gpui::point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(2.5), inset: false }])
            .text_size(px(size))
            .text_color(t.ink)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .child(div().flex_1().min_w_0().h_full().flex().items_center().child(gpui_base::Input::new(&self.chat_name)))
    }

    /// The details' title while it is being renamed: the field, and a line on what Enter and an empty name do.
    pub fn rename_details(&self, c: &Ctx) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(self.chat_name_field(c, "ins-rename-field", 34., 16.).font_weight(FontWeight::SEMIBOLD))
            .child(hint(c))
            .into_any_element()
    }

    /// The small sheet, when the details are not on screen: the chat's current name, the field, Cancel and Save.
    pub fn rename_sheet(&self, c: &Ctx) -> Option<AnyElement> {
        let e = self.chat_edit.as_ref().filter(|e| e.at == EditAt::Sheet)?;
        let t = c.t;
        let cx = c.cx;
        let (label, topic) = c
            .app
            .snap
            .session(&e.key)
            .map(|s| (s.machine_label.clone(), s.topic.clone()))
            .unwrap_or_else(|| (String::new(), names::split(&e.raw).0));
        Some(
            div()
                .absolute()
                .inset_0()
                .bg(t.scrim)
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| this.cancel_chat_name(cx)))
                .child(
                    div().absolute().top(px(ui::header())).left_0().right_0().flex().justify_center().child(
                        div()
                            .id("rename-sheet")
                            .w(px(440.))
                            .max_w(relative(0.96))
                            .p(px(18.))
                            .flex()
                            .flex_col()
                            .gap(px(10.))
                            .rounded_b(px(ui::R_SHEET))
                            .border_1()
                            .border_t_0()
                            .border_color(t.line2)
                            .bg(t.panel2)
                            .shadow(t.drop())
                            .occlude()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.ink).child("Rename chat"))
                            .child(ui::line(if label.is_empty() { topic } else { format!("{topic}, on {label}") }).text_size(px(12.5)).text_color(t.dim))
                            .child(self.chat_name_field(c, "rename-sheet-field", 34., 14.))
                            .child(hint(c))
                            .child(
                                div()
                                    .mt(px(4.))
                                    .flex()
                                    .justify_end()
                                    .gap(px(8.))
                                    // On press, not on click: the field losing focus would save first.
                                    .child(ui::ghost(&t, "rename-cancel", None, None).h(px(30.)).px(px(12.)).text_size(px(13.)).child("Cancel").on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.cancel_chat_name(cx)
                                        }),
                                    ))
                                    .child(ui::primary_btn(&t, "rename-save", 30.).px(px(14.)).text_size(px(13.)).child("Save").on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.commit_chat_name(true, cx)
                                        }),
                                    )),
                            ),
                    ),
                )
                .into_any_element(),
        )
    }
}

fn hint(c: &Ctx) -> gpui::Div {
    let t = c.t;
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(5.))
        .gap_y(px(2.))
        .text_size(px(11.5))
        .line_height(relative(1.4))
        .text_color(t.faint)
        .child(ui::kbd(&t, "↵"))
        .child("saves,")
        .child(ui::kbd(&t, "esc"))
        .child("puts it back. Empty goes back to the automatic name.")
}
