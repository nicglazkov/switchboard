//! The flags drawer (the web app's, 1491dec): what a chat runs with ("Active now"), what can change ("Can add": model,
//! effort, permissions, Claude in Chrome, extra folders, other flags) and the changes waiting ("Pending").
//! Model, effort, plan mode and folders change live: they are typed into the chat (/model, /effort, /add-dir, /plan).
//! Anything else relaunches the chat with `--resume` through POST /api/relaunch, after a notice, and the subagent
//! safety check ("busy": wait for its work, or go now) the rest of the app uses. Opened from a card's flag button, the
//! inspector, the right-click menu and the terminal's bar.

use std::time::Duration;

use gpui::{
    AnimationExt, AnyElement, AppContext, ClickEvent, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::*, px,
    relative,
};
use gpui_base::input::{InputEvent, InputState};
use sb_core::api::{ApiFlagsPlan, ApiFlagsView};

use crate::theme::{NUM, Theme};
use crate::ui;
use crate::view::{BoardView, Ctx};

pub const EFFORTS: [(&str, &str); 6] = [
    ("low", "Low"),
    ("medium", "Medium"),
    ("high", "High"),
    ("xhigh", "Extra high"),
    ("max", "Max"),
    ("ultracode", "UltraCode"),
];
pub const PERMS: [(&str, &str); 6] = [
    ("default", "Asks first"),
    ("acceptEdits", "Accept edits"),
    ("plan", "Plan"),
    ("auto", "Auto"),
    ("bypassPermissions", "Bypass"),
    ("dontAsk", "Never asks"),
];
/// Never removable from the drawer: the model, effort and permissions always have a value; Remote Control is Switchboard's.
const FIXED: [&str; 4] = ["--model", "--effort", "--permission-mode", "--remote-control"];
const DIRS_MAX: usize = 8;

/// What the user wants the chat to run with, as POST /api/flags takes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Want {
    pub model: Option<String>,
    pub effort: Option<String>,
    pub perm: String,
    pub chrome: bool,
    pub dirs: Vec<String>,
    pub extra: Vec<Vec<String>>,
    pub other: String,
}

impl Want {
    pub fn of(v: &ApiFlagsView) -> Want {
        let mut w = Want { perm: "default".into(), ..Default::default() };
        for x in &v.active {
            match x.name.as_str() {
                "--model" => w.model = x.value.clone(),
                "--effort" => w.effort = x.value.clone(),
                "--permission-mode" => {
                    w.perm = if x.value.as_deref() == Some("manual") {
                        "default".into()
                    } else {
                        x.value.clone().unwrap_or_else(|| "default".into())
                    }
                }
                "--chrome" => w.chrome = true,
                "--add-dir" => w.dirs.extend(x.value.clone()),
                "--remote-control" => {}
                n => w.extra.push(std::iter::once(n.to_string()).chain(x.value.clone()).collect()),
            }
        }
        w
    }

    pub fn body(&self) -> serde_json::Value {
        let mut flags: Vec<serde_json::Value> = self.dirs.iter().map(|d| serde_json::json!(["--add-dir", d])).collect();
        flags.extend(self.extra.iter().map(|f| serde_json::json!(f)));
        serde_json::json!({"model": self.model, "effort": self.effort, "perm": self.perm, "chrome": self.chrome, "other": self.other.trim(), "flags": flags})
    }

