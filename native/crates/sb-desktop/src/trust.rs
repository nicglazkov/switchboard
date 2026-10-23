//! The trust question: before a chat starts (New) or moves (Continue) into a folder outside the Claude folder that
//! Claude was never let into, the server answers 409 with need "trust". This card names the folder, says what yes
//! means, and says so when the user once said no to it in Claude's own prompt. Yes sends the same request again
//! with `trust: true`, which writes the same answer accepting Claude's own prompt would.

use gpui::{App, ClickEvent, Div, FontWeight, ParentElement, Styled, Window, div, prelude::*, px, relative};

use crate::theme::{NUM, Theme};
use crate::ui;

/// What the card says.
pub struct Words {
    pub title: String,
    pub body: String,
    pub declined: Option<String>,
    pub yes: String,
}

/// The words for one folder. `what` is the action yes leads to ("start the chat", "move the chat"), `on` the
/// machine's label.
pub fn words(shown: &str, declined: bool, on: &str, what: &str) -> Words {
    let folder = if shown.is_empty() { "this folder".to_string() } else { shown.to_string() };
    Words {
        title: format!("Let Claude work in {folder}?"),
        body: format!(
            "Claude has not worked in this folder on {on} yet. If you say yes, it can read and change the files in it and run commands there, as it does in your Claude folder. Only this folder and the ones inside it."
        ),
        declined: declined
            .then(|| "You said no to this folder in Claude before. Saying yes here changes that answer.".to_string()),
        yes: format!("Yes, {what}"),
    }
}

/// The card itself, with its two buttons. The caller places it (over the board, or over the New sheet).
pub fn card(
    t: &Theme,
    id: &'static str,
    shown: &str,
    w: Words,
    yes: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    no: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let warm = t.cold;
    div()
        .w(px(460.))
        .max_w(relative(0.94))
        .p(px(22.))
        .rounded(px(ui::R_SHEET))
        .border_1()
        .border_color(t.line2)
        .bg(t.panel2)
        .shadow(t.drop())
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex_none()
                        .size(px(36.))
                        .rounded(px(ui::R_CARD))
                        .bg(t.accent.alpha(if t.dark { 0.16 } else { 0.1 }))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(ui::icon("folder", 18., t.accent)),
                )
                .child(
                    ui::wrap(w.title)
                        .flex_1()
                        .min_w_0()
                        .text_size(px(15.))
                        .line_height(relative(1.35))
                        .font_weight(FontWeight::BOLD)
                        .text_color(t.ink),
                ),
        )
        .child(
            div()
                .px(px(10.))
                .py(px(7.))
                .rounded(px(ui::R_CTRL))
                .bg(t.bg2)
                .border_1()
                .border_color(t.line)
                .font_family(NUM)
                .text_size(px(12.))
                .text_color(t.ink2)
                .child(ui::wrap(shown.to_string())),
        )
        .child(ui::wrap(w.body).text_size(px(12.5)).line_height(relative(1.5)).text_color(t.dim))
        .children(w.declined.map(|d| {
            div()
                .px(px(10.))
                .py(px(8.))
                .rounded(px(ui::R_CTRL))
                .bg(warm.alpha(if t.dark { 0.14 } else { 0.1 }))
                .flex()
                .items_start()
                .gap(px(8.))
                .child(div().pt(px(1.)).child(ui::icon("info", 13., warm)))
                .child(ui::wrap(d).flex_1().text_size(px(12.)).line_height(relative(1.45)).text_color(t.ink2))
        }))
        .child(
            div()
                .mt(px(4.))
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(
                    ui::line_btn(t, gpui::ElementId::from(format!("{id}-no")), 32.)
                        .px(px(14.))
                        .text_size(px(13.))
                        .child("Not now")
                        .on_click(no),
                )
                .child(
                    ui::primary_btn(t, gpui::ElementId::from(format!("{id}-yes")), 32.)
                        .px(px(16.))
                        .text_size(px(13.))
                        .child(w.yes)
                        .on_click(yes),
                ),
        )
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_question_names_the_folder_and_what_yes_means() {
        let w = super::words("~/Documents/site", false, "Studio", "start the chat");
        assert_eq!(w.title, "Let Claude work in ~/Documents/site?");
        assert!(w.body.contains("read and change the files"));
        assert!(w.body.contains("run commands"));
        assert!(w.body.contains("on Studio"));
        assert!(w.declined.is_none());
        assert_eq!(w.yes, "Yes, start the chat");
        let w = super::words("@Data/work", true, "Tower", "move it");
        assert!(w.declined.unwrap().starts_with("You said no"));
        assert!(!w.title.contains('\u{2014}') && !w.body.contains('\u{2014}'), "no em dashes");
    }
}
