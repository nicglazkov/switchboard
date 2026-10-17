//! The empty board when there is no server to show: the first run on a computer where no address is saved and none
//! was found on the local network, or a saved address that does not answer. Both let the user type an address.

use gpui::{AnyElement, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, Window, div, prelude::*, px, relative};

use crate::ui;
use crate::view::{BoardView, Ctx};

/// The setup guide: the Install section of the project's README.
fn guide() -> String {
    format!("{}#install", sb_core::brand::HOMEPAGE.trim_end_matches('/'))
}

impl BoardView {
    /// The empty board with an address field. `first` is the first run (nothing saved, nothing found); otherwise the
    /// saved address did not answer and `error` says so.
    pub fn server_screen(&self, c: &Ctx, first: bool, error: &str) -> AnyElement {
        let t = c.t;
        let (title, sub) = if first {
            (
                "Switchboard needs the computer that runs it".to_string(),
                "This app shows the Claude Code chats on your machines. It gets them from one computer that runs \
                 Switchboard, and none turned up on this network. Type that computer's address to connect."
                    .to_string(),
            )
        } else {
            ("Switchboard is not reachable".to_string(), error.to_string())
        };
        let using = (!first).then(|| c.app.core.servers().join(", ")).filter(|s| !s.is_empty());
        let field = div()
            .w(px(300.))
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(ui::R_CTRL))
            .border_1()
            .border_color(t.line2)
            .bg(t.bg)
            .text_size(px(13.))
            .text_color(t.ink)
            .overflow_hidden()
            .child(div().flex_1().min_w_0().child(gpui_base::Input::new(&self.server_field)));
        let connect = ui::primary_btn(&t, "server-connect", 32.)
            .child("Connect")
            .on_click(c.cx.listener(|this, _: &ClickEvent, window, cx| this.connect_server(window, cx)));
        let link = t.accent;
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(8.))
            .px(px(16.))
            .child(ui::mark(&t, 28.))
            .child(div().mt(px(6.)).text_size(px(16.)).font_weight(FontWeight::BOLD).text_color(t.ink).text_center().child(title))
            .child(div().max_w(px(460.)).text_size(px(13.)).line_height(relative(1.45)).text_color(t.dim).text_center().child(sub))
            .when_some(using, |d, u| {
                d.child(div().max_w(px(460.)).text_size(px(12.5)).text_color(t.faint).text_center().child(format!("Trying {u}. To use a different address, type it here.")))
            })
            .child(div().mt(px(10.)).flex().items_center().gap(px(8.)).child(field).child(connect))
            .when_some(self.server_err.clone(), |d, e| d.child(div().text_size(px(12.5)).text_color(t.bad).child(e)))
            .child(
                div()
                    .id("server-guide")
                    .mt(px(6.))
                    .text_size(px(12.5))
                    .text_color(link)
                    .cursor_pointer()
                    .hover(|s| s.underline())
                    .child("How to set up the computer that runs Switchboard")
                    .on_click(|_: &ClickEvent, _, cx| crate::state::open_link(cx, &guide())),
            )
            .into_any_element()
    }

    /// Connect: saves what the field holds as the server's address and tries it.
    pub fn connect_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.server_field.read(cx).value().to_string();
        match self.app.update(cx, |s, cx| s.use_server(&typed, cx)) {
            Ok(()) => {
                self.server_err = None;
                self.server_field.update(cx, |f, cx| f.set_value("", window, cx));
            }
            Err(e) => self.server_err = Some(e),
        }
        cx.notify();
    }
}