    /// Is this "Active now" row still in what the user wants?
    pub fn keeps(&self, name: &str, value: Option<&str>) -> bool {
        match name {
            "--chrome" => self.chrome,
            "--add-dir" => value.is_some_and(|v| self.dirs.iter().any(|d| d == v)),
            n if FIXED.contains(&n) => true,
            n => self.extra.iter().any(|f| f[0] == n && f.get(1).map(String::as_str) == value),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    Load,
    Edit,
    /// "Relaunch with these flags?"
    Confirm,
    /// Sent; the job runs in the corner and any terminal on it reconnects.
    Sent,
    /// Open in a terminal Switchboard does not own: the command to run there.
    Cmd {
        cmd: String,
        err: String,
    },
    /// Typed into the chat.
    Applied(Vec<String>),
    /// Waiting for the reply to end before typing.
    WaitLive,
    Error(String),
}

pub struct Drawer {
    pub key: String,
    pub machine: String,
    pub sid: String,
    pub title: String,
    pub label: String,
    pub view: Option<ApiFlagsView>,
    pub want: Want,
    pub plan: Option<ApiFlagsPlan>,
    pub plan_err: Option<String>,
    pub stage: Stage,
    pub seq: u64,
    pub adding: bool,
    /// Other flags, and a folder being added. Made on the first frame (they need the window).
    pub other: Option<Entity<InputState>>,
    pub dir: Option<Entity<InputState>>,
}

/// The drawer still waits to type into chat `key`: it was not closed or switched to another chat meanwhile.
fn still_waits(d: Option<&Drawer>, key: &str) -> bool {
    d.is_some_and(|d| d.key == key && d.stage == Stage::WaitLive)
}

impl BoardView {
    /// Opens the drawer for a chat and reads what it runs with.
    pub fn open_flags(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(s) = self.app.read(cx).snap.session(&key) else { return };
        self.menu = None;
        self.flags = Some(Drawer {
            key: key.clone(),
            machine: s.machine.clone(),
            sid: s.sid.clone(),
            title: s.topic.clone(),
            label: s.machine_label.clone(),
            view: None,
            want: Want::default(),
            plan: None,
            plan_err: None,
            stage: Stage::Load,
            seq: 0,
            adding: false,
            other: None,
            dir: None,
        });
        self.flags_load(false, cx);
        cx.notify();
    }

    fn flags_load(&mut self, quiet: bool, cx: &mut Context<Self>) {
        let Some(d) = &self.flags else { return };
        let (m, sid, key) = (d.machine.clone(), d.sid.clone(), d.key.clone());
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.flags_view(&m, &sid) }).await;
            let _ = this.update(cx, |v, cx| {
                let Some(d) = v.flags.as_mut().filter(|d| d.key == key) else { return };
                match res {
                    Ok(view) => {
                        let keep =
                            quiet && d.stage == Stage::Edit && d.plan.as_ref().is_some_and(|p| !p.changes.is_empty());
                        if !keep {
                            d.want = Want::of(&view);
                            d.plan = None;
                            d.stage = Stage::Edit;
                        }
                        d.view = Some(view);
                    }
                    Err(e) if !quiet => d.stage = Stage::Error(e.to_string()),
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks the server what the wanted set changes (300 ms after typing in Other flags).
    fn flags_replan(&mut self, soon: bool, cx: &mut Context<Self>) {
        let Some(d) = self.flags.as_mut() else { return };
        d.seq += 1;
        let seq = d.seq;
        let (m, sid, want) = (d.machine.clone(), d.sid.clone(), d.want.body());
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            if soon {
                cx.background_executor().timer(Duration::from_millis(300)).await;
                if this.update(cx, |v, _| v.flags.as_ref().map(|d| d.seq) != Some(seq)).unwrap_or(true) {
                    return;
                }
            }
            let res = cx.background_executor().spawn(async move { core.flags_plan(&m, &sid, &want) }).await;
            let _ = this.update(cx, |v, cx| {
                let Some(d) = v.flags.as_mut().filter(|d| d.seq == seq) else { return };
                match res {
                    Ok(p) => {
                        d.plan = Some(p);
                        d.plan_err = None;
                    }
                    Err(e) => {
                        d.plan = None;
                        d.plan_err = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn flags_pick(&mut self, f: impl FnOnce(&mut Want), cx: &mut Context<Self>) {
        if let Some(d) = self.flags.as_mut() {
            f(&mut d.want);
            d.stage = Stage::Edit;
        }
        self.flags_replan(false, cx);
        cx.notify();
    }

    pub fn flags_relaunching(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(d) = self.flags.as_mut().filter(|d| d.key == key) {
            d.stage = Stage::Sent;
            cx.notify();
        }
    }

    pub fn flags_need_terminal(&mut self, key: &str, cmd: String, err: String, cx: &mut Context<Self>) {
        if let Some(d) = self.flags.as_mut().filter(|d| d.key == key) {
            d.stage = Stage::Cmd { cmd, err };
            cx.notify();
        }
    }

    /// Live changes: typed into the chat's open terminal tab (opened first if needed). Mid-reply, they wait.
    fn flags_apply(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.flags.as_ref() else { return };
        let Some(cmds) = d.plan.as_ref().map(|p| p.cmds.clone()).filter(|c| !c.is_empty()) else { return };
        let key = d.key.clone();
        let busy = d.view.as_ref().and_then(|v| v.status.as_deref()) == Some("busy");
        let (m, sid) = (d.machine.clone(), d.sid.clone());
        if busy {
            if let Some(d) = self.flags.as_mut() {
                d.stage = Stage::WaitLive;
            }
            let core = self.app.read(cx).core.clone();
            cx.spawn(async move |this, cx| {
                for _ in 0..300 {
                    cx.background_executor().timer(Duration::from_secs(3)).await;
                    // Closed, or showing another chat now: these commands are not for it.
                    if !this.update(cx, |v, _| still_waits(v.flags.as_ref(), &key)).unwrap_or(false) {
                        return;
                    }
                    let (c, m, s) = (core.clone(), m.clone(), sid.clone());
                    let st = cx.background_executor().spawn(async move { c.flags_view(&m, &s) }).await;
                    if st.is_ok_and(|v| v.status.as_deref() != Some("busy")) {
                        break;
                    }
                }
                let _ = this.update(cx, |v, cx| v.flags_type(key, cmds, cx));
            })
            .detach();
            cx.notify();
            return;
        }
        self.flags_type(key, cmds, cx);
    }

    /// Types them into chat `key`'s tab, the chat they were planned for, whichever drawer shows by then.
    fn flags_type(&mut self, key: String, cmds: Vec<String>, cx: &mut Context<Self>) {
        let tab = self.tab_for(&key, cx).and_then(|i| self.tabs.get(i)).cloned();
        match tab {
            Some(tab) if tab.read(cx).live() => {
                tab.update(cx, |t, cx| t.type_cmds(cmds.clone(), cx));
                if let Some(d) = self.flags.as_mut().filter(|d| d.key == key) {
                    d.stage = Stage::Applied(cmds);
                }
                // The agent reads the new state on its next look.
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_secs(6)).await;
                    let _ = this.update(cx, |v, cx| v.flags_load(false, cx));
                })
                .detach();
            }
            _ => {
                // Open the terminal; the commands go in once it is attached.
                self.open_terminal(key.clone(), false, cx);
                if let Some(tab) = self.tab_for(&key, cx).and_then(|i| self.tabs.get(i)).cloned() {
                    tab.update(cx, |t, cx| t.type_when_open(cmds.clone(), cx));
                }
                if let Some(d) = self.flags.as_mut().filter(|d| d.key == key) {
                    d.stage = Stage::Applied(cmds);
                }
            }
        }
        cx.notify();
    }

    /// The drawer, when open: over the board or the terminal, on the right, below the header.
    pub fn flags_layer(&mut self, c: &Ctx, _w: f32, _h: f32) -> Option<AnyElement> {
        let d = self.flags.as_ref()?;
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    c.cx.listener(|this, _, _, cx| {
                        this.flags = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .id("flags-drawer")
                        .absolute()
                        .top(px(ui::header() + 8.))
                        .right(px(12.))
                        .bottom(px(ui::FOOTER + 8.))
                        .w(px(460.))
                        .rounded(px(ui::R_SHEET))
                        .border_1()
                        .border_color(c.t.line2)
                        .bg(c.t.panel)
                        .shadow(c.t.drop())
                        .overflow_hidden()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .flex()
                        .flex_col()
                        .child(self.flags_body(c, d))
                        .with_animation(
                            "flags-in",
                            gpui::Animation::new(ui::FADE).with_easing(gpui::ease_out_quint()),
                            |e, k| e.opacity(k).mr(px(-24. * (1. - k))),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Makes the drawer's text fields on the first frame it shows.
    pub fn flags_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.flags.as_mut() else { return };
        if d.other.is_some() {
            return;
        }
        let other = cx.new(|cx| InputState::new(window, cx).placeholder("--verbose"));
        let dir = cx.new(|cx| InputState::new(window, cx).placeholder("Folder in the Claude folder"));
        let s1 = cx.subscribe_in(&other, window, |this, inp, ev: &InputEvent, _, cx| {
            if let InputEvent::Change = ev {
                let v = inp.read(cx).value().to_string();
                if let Some(d) = this.flags.as_mut() {
                    d.want.other = v;
                }
                this.flags_replan(true, cx);
            }
        });
        let s2 = cx.subscribe_in(&dir, window, |this, inp, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = ev {
                let v = inp.read(cx).value().to_string();
                inp.update(cx, |i, cx| i.set_value("", window, cx));
                this.flags_add_dir(v, cx);
            }
        });
        self._subs.push(s1);
        self._subs.push(s2);
        d.other = Some(other);
        d.dir = Some(dir);
    }

    fn flags_add_dir(&mut self, v: String, cx: &mut Context<Self>) {
        // A folder under the Claude folder, with "/" between its parts even when typed the Windows way ("app\src").
        let dir = v.trim().replace('\\', "/");
        let dir = dir.trim_start_matches("./").trim_start_matches('/').trim_end_matches('/').to_string();
        if let Some(d) = self.flags.as_mut() {
            d.adding = false;
        }
        self.flags_pick(
            move |w| {
                if !dir.is_empty() && !w.dirs.contains(&dir) && w.dirs.len() < DIRS_MAX {
                    w.dirs.push(dir);
                }
            },
            cx,
        );
    }

    fn flags_body(&self, c: &Ctx, d: &Drawer) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let head = div()
            .flex_none()
            .px(px(20.))
            .pt(px(16.))
            .pb(px(12.))
            .flex()
            .items_start()
            .gap(px(12.))
            .border_b_1()
            .border_color(t.line)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().text_size(px(17.)).font_weight(FontWeight::BOLD).text_color(t.ink).child("Flags"))
                    .child(ui::wrap(format!("{} on {}", d.title, d.label)).text_size(px(13.)).text_color(t.dim))
                    .children(
                        d.view
                            .as_ref()
                            .filter(|v| !v.summary.is_empty())
                            .map(|v| ui::wrap(v.summary.clone()).text_size(px(12.)).text_color(t.faint)),
                    ),
            )
            .child(ui::icon_btn(&t, "flags-x", "x", 28.).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.flags = None;
                cx.notify();
            })));
        let Some(v) = d.view.as_ref() else {
            let body = match &d.stage {
                Stage::Error(e) => div()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(ui::wrap(e.clone()).text_size(px(13.)).text_color(t.bad))
                    .child(ui::line_btn(&t, "fl-retry", 30.).child("Try again").on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(d) = this.flags.as_mut() {
                                d.stage = Stage::Load;
                            }
                            this.flags_load(false, cx);
                        },
                    ))),
                _ => ui::loading(&t, "fl-load", "Reading what it runs with").p(px(20.)),
            };
            return div().size_full().flex().flex_col().child(head).child(body).into_any_element();
        };
        let w = &d.want;
        let section = |title: &str, n: Option<usize>| {
            div()
                .mt(px(14.))
                .mb(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.dim)
                .child(title.to_string())
                .children(n.map(|n| div().text_color(t.faint).child(n.to_string())))
        };
        let tag = |how: &str| {
            let (text, col) = match how {
                "live" => ("Live", t.warm),
                "key" => ("Shift+Tab", t.warm),
                "locked" => ("Kept on", t.faint),
                _ => ("Relaunch", t.cold),
            };
            div()
                .flex_none()
                .px(px(7.))
                .h(px(20.))
                .flex()
                .items_center()
                .rounded(px(ui::R_CHIP))
                .bg(col.alpha(0.12))
                .text_size(px(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(col)
                .child(text)
        };
        // Active now: what, its value, how it changes, and remove (or keep it, once removed).
        let mut active = div().flex().flex_col();
        for (i, x) in v.active.iter().enumerate() {
            let kept = w.keeps(&x.name, x.value.as_deref());
            let fixed = FIXED.contains(&x.name.as_str());
            let (name, val) = (x.name.clone(), x.value.clone());
            active = active.child(
                div()
                    .min_h(px(36.))
                    .py(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .border_b_1()
                    .border_color(t.line)
                    .when(!kept, |d| d.opacity(0.5))
                    .child(div().w(px(120.)).flex_none().text_size(px(13.)).text_color(t.ink2).child(x.label.clone()))
                    .child(
                        ui::wrap(x.text.clone())
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink),
                    )
                    .child(tag(&x.how))
                    .child(if fixed {
                        div().w(px(26.)).into_any_element()
                    } else {
                        ui::icon_btn(&t, SharedString::from(format!("fl-rm-{i}")), if kept { "x" } else { "undo" }, 26.)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let (name, val) = (name.clone(), val.clone());
                                this.flags_pick(
                                    move |w| {
                                        let kept = w.keeps(&name, val.as_deref());
                                        match name.as_str() {
                                            "--chrome" => w.chrome = !kept,
                                            "--add-dir" => {
                                                let v = val.clone().unwrap_or_default();
                                                if kept { w.dirs.retain(|d| *d != v) } else { w.dirs.push(v) }
                                            }
                                            _ => {
                                                if kept {
                                                    w.extra.retain(|f| !(f[0] == name && f.get(1) == val.as_ref()))
                                                } else {
                                                    w.extra.push(
                                                        std::iter::once(name.clone()).chain(val.clone()).collect(),
                                                    )
                                                }
                                            }
                                        }
                                    },
                                    cx,
                                )
                            }))
                            .into_any_element()
                    }),
            );
        }
        // The models the machines take (GET /api/models, or the built-in list): segmented buttons. Six efforts and
        // five permission modes: dropdowns.
        let mut models = ui::seg_box(&t);
        for (k, l) in self.app.read(cx).model_choices().into_iter().skip(1) {
            let on = w.model.as_deref() == Some(k.as_str());
            models =
                models.child(ui::seg_btn(&t, SharedString::from(format!("fl-m-{k}")), on, false).child(l).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let k = k.clone();
                        this.flags_pick(move |w| w.model = Some(k), cx)
                    }),
                ));
        }
        let orow = |label: &str, ctl: AnyElement| {
            div()
                .flex()
                .items_start()
                .gap(px(10.))
                .py(px(4.))
                .child(
                    div()
                        .w(px(96.))
                        .flex_none()
                        .pt(px(5.))
                        .text_size(px(12.5))
                        .text_color(t.dim)
                        .child(label.to_string()),
                )
                .child(div().flex_1().min_w_0().child(ctl))
        };
        let mut dirs = div().flex().flex_wrap().items_center().gap(px(5.));
        for dd in &w.dirs {
            let d2 = dd.clone();
            dirs = dirs.child(
                chip(&t, SharedString::from(format!("fl-dir-{dd}")), true, dd)
                    .child(ui::icon("x", 11., t.dim))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let d2 = d2.clone();
                        this.flags_pick(move |w| w.dirs.retain(|x| *x != d2), cx)
                    })),
            );
        }
        if d.adding {
            if let Some(inp) = &d.dir {
                dirs = dirs.child(field(&t, inp, 200.));
            }
        } else if w.dirs.len() < DIRS_MAX {
            dirs =
                dirs.child(chip(&t, "fl-adddir", false, "Add a folder").child(ui::icon("plus", 11., t.dim)).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        if let Some(d) = this.flags.as_mut() {
                            d.adding = true;
                            if let Some(i) = &d.dir {
                                i.update(cx, |i, cx| i.focus(window, cx));
                            }
                        }
                        cx.notify();
                    }),
                ));
        }
        // Two groups, so what changes live and what relaunches the chat can never be mistaken for each other.
        let running = v.status.as_deref() != Some("stopped");
        let group = |live: bool, title: &str, sub: &str, rows: gpui::Div| {
            let col = if live { t.warm } else { t.cold };
            div()
                .mt(px(10.))
                .p(px(ui::PAD_CARD))
                .rounded(px(ui::R_CARD))
                .bg(col.alpha(if t.dark { 0.09 } else { 0.06 }))
                .shadow(vec![Theme::ring(col.alpha(0.28))])
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(ui::icon(if live { "bolt" } else { "relaunch" }, 15., col))
                        .child(
                            ui::wrap(title.to_string())
                                .flex_1()
                                .min_w_0()
                                .text_size(px(13.5))
                                .font_weight(FontWeight::BOLD)
                                .text_color(t.ink),
                        )
                        .child(tag(if live { "live" } else { "relaunch" })),
                )
                .child(
                    ui::wrap(sub.to_string())
                        .mb(px(6.))
                        .text_size(px(12.))
                        .line_height(relative(1.45))
                        .text_color(t.dim),
                )
                .child(rows)
        };
        let orig_perm = Want::of(v).perm;
        let plan_on = w.perm == "plan";
        let plan_chip =
            chip(&t, "fl-plan", plan_on, "Plan mode").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let back = if orig_perm == "plan" { "default".to_string() } else { orig_perm.clone() };
                this.flags_pick(move |w| w.perm = if w.perm == "plan" { back } else { "plan".into() }, cx)
            }));
        let perms: Vec<(&'static str, &'static str)> = PERMS.iter().copied().filter(|(k, _)| *k != "plan").collect();
        let efforts = ui::choice(
            &t,
            cx,
            "fl-effort",
            self.drop.is_open("fl-effort"),
            &EFFORTS,
            w.effort.as_deref().unwrap_or(""),
            false,
            None,
            |v| &mut v.drop,
            |v, k, cx| v.flags_pick(move |w| w.effort = Some(k.into()), cx),
        );
        let perm_drop = ui::choice(
            &t,
            cx,
            "fl-perm",
            self.drop.is_open("fl-perm"),
            &perms,
            &w.perm,
            false,
            Some("bypassPermissions"),
            |v| &mut v.drop,
            |v, k, cx| v.flags_pick(move |w| w.perm = k.into(), cx),
        );
        let live_rows = div()
            .flex()
            .flex_col()
            .child(orow("Model", div().flex().child(models).into_any_element()))
            .child(orow("Effort", div().flex().child(efforts).into_any_element()))
            .child(orow("Plan", div().flex().child(plan_chip).into_any_element()))
            .child(orow("Also open", dirs.into_any_element()));
        let relaunch_rows = div()
            .flex()
            .flex_col()
            .child(orow("Permissions", div().flex().child(perm_drop).into_any_element()))
            .child(orow(
                "Browser",
                div()
                    .flex()
                    .child(chip(&t, "fl-chrome", w.chrome, "Claude in Chrome").on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.flags_pick(|w| w.chrome = !w.chrome, cx)),
                    ))
                    .into_any_element(),
            ))
            .children(d.other.as_ref().map(|inp| orow("Other flags", field(&t, inp, 0.).into_any_element())));
        let can = div()
            .flex()
            .flex_col()
            .child(group(
                true,
                "Changes live",
                if running {
                    "Typed straight into the chat (/model, /effort, /add-dir, plan mode with Shift+Tab). No restart."
                } else {
                    "Typed straight into a running chat. This one is stopped, so they go in when it resumes."
                },
                live_rows,
            ))
            .child(group(
                false,
                "Needs a relaunch",
                "Claude Code reads these when it starts, so the chat relaunches with --resume and carries on with its history.",
                relaunch_rows,
            ))
            .children(d.plan_err.clone().map(|e| ui::wrap(e).mt(px(6.)).text_size(px(12.)).text_color(t.bad)));
        let changes = d.plan.as_ref().map(|p| p.changes.clone()).unwrap_or_default();
        let mut pend = div().flex().flex_col();
        if changes.is_empty() {
            pend = pend.child(div().text_size(px(13.)).text_color(t.faint).child("Nothing yet. Pick a change above."));
        }
        for ch in &changes {
            let what = match (&ch.from, &ch.to) {
                (None, Some(to)) if to == "On" => "Turns on".to_string(),
                (None, Some(to)) => format!("Adds {to}"),
                (Some(f), None) if f == "On" => "Turns off".to_string(),
                (Some(f), None) => format!("Drops {f}"),
                (Some(f), Some(to)) => format!("{f} to {to}"),
                (None, None) => String::new(),
            };
            pend = pend.child(
                div()
                    .min_h(px(34.))
                    .py(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .border_b_1()
                    .border_color(t.line)
                    .child(div().w(px(120.)).flex_none().text_size(px(13.)).text_color(t.ink2).child(ch.label.clone()))
                    .child(
                        ui::wrap(what)
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink),
                    )
                    .child(tag(&ch.how)),
            );
        }
        if let Some(p) = d.plan.as_ref().filter(|p| p.relaunch && !p.dropped.is_empty()) {
            pend = pend.child(
                ui::wrap(format!("The relaunch leaves out {}.", p.dropped.join(" ")))
                    .mt(px(6.))
                    .text_size(px(12.))
                    .text_color(t.dim),
            );
        }
        let body = div()
            .id("flags-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.flags_scroll)
            .px(px(20.))
            .pb(px(16.))
            .child(section("Active now", Some(v.active.len())))
            .child(active)
            .when(!v.other.is_empty(), |e| {
                e.child(
                    ui::wrap(format!(
                        "Also on its command line: {}. A relaunch leaves {} out.",
                        v.other.join(" "),
                        if v.other.len() == 1 { "it" } else { "them" }
                    ))
                    .mt(px(6.))
                    .text_size(px(12.))
                    .text_color(t.dim),
                )
            })
            .when(!v.complete, |e| {
                e.child(
                    ui::wrap("Switchboard on that machine doesn't report every flag yet. Update it to see them all.")
                        .mt(px(6.))
                        .text_size(px(12.))
                        .text_color(t.dim),
                )
            })
            .child(section("What you can change", None))
            .child(can)
            .child(section("Pending", Some(changes.len())))
            .child(pend);
        let body =
            div().flex_1().min_h_0().relative().flex().flex_col().child(body).child(ui::scrollbar(&self.flags_scroll));
        div().size_full().flex().flex_col().child(head).child(body).child(self.flags_foot(c, d, v)).into_any_element()
    }

    fn flags_foot(&self, c: &Ctx, d: &Drawer, v: &ApiFlagsView) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let plan = d.plan.as_ref();
        let n = plan.map(|p| p.changes.len()).unwrap_or(0);
        // Its own shade, with the drawer's bottom radius (less the 1 px border) so the corners stay round.
        let foot = div()
            .flex_none()
            .px(px(20.))
            .py(px(14.))
            .border_t_1()
            .border_color(t.line)
            .bg(t.panel2)
            .rounded_b(px(ui::R_SHEET - 1.))
            .flex()
            .flex_col()
            .gap(px(10.));
        let note = |text: String, col| ui::wrap(text).text_size(px(13.)).line_height(relative(1.45)).text_color(col);
        let reset = ui::ghost(&t, "fl-reset", Some("Undo the changes"), None)
            .h(px(32.))
            .px(px(10.))
            .text_size(px(13.))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Some(d) = this.flags.as_mut() {
                    if let Some(v) = &d.view {
                        d.want = Want::of(v);
                    }
                    d.plan = None;
                    d.plan_err = None;
                    d.stage = Stage::Edit;
                }
                cx.notify();
            }));
        let primary =
            |id: &'static str, label: String| ui::primary_btn(&t, id, 34.).px(px(14.)).text_size(px(13.)).child(label);
        let back = ui::line_btn(&t, "fl-back", 34.).px(px(14.)).child("Back").on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                if let Some(d) = this.flags.as_mut() {
                    d.stage = Stage::Edit;
                }
                cx.notify();
            },
        ));
        let el = match &d.stage {
            Stage::Confirm => {
                let busy = plan.is_some_and(|p| p.busy);
                let body = plan.map(|p| p.body.clone()).unwrap_or_default();
                let key = d.key.clone();
                foot.child(note("Claude restarts in the same folder with the new flags and picks up where it left off. An open terminal reconnects by itself.".into(), t.ink2))
                    .when(busy, |f| f.child(note("Claude is replying right now. You can wait for it to finish, or relaunch now.".into(), t.cold)))
                    .child(
                        div().flex().justify_end().gap(px(8.)).child(back).child(primary("fl-go", "Relaunch".into()).child(ui::icon("resume", 14., t.on_accent)).on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                let (k, b) = (key.clone(), body.clone());
                                this.app.update(cx, |s, cx| s.relaunch(&k, b, None, cx));
                            },
                        ))),
                    )
            }
            Stage::Sent => foot
                .child(note(
                    "Relaunching. You can follow it in the corner, and an open terminal reconnects when it's done."
                        .into(),
                    t.ink2,
                ))
                .child(div().flex().justify_end().child(
                    ui::line_btn(&t, "fl-done", 34.).px(px(14.)).child("Done").on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.flags = None;
                            cx.notify();
                        },
                    )),
                )),
            Stage::Cmd { cmd, err } => {
                let cmd2 = cmd.clone();
                foot.child(note(err.clone(), t.ink2))
                    .child(
                        div()
                            .p(px(10.))
                            .rounded(px(ui::R_CTRL))
                            .bg(t.panel3)
                            .font_family(NUM)
                            .text_size(px(12.))
                            .text_color(t.ink)
                            .child(ui::wrap(cmd.clone())),
                    )
                    .child(div().flex().justify_end().gap(px(8.)).child(back).child(
                        primary("fl-copy", "Copy the command".into()).on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                crate::state::copy_text(cx, cmd2.clone());
                                this.app.update(cx, |s, cx| {
                                    s.toast(
                                        "Copied. Paste it in that terminal once Claude has exited.",
                                        None,
                                        false,
                                        cx,
                                    )
                                });
                            },
                        )),
                    ))
            }
            Stage::Applied(cmds) => foot.child(note(
                format!("Typed into the chat: {}. It shows here within a few seconds.", cmds.join(", ")),
                t.warm,
            )),
            Stage::WaitLive => foot
                .child(note("Claude is replying. The changes go in once it finishes.".into(), t.ink2))
                .child(div().flex().justify_end().child(back)),
            Stage::Error(e) => foot.child(note(e.clone(), t.bad)).child(div().flex().justify_end().child(back)),
            Stage::Load | Stage::Edit if n == 0 => foot.child(
                div()
                    .flex()
                    .justify_end()
                    .child(ui::line_btn(&t, "fl-none", 34.).px(px(14.)).opacity(0.6).child("Nothing to apply yet")),
            ),
            Stage::Load | Stage::Edit => {
                let p = plan.cloned().unwrap_or_default();
                let mine =
                    v.kind.as_deref() == Some("interactive") && v.status.as_deref().is_some_and(|s| s != "stopped");
                // The summary bar: how many of each kind, so what the button will do is never a surprise.
                let lives = p.changes.iter().filter(|c| c.how == "live" || c.how == "key").count();
                let rels = p.changes.len() - lives;
                let mut parts = Vec::new();
                if lives > 0 {
                    parts.push(format!("{} live {}", lives, if lives == 1 { "change" } else { "changes" }));
                }
                if rels > 0 {
                    parts.push(format!("{} {} a relaunch", rels, if rels == 1 { "needs" } else { "need" }));
                }
                let count = |n: usize, col, ic: &'static str| {
                    div().flex_none().flex().items_center().gap(px(5.)).child(ui::icon(ic, 13., col)).child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(col)
                            .child(n.to_string()),
                    )
                };
                let summary = div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        ui::wrap(parts.join(", "))
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink),
                    )
                    .when(lives > 0, |d| d.child(count(lives, t.warm, "bolt")))
                    .when(rels > 0, |d| d.child(count(rels, t.cold, "relaunch")));
                let act = if mine {
                    primary("fl-relaunch", "Get the command to relaunch it".into()).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            if let Some(d) = this.flags.as_ref() {
                                let (k, b) =
                                    (d.key.clone(), d.plan.as_ref().map(|p| p.body.clone()).unwrap_or_default());
                                this.app.update(cx, |s, cx| s.relaunch(&k, b, None, cx));
                            }
                        },
                    ))
                } else if !p.relaunch && p.running {
                    primary("fl-apply", "Apply live".into())
                        .child(ui::icon("bolt", 14., t.on_accent))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.flags_apply(cx)))
                } else {
                    primary(
                        "fl-relaunch",
                        if p.running {
                            "Apply and relaunch".to_string()
                        } else {
                            format!("Resume with {}", plural(n, "change"))
                        },
                    )
                    .child(ui::icon("relaunch", 14., t.on_accent))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if let Some(d) = this.flags.as_mut() {
                            d.stage = Stage::Confirm;
                        }
                        cx.notify();
                    }))
                };
                let how = if mine {
                    format!("It's open in a terminal on {}, so you'll get the command to run there.", d.label)
                } else if !p.relaunch && p.running {
                    format!("Types {} into the chat. No restart.", p.cmds.join(", "))
                } else if p.running && lives > 0 {
                    "Relaunches the same chat with --resume, so it keeps its history. The live changes go in with the relaunch.".to_string()
                } else if p.running {
                    "Relaunches the same chat with --resume, so it keeps its history.".to_string()
                } else {
                    "Resumes the chat with --resume and these flags. It keeps its history.".to_string()
                };
                foot.child(summary)
                    .child(note(how, t.dim))
                    .child(div().flex().items_center().justify_end().gap(px(8.)).child(reset).child(act))
            }
        };
        el.into_any_element()
    }
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

