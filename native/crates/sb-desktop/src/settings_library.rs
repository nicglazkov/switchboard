//! Settings > Library: every tool in the library as a card (what it is, the keys it needs, each machine's copy, on
//! for every new chat, its versions), and the two ways in: Import from a machine, and New tool (a chat builds it).

use gpui::{AnyElement, ClickEvent, Context, Focusable, FontWeight, IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::*, px, relative};
use sb_core::api::{ApiLibrary, ApiLibrarySources, ApiTool};

use super::{Edit, SettingsView};
use crate::library::{check_secret_value, is_this_mac, secret_command, this_mac, valid_tool_name};
use crate::state::AppState;
use crate::theme::{NUM, Theme};
use crate::ui;

/// Import: the machine picked and what it offers.
#[derive(Default)]
pub(super) struct Import {
    pub machine: Option<String>,
    pub sources: Option<Result<ApiLibrarySources, String>>,
}

/// New tool: the machine it is built on, and the start in flight.
pub(super) struct NewTool {
    pub machine: String,
    pub sending: bool,
    pub error: Option<String>,
}

fn size_text(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024. * 1024. {
        format!("{:.1} MB", b / (1024. * 1024.))
    } else if b >= 1024. {
        format!("{:.0} KB", b / 1024.)
    } else {
        format!("{bytes} bytes")
    }
}

fn kind_label(k: &str) -> String {
    match k {
        "mcp" => "MCP".into(),
        "skill" => "SKILL".into(),
        o => o.to_uppercase(),
    }
}

/// "MCP" in the board's machine-tag style: a soft accent fill.
fn soft_tag(t: &Theme, text: &str) -> gpui::Div {
    ui::state_pill(t, text, t.accent)
}

impl SettingsView {
    pub(super) fn open_import(&mut self, cx: &mut Context<Self>) {
        self.lib_new = None;
        self.lib_import = if self.lib_import.is_some() { None } else { Some(Import::default()) };
        cx.notify();
    }

    pub(super) fn open_new_tool(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.lib_import = None;
        if self.lib_new.is_some() {
            self.lib_new = None;
        } else {
            let snap = &self.app.read(cx).snap;
            let machine = snap.machines.iter().find(|m| !m.is_offline()).map(|m| m.name.clone()).unwrap_or_default();
            self.lib_new = Some(NewTool { machine, sending: false, error: None });
            self.lib_name.update(cx, |f, cx| {
                f.set_value("", window, cx);
                f.focus(window, cx);
            });
            self.lib_brief.update(cx, |f, cx| f.set_value("", window, cx));
        }
        cx.notify();
    }