/// A choice chip: on (filled) or off (a ring).
fn chip(t: &Theme, id: impl Into<gpui::ElementId>, on: bool, label: &str) -> gpui::Stateful<gpui::Div> {
    let (hover, press) = (t.hover, t.press);
    ui::focusable(div().id(id.into()), t)
        .flex_none()
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(5.))
        .rounded(px(ui::R_CTRL))
        .text_size(px(12.5))
        .cursor_pointer()
        .active(move |s| s.bg(press))
        .when(on, |d| {
            d.bg(t.accent.alpha(if t.dark { 0.2 } else { 0.12 }))
                .shadow(vec![Theme::ring(t.accent)])
                .text_color(t.ink)
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!on, |d| d.shadow(vec![Theme::ring(t.line2)]).text_color(t.ink2).hover(move |s| s.bg(hover)))
        .child(label.to_string())
}

/// A one-line text field in the drawer.
fn field(t: &Theme, inp: &Entity<InputState>, w: f32) -> gpui::Div {
    div()
        .when(w > 0., |d| d.w(px(w)))
        .when(w == 0., |d| d.w_full())
        .h(px(30.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(px(ui::R_CTRL))
        .border_1()
        .border_color(t.line2)
        .bg(t.bg)
        .font_family(NUM)
        .text_size(px(12.5))
        .text_color(t.ink)
        .child(gpui_base::Input::new(inp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_core::api::ApiFlagItem;

    fn item(name: &str, value: Option<&str>) -> ApiFlagItem {
        ApiFlagItem {
            name: name.into(),
            value: value.map(str::to_string),
            label: name.into(),
            text: value.unwrap_or("On").into(),
            how: "relaunch".into(),
        }
    }

    fn drawer(key: &str, stage: Stage) -> Drawer {
        Drawer {
            key: key.into(),
            machine: "studio".into(),
            sid: "s1".into(),
            title: String::new(),
            label: String::new(),
            view: None,
            want: Want::default(),
            plan: None,
            plan_err: None,
            stage,
            seq: 0,
            adding: false,
            other: None,
            dir: None,
        }
    }

    #[test]
    fn commands_wait_only_while_the_drawer_shows_their_chat() {
        assert!(still_waits(Some(&drawer("studio:a", Stage::WaitLive)), "studio:a"));
        assert!(!still_waits(Some(&drawer("studio:b", Stage::WaitLive)), "studio:a"), "switched to another chat");
        assert!(!still_waits(Some(&drawer("studio:a", Stage::Edit)), "studio:a"));
        assert!(!still_waits(None, "studio:a"), "closed");
    }

    #[test]
    fn want_reads_the_active_list_and_round_trips() {
        let v = ApiFlagsView {
            active: vec![
                item("--model", Some("opus")),
                item("--effort", Some("high")),
                item("--permission-mode", Some("manual")),
                item("--chrome", None),
                item("--add-dir", Some("notes")),
                item("--verbose", None),
                item("--remote-control", None),
            ],
            ..Default::default()
        };
        let w = Want::of(&v);
        assert_eq!(
            (w.model.as_deref(), w.effort.as_deref(), w.perm.as_str(), w.chrome),
            (Some("opus"), Some("high"), "default", true)
        );
        assert_eq!(w.dirs, vec!["notes".to_string()]);
        assert_eq!(w.extra, vec![vec!["--verbose".to_string()]]);
        assert!(w.keeps("--remote-control", None), "Switchboard's own flag never drops");
        let b = w.body();
        assert_eq!(b["flags"], serde_json::json!([["--add-dir", "notes"], ["--verbose"]]));
        let mut w2 = w.clone();
        w2.dirs.clear();
        assert!(!w2.keeps("--add-dir", Some("notes")));
    }
}