    /// Import: what `machine` can give (its skills and MCP servers).
    fn import_from(&mut self, machine: String, cx: &mut Context<Self>) {
        self.lib_import = Some(Import { machine: Some(machine.clone()), sources: None });
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let m2 = machine.clone();
            let res = cx.background_executor().spawn(async move { core.library_sources(&m2) }).await;
            let _ = this.update(cx, |v, cx| {
                if let Some(imp) = v.lib_import.as_mut().filter(|i| i.machine.as_deref() == Some(machine.as_str())) {
                    imp.sources = Some(res.map_err(|e| e.to_string()));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Adds one skill or MCP server to the library (Import, or a machine's MCP servers).
    pub(super) fn import_one(&mut self, machine: String, kind: &'static str, source: String, cx: &mut Context<Self>) {
        self.app.update(cx, |s, cx| s.lib_import(&machine, kind, &source, |_, _| {}, cx));
    }

    fn new_tool_go(&mut self, cx: &mut Context<Self>) {
        let name = self.lib_name.read(cx).value().trim().to_string();
        let brief = self.lib_brief.read(cx).value().trim().to_string();
        let Some(n) = self.lib_new.as_mut() else { return };
        if n.sending {
            return;
        }
        if !valid_tool_name(&name) {
            n.error = Some("Give it a name: lowercase letters, digits and dashes, up to 40.".into());
            return cx.notify();
        }
        if brief.is_empty() {
            n.error = Some("Say what it should do, so the chat knows what to build.".into());
            return cx.notify();
        }
        if n.machine.is_empty() {
            n.error = Some("Pick a machine to build it on.".into());
            return cx.notify();
        }
        n.sending = true;
        n.error = None;
        let machine = n.machine.clone();
        let me = cx.entity().downgrade();
        self.app.update(cx, |s, cx| {
            s.lib_new(
                &machine,
                &name,
                &brief,
                move |res, cx| {
                    let _ = me.update(cx, |v, cx| {
                        match res {
                            Ok(()) => v.lib_new = None,
                            Err(e) => {
                                if let Some(n) = v.lib_new.as_mut() {
                                    n.sending = false;
                                    n.error = Some(e);
                                }
                            }
                        }
                        cx.notify();
                    });
                },
                cx,
            )
        });
        cx.notify();
    }

    /// Set key on this Mac: takes the value out of the field (emptied at once), saves it in the keychain, and closes
    /// on success. A value the agent would refuse stays in the field to fix.
    pub(super) fn save_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((tool, _, secret)) = self.lib_key.clone() else { return };
        let value = self.lib_secret.read(cx).value().to_string();
        if let Err(e) = check_secret_value(&value) {
            self.lib_key_err = Some(if value.is_empty() { format!("Paste the value of {secret} first.") } else { e });
            return cx.notify();
        }
        self.lib_key_err = None;
        self.lib_secret.update(cx, |f, cx| f.set_value("", window, cx));
        let me = cx.entity().downgrade();
        let k = self.lib_key.clone();
        self.app.update(cx, |s, cx| {
            s.lib_set_key_here(
                &tool,
                &secret,
                value,
                move |res, cx| {
                    let _ = me.update(cx, |v, cx| {
                        if v.lib_key == k {
                            match res {
                                Ok(()) => v.lib_key = None,
                                Err(e) => v.lib_key_err = Some(e),
                            }
                        }
                        cx.notify();
                    });
                },
                cx,
            )
        });
        cx.notify();
    }

    // ---------- drawing ----------

    pub(super) fn library(&self, t: &Theme, window: &Window, cx: &Context<Self>) -> AnyElement {
        let app = self.app.read(cx);
        let mut body = div().child(Self::head(
            t,
            "Library",
            "Tools you built, kept on the server so a chat on any machine can use them. A chat gets a tool when you attach it, or when the tool is on for every new chat.",
        ));
        let has_lib = matches!(app.library, Some(Ok(_)));
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .child(
                    ui::primary_btn(t, "lib-import", 34.)
                        .px(px(14.))
                        .text_size(px(13.))
                        .when(!has_lib, |d| d.opacity(0.5))
                        .child(ui::icon("plus", 14., t.on_accent))
                        .child("Import...")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if has_lib {
                                this.open_import(cx)
                            }
                        })),
                )
                .child(
                    ui::line_btn(t, "lib-new", 34.)
                        .px(px(14.))
                        .text_size(px(13.))
                        .when(!has_lib, |d| d.opacity(0.5))
                        .child(ui::icon("spark", 14., t.dim))
                        .child("New tool...")
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            if has_lib {
                                this.open_new_tool(window, cx)
                            }
                        })),
                ),
        );
        if let Some(imp) = &self.lib_import {
            body = body.child(self.import_panel(t, imp, app, cx));
        }
        if let Some(n) = &self.lib_new {
            body = body.child(self.new_tool_panel(t, n, app, window, cx));
        }
        match &app.library {
            None => body.child(div().mt(px(22.)).text_size(px(13.)).text_color(t.faint).child("Reading the library")).into_any_element(),
            Some(Err(e)) => body
                .child(Self::group(
                    t,
                    "No library here yet",
                    None,
                    vec![Self::crow(t, "Could not read the library", Some(format!("{e}. The server may need an update to have one.")), None)],
                ))
                .into_any_element(),
            Some(Ok(l)) if l.tools.is_empty() => body.child(Self::empty_library(t)).into_any_element(),
            Some(Ok(l)) => {
                let mut list = div().flex().flex_col();
                for tool in &l.tools {
                    list = list.child(self.tool_card(t, l, tool, app, window, cx));
                }
                body.child(list).child(div().h(px(20.))).into_any_element()
            }
        }
    }

    fn empty_library(t: &Theme) -> AnyElement {
        div()
            .mt(px(22.))
            .py(px(36.))
            .px(px(24.))
            .rounded(px(ui::R_CARD))
            .border_1()
            .border_color(t.line)
            .bg(t.bg)
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .child(div().size(px(44.)).rounded(px(11.)).bg(t.accent.alpha(if t.dark { 0.16 } else { 0.1 })).flex().items_center().justify_center().child(ui::icon("cube", 22., t.accent)))
            .child(div().mt(px(4.)).text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.ink).child("No tools yet"))
            .child(
                ui::wrap("Import a skill or an MCP server from one of your machines, or describe a new tool and a chat builds it.")
                    .max_w(px(420.))
                    .text_center()
                    .text_size(px(13.))
                    .line_height(relative(1.45))
                    .text_color(t.dim),
            )
            .into_any_element()
    }

    /// One row inside a tool card: a hairline above all but the first.
    fn card_row(t: &Theme, first: bool, el: impl IntoElement) -> gpui::Div {
        div().px(px(14.)).py(px(11.)).when(!first, |d| d.border_t_1().border_color(t.line)).child(el)
    }

    fn tool_card(&self, t: &Theme, lib: &ApiLibrary, tool: &ApiTool, app: &AppState, window: &Window, cx: &Context<Self>) -> AnyElement {
        let name = tool.name.clone();
        let soft = t.accent.alpha(if t.dark { 0.16 } else { 0.1 });
        let mut card = div().mt(px(16.)).rounded(px(ui::R_CARD)).border_1().border_color(t.line).bg(t.bg).flex().flex_col();

        // What it is
        let mut title = div().flex().flex_wrap().items_center().gap(px(7.)).child(ui::wrap(name.clone()).text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.ink));
        for k in &tool.kinds {
            title = title.child(soft_tag(t, &kind_label(k)));
        }
        if tool.latest > 0 {
            title = title.child(div().font_family(NUM).text_size(px(12.)).text_color(t.dim).child(format!("v{}", tool.latest)));
        }
        let desc = if tool.description.trim().is_empty() {
            ui::wrap("No description.").text_size(px(13.)).text_color(t.faint)
        } else {
            ui::wrap(tool.description.clone()).text_size(px(13.)).line_height(relative(1.45)).text_color(t.dim)
        };
        card = card.child(
            div()
                .px(px(14.))
                .pt(px(14.))
                .pb(px(12.))
                .flex()
                .items_start()
                .gap(px(12.))
                .child(div().flex_none().size(px(34.)).rounded(px(8.)).bg(soft).flex().items_center().justify_center().child(ui::icon("cube", 17., t.accent)))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(4.)).child(title).child(desc)),
        );

        // The keys it needs, and the warning for a skill that holds them
        if !tool.secrets.is_empty() {
            let mut keys = div().flex().flex_col().gap(px(6.)).child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.dim).child("Keys it needs"));
            for s in &tool.secrets {
                keys = keys.child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(ui::icon("lock", 12., t.faint))
                        .child(div().font_family(NUM).text_size(px(12.5)).text_color(t.ink).child(s.name.clone()))
                        .when(!s.about.is_empty(), |d| d.child(ui::wrap(s.about.clone()).text_size(px(12.5)).text_color(t.faint))),
                );
            }
            keys = keys.child(ui::wrap("Each machine keeps its own copy in its keychain. The values never go to the server.").text_size(px(12.)).text_color(t.faint));
            card = card.child(Self::card_row(t, false, keys));
        }
        if tool.skill_secrets {
            card = card.child(Self::card_row(
                t,
                false,
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(px(ui::R_CTRL))
                    .bg(t.need.alpha(if t.dark { 0.14 } else { 0.1 }))
                    .child(div().mt(px(1.)).child(ui::icon("warn", 14., t.need)))
                    .child(ui::wrap("This skill uses keys Claude can read; make it an MCP to keep them hidden").flex_1().text_size(px(12.5)).line_height(relative(1.4)).text_color(t.ink)),
            ));
        }

        // On for every new chat
        let def = lib.is_default(&name);
        let def_busy = app.lib_busy.contains(&format!("default:{name}"));
        let n1 = name.clone();
        card = card.child(Self::card_row(
            t,
            false,
            Self::crow(
                t,
                "On for every new chat",
                Some(if def { "New chats on every machine start with it.".into() } else { "Only chats you attach it to get it.".into() }),
                Some(
                    ui::switch(t, SharedString::from(format!("lib-def-{name}")), def, def_busy)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.app.update(cx, |s, cx| s.lib_default(&n1, !def, cx))))
                        .into_any_element(),
                ),
            ),
        ));

        // In Auto, and when to use it, on a server that knows Auto.
        if lib.has_auto() {
            let auto = tool.auto != Some(false);
            let when = tool.when.clone().unwrap_or_default();
            let busy = app.lib_busy.contains(&format!("auto:{name}"));
            let (n2, w2) = (name.clone(), when.clone());
            card = card.child(Self::card_row(
                t,
                false,
                Self::crow(
                    t,
                    "Include in Auto",
                    Some(if auto { "Chats that start with Auto tools get it.".into() } else { "Auto leaves it out; pick it to use it.".into() }),
                    Some(
                        ui::switch(t, SharedString::from(format!("lib-auto-{name}")), auto, busy)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.app.update(cx, |s, cx| s.lib_auto(&n2, !auto, &w2, cx))))
                            .into_any_element(),
                    ),
                ),
            ));
            let editing = self.edit == Some(Edit::When(name.clone()));
            let n3 = name.clone();
            let body = if editing {
                let focused = self.field.focus_handle(cx).is_focused(window);
                self.edit_row(t, ("lib-when", "lib-when-save", "lib-when-x"), focused, cx)
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(13.5)).text_color(t.ink).child("When to use"))
                            .child(if when.trim().is_empty() {
                                ui::wrap("Not set, so Claude goes by the description. One line tells it when to reach for this.").text_size(px(12.5)).text_color(t.faint)
                            } else {
                                ui::wrap(when.clone()).text_size(px(12.5)).line_height(relative(1.4)).text_color(t.dim)
                            }),
                    )
                    .child(ui::line_btn(t, SharedString::from(format!("lib-when-edit-{name}")), 30.).px(px(12.)).text_size(px(13.)).child("Edit").on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| this.start_edit(Edit::When(n3.clone()), when.clone(), window, cx),
                    )))
                    .into_any_element()
            };
            card = card.child(Self::card_row(t, false, body));
        }

        // Each machine
        let mut machines = div().flex().flex_col().gap(px(2.)).child(div().mb(px(4.)).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.dim).child("Machines"));
        for m in &app.snap.machines {
            machines = machines.child(self.machine_line(t, lib, tool, m, app, window, cx));
        }
        card = card.child(Self::card_row(t, false, machines));

        // Versions and Delete
        card = card.child(Self::card_row(t, false, self.versions_and_delete(t, tool, app, cx)));
        card.into_any_element()
    }

    /// One machine's copy of a tool: connected or not, the version there, keys it still needs (with Set key...).
    fn machine_line(&self, t: &Theme, lib: &ApiLibrary, tool: &ApiTool, m: &sb_core::Machine, app: &AppState, window: &Window, cx: &Context<Self>) -> AnyElement {
        let name = tool.name.clone();
        let mut st = lib.on_machine(&m.name, &name).cloned().unwrap_or_default();
        // This Mac: Set key saves straight into its keychain, and a key saved here stops showing as needed at once.
        let here = this_mac().is_some_and(|h| is_this_mac(&m.name, m.os.as_deref(), h));
        if here {
            st.missing_secrets.retain(|s| !app.lib_keys_here.contains(&format!("{name}/{s}")));
        }
        let offline = m.is_offline();
        let busy = app.lib_busy.contains(&format!("connect:{name}@{}", m.name));
        let on = st.connected;
        let (status, tone) = if let Some(e) = st.error.clone().filter(|e| !e.is_empty()) {
            (e, t.bad)
        } else if busy {
            (if on { format!("Putting v{} there", tool.latest) } else { "Taking it off".into() }, t.dim)
        } else if !on {
            ("Not on this machine".to_string(), t.faint)
        } else {
            match st.installed {
                Some(v) if v == tool.latest || tool.latest == 0 => (format!("v{v} installed"), t.dim),
                Some(v) => (format!("v{v} installed, v{} on its way", tool.latest), t.dim),
                None => (format!("v{} on its way", tool.latest), t.dim),
            }
        };
        let status = if offline { format!("{status}. Offline now.") } else { status };
        let (n1, m1) = (name.clone(), m.name.clone());
        let mut note = div().flex().flex_wrap().items_center().gap(px(8.)).child(div().text_size(px(12.5)).text_color(tone).child(status));
        if on {
            for s in &st.missing_secrets {
                let showing = self.lib_key.as_ref().is_some_and(|(a, b, c)| *a == name && *b == m.name && c == s);
                let (n2, m2, s2) = (name.clone(), m.name.clone(), s.clone());
                note = note
                    .child(div().text_size(px(12.5)).text_color(t.need).child(format!("needs {s}")))
                    .child(
                        ui::ghost(t, SharedString::from(format!("lib-key-{name}-{}-{s}", m.name)), Some("Set key..."), Some("lock"))
                            .text_color(t.accent)
                            .when(showing, |d| d.bg(t.sel))
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                let k = (n2.clone(), m2.clone(), s2.clone());
                                this.lib_key = if this.lib_key.as_ref() == Some(&k) { None } else { Some(k) };
                                this.lib_key_err = None;
                                // The field starts empty each time, and has focus when it is this Mac's.
                                let open = this.lib_key.is_some();
                                this.lib_secret.update(cx, |f, cx| {
                                    f.set_value("", window, cx);
                                    if open && here {
                                        f.focus(window, cx);
                                    }
                                });
                                cx.notify();
                            })),
                    );
            }
        }
        let row = div()
            .py(px(6.))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().flex_none().child(ui::machine_icon(&m.name, 18., if offline { t.faint } else { t.dim })))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).child(ui::wrap(m.label.clone()).text_size(px(13.5)).text_color(t.ink)).child(note))
            .child(
                ui::switch(t, SharedString::from(format!("lib-on-{name}-{}", m.name)), on, busy || (offline && !on))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if !busy {
                            this.app.update(cx, |s, cx| s.lib_connect(&n1, &m1, !on, cx))
                        }
                    })),
            );
        let mut out = div().flex().flex_col().child(row);
        let open = self.lib_key.as_ref().filter(|(a, b, _)| *a == name && *b == m.name);
        // Set key on this Mac: a masked field that saves straight into its keychain.
        if let Some((_, _, secret)) = open.filter(|_| here) {
            let saving = app.lib_busy.contains(&format!("key:{name}/{secret}"));
            let focused = self.lib_secret.focus_handle(cx).is_focused(window);
            return out
                .child(
                    div()
                        .ml(px(30.))
                        .mb(px(6.))
                        .p(px(10.))
                        .rounded(px(ui::R_CTRL))
                        .bg(t.panel)
                        .border_1()
                        .border_color(t.line)
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            ui::wrap(format!("Paste {secret}. It goes straight into this Mac's keychain, where the agent reads it. It never goes to the server."))
                                .text_size(px(12.5))
                                .line_height(relative(1.4))
                                .text_color(t.dim),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .id(SharedString::from(format!("lib-secret-{name}")))
                                        .flex_1()
                                        .min_w(px(160.))
                                        .h(px(32.))
                                        .px(px(10.))
                                        .flex()
                                        .items_center()
                                        .rounded(px(ui::R_CTRL))
                                        .border_1()
                                        .border_color(if focused { t.accent } else { t.line2 })
                                        .bg(t.bg)
                                        .font_family(NUM)
                                        .text_size(px(13.))
                                        .text_color(t.ink)
                                        .child(gpui_base::Input::new(&self.lib_secret)),
                                )
                                .child(
                                    ui::primary_btn(t, SharedString::from(format!("lib-secret-save-{name}")), 32.)
                                        .px(px(12.))
                                        .text_size(px(13.))
                                        .when(saving, |d| d.opacity(0.6))
                                        .child(if saving { "Saving" } else { "Save" })
                                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.save_key(window, cx))),
                                )
                                .child(ui::ghost(t, SharedString::from(format!("lib-secret-x-{name}")), Some("Cancel"), None).h(px(32.)).px(px(10.)).text_size(px(13.)).on_click(
                                    cx.listener(|this, _: &ClickEvent, window, cx| {
                                        this.lib_key = None;
                                        this.lib_key_err = None;
                                        this.lib_secret.update(cx, |f, cx| f.set_value("", window, cx));
                                        cx.notify();
                                    }),
                                )),
                        )
                        .when_some(self.lib_key_err.clone(), |d, e| d.child(ui::wrap(e).text_size(px(12.5)).line_height(relative(1.4)).text_color(t.bad))),
                )
                .into_any_element();
        }
        // Set key elsewhere: the terminal can only attach to chats, so this shows the command to run there, with Copy.
        if let Some((_, _, secret)) = open {
            let cmd = secret_command(m.os.as_deref(), &name, secret);
            let c2 = cmd.clone();
            out = out.child(
                div()
                    .ml(px(30.))
                    .mb(px(6.))
                    .p(px(10.))
                    .rounded(px(ui::R_CTRL))
                    .bg(t.panel)
                    .border_1()
                    .border_color(t.line)
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        ui::wrap(format!("Run this in a terminal on {}. It asks for the value without showing it, and keeps it in that machine's keychain.", m.label))
                            .text_size(px(12.5))
                            .line_height(relative(1.4))
                            .text_color(t.dim),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .px(px(10.))
                                    .py(px(7.))
                                    .rounded(px(ui::R_ITEM))
                                    .bg(t.bg)
                                    .border_1()
                                    .border_color(t.line2)
                                    .child(ui::wrap(cmd).font_family(NUM).text_size(px(12.)).text_color(t.ink)),
                            )
                            .child(ui::line_btn(t, SharedString::from(format!("lib-copy-{name}-{}", m.name)), 30.).child(ui::icon("copy", 13., t.dim)).child("Copy").on_click(cx.listener(
                                move |_, _: &ClickEvent, _, cx| {
                                    crate::state::copy_text(cx, c2.clone());
                                    crate::state::app(cx).update(cx, |s, cx| s.toast("Copied the command", None, false, cx));
                                },
                            ))),
                    ),
            );
        }
        out.into_any_element()
    }

    fn versions_and_delete(&self, t: &Theme, tool: &ApiTool, app: &AppState, cx: &Context<Self>) -> AnyElement {
        let name = tool.name.clone();
        let open = self.lib_hist.contains(&name);
        let n = tool.versions.len();
        let (n1, n2, n3) = (name.clone(), name.clone(), name.clone());
        let now = AppState::now();
        let arming = self.lib_deleting.as_ref().is_some_and(|(x, at)| *x == name && now - at < 4.);
        let deleting = app.lib_busy.contains(&format!("delete:{name}"));
        let head = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                ui::ghost(t, SharedString::from(format!("lib-hist-{name}")), None, None)
                    .ml(px(-6.))
                    .h(px(28.))
                    .px(px(6.))
                    .text_size(px(12.5))
                    .child(ui::icon(if open { "down" } else { "chev" }, 12., t.dim))
                    .child(match n {
                        0 => "Versions".to_string(),
                        1 => "1 version".to_string(),
                        n => format!("{n} versions"),
                    })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if !this.lib_hist.remove(&n1) {
                            this.lib_hist.insert(n1.clone());
                        }
                        cx.notify();
                    })),
            )
            .child(ui::spacer())
            .child(
                ui::line_btn(t, SharedString::from(format!("lib-del-{name}")), 28.)
                    .px(px(10.))
                    .text_size(px(12.5))
                    .text_color(t.bad)
                    .when(arming, |d| d.bg(t.bad.alpha(0.12)))
                    .when(deleting, |d| d.opacity(0.55))
                    .child(if deleting { "Deleting" } else if arming { "Click again to delete" } else { "Delete" })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let now = AppState::now();
                        if this.lib_deleting.as_ref().is_some_and(|(x, at)| *x == n2 && now - at < 4.) {
                            this.lib_deleting = None;
                            let n = n3.clone();
                            this.app.update(cx, |s, cx| s.lib_delete(&n, cx));
                        } else {
                            this.lib_deleting = Some((n2.clone(), now));
                        }
                        cx.notify();
                    })),
            );
        let mut out = div().flex().flex_col().gap(px(4.)).child(head);
        if arming {
            out = out.child(ui::wrap("Deleting takes it off every machine and out of every chat, and removes all its versions.").text_size(px(12.)).text_color(t.faint));
        }
        if open {
            let rolling = app.lib_busy.contains(&format!("rollback:{name}"));
            let mut vs: Vec<_> = tool.versions.iter().collect();
            vs.sort_by(|a, b| b.v.cmp(&a.v));
            let mut list = div().mt(px(4.)).flex().flex_col();
            for (i, v) in vs.into_iter().enumerate() {
                let latest = v.v == tool.latest;
                let (n4, ver) = (name.clone(), v.v);
                let by = app.snap.label_of(&v.by).unwrap_or_else(|| v.by.clone());
                let mut sub = vec![];
                if v.at > 0. {
                    sub.push(sb_core::fmt::md(v.at));
                }
                if !by.is_empty() {
                    sub.push(format!("from {by}"));
                }
                if v.size > 0 {
                    sub.push(size_text(v.size));
                }
                list = list.child(
                    div()
                        .py(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .when(i > 0, |d| d.border_t_1().border_color(t.rule))
                        .child(div().flex_none().w(px(36.)).font_family(NUM).text_size(px(13.)).font_weight(FontWeight::BOLD).text_color(if latest { t.ink } else { t.dim }).child(format!("v{}", v.v)))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(div().text_size(px(12.5)).text_color(t.dim).child(sub.join(", ")))
                                .when(!v.note.is_empty(), |d| d.child(ui::wrap(v.note.clone()).text_size(px(12.5)).text_color(t.ink2))),
                        )
                        .child(if latest {
                            soft_tag(t, "Latest").into_any_element()
                        } else {
                            ui::line_btn(t, SharedString::from(format!("lib-roll-{name}-{}", v.v)), 28.)
                                .px(px(10.))
                                .text_size(px(12.5))
                                .when(rolling, |d| d.opacity(0.5))
                                .child("Make latest")
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if !rolling {
                                        let n = n4.clone();
                                        this.app.update(cx, |s, cx| s.lib_rollback(&n, ver, cx));
                                    }
                                }))
                                .into_any_element()
                        }),
                );
            }
            if n == 0 {
                list = list.child(div().py(px(6.)).text_size(px(12.5)).text_color(t.faint).child("No versions listed."));
            }
            out = out.child(list).child(ui::wrap("Every version is kept. Machines that have the tool get the one marked latest; running chats keep the version they started with.").mt(px(2.)).text_size(px(12.)).text_color(t.faint));
        }
        out.into_any_element()
    }

    /// A panel under the header buttons, like Add one over SSH.
    fn panel(t: &Theme, id: &'static str, title: &str, sub: &str, cx: &Context<Self>, close: impl Fn(&mut Self, &mut Context<Self>) + 'static) -> gpui::Div {
        div()
            .mt(px(16.))
            .p(px(16.))
            .rounded(px(ui::R_CARD))
            .border_1()
            .border_color(t.line)
            .bg(t.bg)
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(title.to_string()))
                            .child(ui::wrap(sub.to_string()).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.dim)),
                    )
                    .child(ui::icon_btn(t, id, "x", 26.).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        close(this, cx);
                        cx.notify();
                    }))),
            )
    }

    /// The machines to pick from, as a segmented row (offline ones dimmed and not pickable).
    fn machine_seg(t: &Theme, prefix: &str, cur: Option<&str>, app: &AppState, cx: &Context<Self>, pick: impl Fn(&mut Self, String, &mut Context<Self>) + Clone + 'static) -> AnyElement {
        let mut seg = div().flex().flex_wrap().gap(px(6.));
        for m in &app.snap.machines {
            let on = cur == Some(m.name.as_str());
            let off = m.is_offline();
            let (name, pick) = (m.name.clone(), pick.clone());
            seg = seg.child(
                ui::seg_btn(t, SharedString::from(format!("{prefix}-{}", m.name)), on, off)
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(px(ui::R_CTRL))
                    .border_1()
                    .border_color(if on { t.accent.alpha(0.6) } else { t.line })
                    .child(ui::machine_icon(&m.name, 14., if on { t.ink } else { t.dim }))
                    .child(m.label.clone())
                    .when(off, |d| d.child(div().text_color(t.faint).child("offline")))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if !off {
                            pick(this, name.clone(), cx)
                        }
                    })),
            );
        }
        seg.into_any_element()
    }

    fn import_panel(&self, t: &Theme, imp: &Import, app: &AppState, cx: &Context<Self>) -> AnyElement {
        let mut p = Self::panel(
            t,
            "lib-imp-x",
            "Import from a machine",
            "Pick a machine, then a skill or an MCP server it has. A skill brings its whole folder. An MCP server keeps its command; its env values stay behind, and each one becomes a key to set on every machine.",
            cx,
            |this, _| this.lib_import = None,
        )
        .child(Self::machine_seg(t, "lib-imp-m", imp.machine.as_deref(), app, cx, |this, m, cx| this.import_from(m, cx)));
        let Some(machine) = imp.machine.clone() else { return p.into_any_element() };
        let label = app.snap.label_of(&machine).unwrap_or_else(|| machine.clone());
        let lib = app.lib();
        let exists = |n: &str| lib.and_then(|l| l.tool(n)).map(|x| x.latest);
        match &imp.sources {
            None => p = p.child(div().text_size(px(13.)).text_color(t.faint).child(format!("Asking {label} what it has"))),
            Some(Err(e)) => p = p.child(ui::wrap(format!("Could not ask {label}. {e}")).text_size(px(13.)).text_color(t.bad)),
            Some(Ok(s)) if s.skills.is_empty() && s.mcp.is_empty() => p = p.child(div().text_size(px(13.)).text_color(t.faint).child(format!("{label} has no skills or MCP servers to import."))),
            Some(Ok(s)) => {
                let row = |id: String, title: String, detail: String, kind: &'static str, source: String| {
                    let busy = app.lib_busy.contains(&format!("import:{machine}/{kind}/{source}"));
                    let have = exists(&title);
                    let m2 = machine.clone();
                    div()
                        .py(px(8.))
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .border_t_1()
                        .border_color(t.rule)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(8.))
                                        .child(ui::wrap(title.clone()).text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink))
                                        .when(have.is_some(), |d| d.child(ui::tag(t, "In the library"))),
                                )
                                .child(ui::wrap(detail).font_family(NUM).text_size(px(11.5)).line_height(relative(1.4)).text_color(t.faint)),
                        )
                        .child(
                            ui::line_btn(t, SharedString::from(id), 30.)
                                .px(px(12.))
                                .text_size(px(13.))
                                .when(busy, |d| d.opacity(0.55))
                                .child(if busy {
                                    "Adding".to_string()
                                } else {
                                    match have {
                                        Some(v) => format!("Add as v{}", v + 1),
                                        None => "Add".to_string(),
                                    }
                                })
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if !busy {
                                        this.import_one(m2.clone(), kind, source.clone(), cx)
                                    }
                                })),
                        )
                };
                if !s.skills.is_empty() {
                    let mut list = div().flex().flex_col().child(ui::lbl("Skills", t.label).pb(px(6.)));
                    for (i, k) in s.skills.iter().enumerate() {
                        let detail = format!("{}{}", k.path, if k.files > 0 { format!(", {} files, {}", k.files, size_text(k.size)) } else { String::new() });
                        list = list.child(row(format!("lib-imp-s-{i}"), k.name.clone(), detail, "skill", k.name.clone()));
                    }
                    p = p.child(list);
                }
                if !s.mcp.is_empty() {
                    let mut list = div().flex().flex_col().child(ui::lbl("MCP servers", t.label).pb(px(6.)));
                    for (i, k) in s.mcp.iter().enumerate() {
                        let mut detail = k.line();
                        if !k.env_names.is_empty() {
                            detail.push_str(&format!("  (keys: {})", k.env_names.join(", ")));
                        }
                        list = list.child(row(format!("lib-imp-m-{i}"), k.name.clone(), detail, "mcp", k.name.clone()));
                    }
                    p = p.child(list);
                }
            }
        }
        p.into_any_element()
    }

    fn new_tool_panel(&self, t: &Theme, n: &NewTool, app: &AppState, window: &Window, cx: &Context<Self>) -> AnyElement {
        let name_focused = self.lib_name.focus_handle(cx).is_focused(window);
        let brief_focused = self.lib_brief.focus_handle(cx).is_focused(window);
        let typed = self.lib_name.read(cx).value().trim().to_string();
        let bad_name = !typed.is_empty() && !valid_tool_name(&typed);
        let labeled = |l: &str, el: AnyElement| div().flex().flex_col().gap(px(5.)).child(div().text_size(px(12.)).text_color(t.dim).child(l.to_string())).child(el);
        let name_box = div()
            .w(px(240.))
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(ui::R_CTRL))
            .border_1()
            .border_color(if bad_name { t.bad } else if name_focused { t.accent } else { t.line2 })
            .bg(t.bg)
            .font_family(NUM)
            .text_size(px(13.))
            .text_color(t.ink)
            .child(gpui_base::Input::new(&self.lib_name));
        let brief_box = div()
            .px(px(10.))
            .py(px(8.))
            .rounded(px(ui::R_CTRL))
            .border_1()
            .border_color(if brief_focused { t.accent } else { t.line2 })
            .bg(t.bg)
            .text_size(px(13.))
            .line_height(relative(1.5))
            .text_color(t.ink)
            .child(gpui_base::Textarea::new(&self.lib_brief));
        let sending = n.sending;
        Self::panel(
            t,
            "lib-new-x",
            "New tool",
            "A Claude chat builds it in its own folder, tests it, and publishes it to the library. You can follow it on the board like any chat.",
            cx,
            |this, _| this.lib_new = None,
        )
        .child(
            labeled(
                "Name",
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(10.))
                    .child(name_box)
                    .child(div().text_size(px(12.)).text_color(if bad_name { t.bad } else { t.faint }).child("Lowercase letters, digits and dashes."))
                    .into_any_element(),
            ),
        )
        .child(labeled("What it should do", brief_box.into_any_element()))
        .child(labeled("Build it on", Self::machine_seg(t, "lib-new-m", Some(n.machine.as_str()), app, cx, |this, m, cx| {
            if let Some(n) = this.lib_new.as_mut() {
                n.machine = m;
            }
            cx.notify();
        })))
        .when_some(n.error.clone(), |d, e| d.child(ui::wrap(e).text_size(px(13.)).text_color(t.bad)))
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(ui::ghost(t, "lib-new-cancel", Some("Cancel"), None).h(px(32.)).px(px(10.)).text_size(px(13.)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.lib_new = None;
                    cx.notify();
                })))
                .child(
                    ui::primary_btn(t, "lib-new-go", 32.)
                        .px(px(14.))
                        .text_size(px(13.))
                        .when(sending, |d| d.opacity(0.6))
                        .child(ui::icon("spark", 14., t.on_accent))
                        .child(if sending { "Starting" } else { "Start building" })
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.new_tool_go(cx))),
                ),
        )
        .into_any_element()
    }
}
