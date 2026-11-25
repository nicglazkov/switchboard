//! Settings > Devices: the computers and phones paired with the home computer, each with its own key
//! (docs/plans/2026-11-22-keys-phase-1-2.md, task 2.10). Add a device shows a code, the same thing as a link and as
//! a QR code, and counts down until it runs out; the card turns to "paired" the moment a device joins with it.
//! Remove says what it does before it does it. Under the list, the card for moving to keys: what is left before
//! the home computer can stop answering on the network.
//!
//! The page is in the sidebar when the server connects devices with keys, and on the home computer while keys are
//! off, where it is one card that turns them on (settings.rs `in_sidebar`). On the home computer it also says
//! which relay is in use, and changes it. What it says and in what order is worked out in sb_core::keys, which has
//! the tests; this file lays it out.

use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, div, prelude::*, px, relative,
};
use sb_core::keys::{
    self, Device, DeviceRow, HeldDevice, HeldList, ModeRefusal, Moving, NodeStatus, OfferView, PairOffer, Tone,
};

use super::SettingsView;
use crate::state::AppState;
use crate::theme::{NUM, Theme};
use crate::ui;

/// While a code shows, the countdown moves every second and the list is read again every other one, so the card
/// turns to "paired" within a couple of seconds of the device joining.
const TICK: Duration = Duration::from_secs(1);
const LIST_EVERY: u32 = 2;
/// While the page is open and no code shows, the list is read again this often, so "Not connected yet" becomes
/// "Connected" without the page being left and opened again.
const PAGE_EVERY: Duration = Duration::from_secs(10);
/// A Copy button says "Copied" for this long (seconds).
const COPIED_FOR: f64 = 1.6;
/// The QR tile's side, in points.
const QR_SIDE: f32 = 176.;
/// How long the page waits for a server that is restarting with a new setting, in ticks.
const RESTART_TICKS: u32 = 60;

#[derive(Default)]
pub(super) struct Pane {
    devices: Option<Result<Vec<Device>, String>>,
    loading: bool,
    /// Add a device: the code showing.
    add: Option<Add>,
    /// The new computer whose folder question waits for that card to go.
    folder_wait: FolderWait,
    /// Screenshots: the Remove confirm that SB_SHOT_DEVICES=remove asks for has been opened.
    #[cfg_attr(not(feature = "shots"), allow(dead_code))]
    shot_removed: bool,
    /// A code was asked for and has not come yet, and why the last ask failed.
    asking: bool,
    add_err: Option<String>,
    /// The device whose Remove was clicked: its row is open, saying what removal does.
    confirm: Option<String>,
    /// The device being removed, and why the last removal failed.
    removing: Option<String>,
    error: Option<String>,
    /// Which Copy button was last pressed, and when.
    copied: Option<(Copied, f64)>,
    /// Close the network port asks twice.
    close_armed: Option<f64>,
    closing: bool,
    close_err: Option<String>,
    /// Bumped when the page closes or a new code is asked for, so an older countdown stops.
    watch: u64,
    /// Bumped when the page opens or closes, so an older look at the list stops.
    page_watch: u64,
    /// How many looks at the list failed in a row.
    misses: u32,
    /// Pair this computer: going, and why the last try failed.
    pairing: bool,
    pair_err: Option<String>,
    /// The last try said this computer is paired with another home computer: the button says Replace, and the
    /// next press says so to the node.
    pair_replace: bool,
    /// Turn keys on: going, what the server said instead of doing it, and any other failure.
    turning: bool,
    turn_no: Option<ModeRefusal>,
    turn_err: Option<String>,
    /// What the server said instead of closing the port.
    close_no: Option<ModeRefusal>,
    /// The server took a new setting and is restarting: the page waits for it. `restart_late` once it has waited
    /// long enough to say so.
    restarting: bool,
    restart_late: bool,
    mode_watch: u64,
    /// The relay: its field is open, a change is being sent, and why the last one failed.
    relay_open: bool,
    relay_busy: bool,
    relay_err: Option<String>,
    /// On a computer that joined a home: the home computer's list of devices as this computer holds it, the
    /// device whose Block or Unblock is being sent, and why the last one failed.
    held: Option<HeldList>,
    blocking: Option<String>,
    block_err: Option<String>,
}

/// A computer that has just paired shows up as a new machine, and Settings would open its page to ask for its
/// Claude folder (state.rs `ask_roots`). While Add a device is showing (the code, then "is paired") that would pull
/// the user off the card, so the question waits here until the card goes.
#[derive(Default, Debug)]
pub(super) struct FolderWait(Option<String>);

impl FolderWait {
    /// A machine's folder question is due. True when it waits (`on_devices`: the Devices page is showing;
    /// `adding`: its Add a device card is up).
    pub(super) fn hold(&mut self, on_devices: bool, adding: bool, machine: &str) -> bool {
        let wait = on_devices && adding;
        if wait {
            self.0 = Some(machine.to_string());
        }
        wait
    }

    /// The card has gone: the machine whose question waited, once.
    pub(super) fn release(&mut self) -> Option<String> {
        self.0.take()
    }

    /// The page was left: nothing is opened later. The question was not marked as asked, so it comes again.
    pub(super) fn forget(&mut self) {
        self.0 = None;
    }
}

/// True the first time it is asked with this flag, and never again: for what a screenshot hook does once.
#[cfg_attr(not(feature = "shots"), allow(dead_code))]
fn first_time(done: &mut bool) -> bool {
    !std::mem::replace(done, true)
}

struct Add {
    offer: PairOffer,
    qr: Option<crate::qr::Qr>,
    /// The devices there were when the code was made: one that is not among them is the one that joined.
    known: Vec<String>,
    paired: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Copied {
    Code,
    Link,
    /// The command a server that cannot restart itself asks for.
    Command,
    /// This home computer's address, for a link without a relay.
    Address,
}

/// A Copy button's label: "Copied" for a moment after it was pressed.
fn copy_label(copied: Option<(Copied, f64)>, which: Copied, now: f64) -> &'static str {
    match (copied, which) {
        (Some((c, at)), _) if c == which && (0.0..COPIED_FOR).contains(&(now - at)) => "Copied",
        (_, Copied::Code) => "Copy code",
        (_, Copied::Link) => "Copy link",
        (_, Copied::Command | Copied::Address) => "Copy",
    }
}

/// A code's two halves as they are drawn, "ABCD" and "EFGH".
fn code_halves(code: &str) -> (String, String) {
    let shown = keys::group_code(code);
    match shown.split_once('-') {
        Some((a, b)) => (a.to_string(), b.to_string()),
        None => (shown, String::new()),
    }
}

impl SettingsView {
    pub(super) fn devices_open(&mut self, cx: &mut Context<Self>) {
        self.dev.error = None;
        self.devices_load(cx);
        self.app.update(cx, |s, cx| s.load_keys(cx));
        self.devices_keep_fresh(cx);
        // Screenshots: SB_SHOT_DEVICES=add opens with a code showing.
        #[cfg(feature = "shots")]
        if std::env::var("SB_SHOT_DEVICES").is_ok_and(|v| v == "add") {
            self.devices_add(cx);
        }
        // SB_SHOT_DEVICES=relay opens the relay's field; =byhand shows a server that cannot restart itself.
        #[cfg(feature = "shots")]
        match std::env::var("SB_SHOT_DEVICES").as_deref() {
            Ok("relay") => self.dev.relay_open = true,
            Ok("byhand") => {
                self.dev.turn_no = Some(ModeRefusal {
                    text: "Nothing was changed. This server was started by hand from a copy of the code, so it cannot \
                           restart itself. To turn keys on, run this on the home computer, then start the server \
                           again:"
                        .into(),
                    command: Some("python3 /srv/switchboard/server/keys.py on".into()),
                    blockers: vec![],
                })
            }
            Ok("restarting") => self.dev.restarting = true,
            _ => {}
        }
    }

    /// The list and the report, read again every few seconds for as long as this page is open.
    fn devices_keep_fresh(&mut self, cx: &mut Context<Self>) {
        self.dev.page_watch += 1;
        let ticket = self.dev.page_watch;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PAGE_EVERY).await;
                let go_on = this.update(cx, |v, cx| {
                    if v.dev.page_watch != ticket {
                        return false;
                    }
                    // While a code waits, the countdown reads the list itself, and more often.
                    let waiting = v.dev.add.as_ref().is_some_and(|a| a.paired.is_none());
                    if !waiting && v.dev.confirm.is_none() {
                        v.devices_load(cx);
                        v.app.update(cx, |s, cx| s.load_keys(cx));
                    }
                    true
                });
                if !go_on.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    /// A new machine's folder question is due (settings.rs `ask_root`). True when it waits for the Add a device
    /// card to go, so the user stays on "is paired".
    pub(super) fn devices_hold_folder(&mut self, machine: &str) -> bool {
        let adding = self.dev.add.is_some();
        self.dev.folder_wait.hold(self.section == super::Section::Devices, adding, machine)
    }

    pub(super) fn devices_close(&mut self) {
        self.dev.watch += 1;
        self.dev.page_watch += 1;
        self.dev.add = None;
        self.dev.folder_wait.forget();
        self.dev.confirm = None;
        self.dev.close_armed = None;
        self.dev.add_err = None;
        self.dev.relay_open = false;
        self.dev.relay_err = None;
        self.dev.turn_no = None;
        self.dev.turn_err = None;
        self.dev.close_no = None;
    }

    /// The list this computer holds, from its own node: asked whenever the page looks at the server's list, and
    /// answered with the home computer away too.
    fn devices_load_held(&mut self, cx: &mut Context<Self>) {
        let app = self.app.read(cx);
        if !app.keys.joined() {
            self.dev.held = None;
            return;
        }
        let node = app.node.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { node.held() }).await;
            let _ = this.update(cx, |v, cx| {
                // (a look that failed keeps what shows: the node is restarting, or busy)
                if let Ok(list) = res
                    && v.dev.held.as_ref() != Some(&list)
                {
                    v.dev.held = Some(list);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Block or Unblock on a row of that list.
    fn devices_block(&mut self, id: String, block: bool, cx: &mut Context<Self>) {
        if self.dev.blocking.is_some() {
            return;
        }
        self.dev.blocking = Some(id.clone());
        self.dev.block_err = None;
        cx.notify();
        let node = self.app.read(cx).node.clone();
        cx.spawn(async move |this, cx| {
            let id2 = id.clone();
            let res = cx.background_executor().spawn(async move { node.block(&id2, block) }).await;
            let _ = this.update(cx, |v, cx| {
                v.dev.blocking = None;
                match res {
                    Ok(()) => {
                        if let Some(d) = v.dev.held.as_mut().and_then(|h| h.devices.iter_mut().find(|d| d.id == id)) {
                            d.blocked = block;
                        }
                        v.devices_load_held(cx);
                    }
                    Err(e) => {
                        let what = if block { "Couldn't block it." } else { "Couldn't unblock it." };
                        v.dev.block_err = Some(format!("{what} {e}"));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn devices_load(&mut self, cx: &mut Context<Self>) {
        self.devices_load_held(cx);
        if self.dev.loading {
            return;
        }
        self.dev.loading = true;
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.keys_devices() }).await;
            let _ = this.update(cx, |v, cx| {
                v.dev.loading = false;
                v.devices_took(res.map_err(|e| e.to_string()), cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// A fresh list: shown, and looked through for the device that joined with the code on screen.
    fn devices_took(&mut self, res: Result<Vec<Device>, String>, cx: &mut Context<Self>) {
        if let (Ok(list), Some(add)) = (&res, self.dev.add.as_mut())
            && add.paired.is_none()
            && let Some(d) = keys::newly_paired(&add.known, list)
        {
            add.paired = Some(d.name.clone());
            // The report under the list counts this device from now on.
            self.app.update(cx, |s, cx| s.load_keys(cx));
        }
        // A list that could not be read while a good one shows (one missed look while a code counts down, or one
        // of the page's own looks) is not worth replacing the page with. A second miss in a row is.
        self.dev.misses = if res.is_ok() { 0 } else { self.dev.misses + 1 };
        if res.is_ok() || !matches!(self.dev.devices, Some(Ok(_))) || (self.dev.add.is_none() && self.dev.misses > 1) {
            self.dev.devices = Some(res);
        }
        // Screenshots: SB_SHOT_DEVICES=remove opens the first other device's Remove, on the first list that comes.
        // Once: the list is looked at again every few seconds, and a confirm that was cancelled stays closed.
        #[cfg(feature = "shots")]
        if std::env::var("SB_SHOT_DEVICES").is_ok_and(|v| v == "remove")
            && let Some(Ok(list)) = &self.dev.devices
            && first_time(&mut self.dev.shot_removed)
        {
            let app = self.app.read(cx);
            let rows = keys::device_rows(list, AppState::now(), app.keys.node.as_ref());
            self.dev.confirm = rows.iter().find(|r| !r.here).map(|r| r.id.clone());
        }
    }

    /// Add a device: asks the home computer for a code, and watches for the device that joins with it.
    fn devices_add(&mut self, cx: &mut Context<Self>) {
        if self.dev.asking {
            return;
        }
        self.dev.asking = true;
        self.dev.add_err = None;
        self.dev.add = None;
        self.dev.watch += 1;
        cx.notify();
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let (known, offer) = cx
                .background_executor()
                .spawn(async move {
                    // The list as it is just before the code exists, so whatever is new after it is the joiner.
                    let known = core.keys_devices();
                    (known, core.pair_device())
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.dev.asking = false;
                match offer {
                    Ok(offer) => {
                        let ids = |l: &[Device]| l.iter().map(|d| d.id.clone()).collect::<Vec<_>>();
                        let known = match (&known, &v.dev.devices) {
                            (Ok(l), _) => ids(l),
                            (Err(_), Some(Ok(l))) => ids(l),
                            _ => Vec::new(),
                        };
                        let qr = crate::qr::Qr::of(&offer.link);
                        v.dev.add = Some(Add { offer, qr, known, paired: None });
                        v.dev.copied = None;
                        // To the very top, page heading included (scroll_to_item stops at the first child).
                        v.scroll.set_offset(gpui::point(px(0.), px(0.)));
                        v.devices_watch(cx);
                    }
                    Err(e) => v.dev.add_err = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Once a second while a code shows: the countdown moves, and every other second the list is read again.
    fn devices_watch(&mut self, cx: &mut Context<Self>) {
        self.dev.watch += 1;
        let ticket = self.dev.watch;
        cx.spawn(async move |this, cx| {
            let mut n = 0u32;
            loop {
                cx.background_executor().timer(TICK).await;
                n += 1;
                let go_on = this.update(cx, |v, cx| {
                    if v.dev.watch != ticket {
                        return false;
                    }
                    let Some(add) = &v.dev.add else { return false };
                    let view = keys::offer_view(&add.offer, add.paired.as_deref(), AppState::now());
                    cx.notify();
                    match view {
                        OfferView::Waiting(_) => {
                            if n.is_multiple_of(LIST_EVERY) {
                                v.devices_load(cx);
                            }
                            true
                        }
                        // One last look: a device that joined in the code's final second still shows as paired.
                        OfferView::Expired => {
                            v.devices_load(cx);
                            false
                        }
                        OfferView::Paired(_) => false,
                    }
                });
                if !go_on.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    fn devices_copy(&mut self, which: Copied, cx: &mut Context<Self>) {
        let Some(add) = &self.dev.add else { return };
        let text = match which {
            Copied::Code => keys::group_code(&add.offer.code),
            _ => add.offer.link.clone(),
        };
        self.devices_copy_text(which, text, cx);
    }

    fn devices_copy_text(&mut self, which: Copied, text: String, cx: &mut Context<Self>) {
        crate::state::copy_text(cx, text);
        self.dev.copied = Some((which, AppState::now()));
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(COPIED_FOR + 0.05)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    fn devices_remove(&mut self, id: String, here: bool, cx: &mut Context<Self>) {
        if self.dev.removing.is_some() {
            return;
        }
        self.dev.removing = Some(id.clone());
        self.dev.error = None;
        cx.notify();
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let id2 = id.clone();
            let res = cx.background_executor().spawn(async move { core.keys_remove_device(&id2) }).await;
            let _ = this.update(cx, |v, cx| {
                v.dev.removing = None;
                match res {
                    Ok(()) => {
                        v.dev.confirm = None;
                        if let Some(Ok(list)) = v.dev.devices.as_mut() {
                            list.retain(|d| d.id != id);
                        }
                        v.devices_load(cx);
                        v.app.update(cx, |s, cx| {
                            s.load_keys(cx);
                            // This computer's own key is gone: the board finds out now, not at the next poll.
                            if here {
                                s.refresh(cx);
                            }
                        });
                    }
                    Err(e) => v.dev.error = Some(format!("Couldn't remove it. {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Close the network port: the first click arms it, the second sends it. Reached only when the server has a
    /// route for the switch (sb_core::keys::CLOSE_PORT_ROUTE); without one there is no button to click.
    fn devices_close_port(&mut self, cx: &mut Context<Self>) {
        if keys::CLOSE_PORT_ROUTE.is_none() || self.dev.closing {
            return;
        }
        let now = AppState::now();
        match super::arm_click(self.dev.close_armed, now) {
            super::ArmClick::Go => {}
            super::ArmClick::Wait => return,
            super::ArmClick::Arm => {
                self.dev.close_armed = Some(now);
                self.disarm_later(cx);
                cx.notify();
                return;
            }
        }
        self.dev.close_armed = None;
        self.dev.closing = true;
        self.dev.close_err = None;
        self.dev.close_no = None;
        cx.notify();
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.keys_set_mode("only") }).await;
            let _ = this.update(cx, |v, cx| {
                v.dev.closing = false;
                match res {
                    Ok(Ok(restarting)) => v.devices_mode_taken(restarting, cx),
                    Ok(Err(no)) => v.dev.close_no = Some(no),
                    Err(e) => v.dev.close_err = Some(format!("Couldn't close the port. {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Turn keys on: asks the server on this home computer to connect devices with keys as well as by address.
    fn devices_turn_on(&mut self, cx: &mut Context<Self>) {
        if self.dev.turning || self.dev.restarting {
            return;
        }
        self.dev.turning = true;
        self.dev.turn_no = None;
        self.dev.turn_err = None;
        cx.notify();
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.keys_set_mode("both") }).await;
            let _ = this.update(cx, |v, cx| {
                v.dev.turning = false;
                match res {
                    Ok(Ok(restarting)) => v.devices_mode_taken(restarting, cx),
                    Ok(Err(no)) => v.dev.turn_no = Some(no),
                    Err(e) => v.dev.turn_err = Some(format!("Couldn't turn keys on. {e}")),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The server took a new setting. When it restarts for it, the page asks once a second until the server says
    /// it runs with the new setting, then reads the list again.
    fn devices_mode_taken(&mut self, restarting: bool, cx: &mut Context<Self>) {
        self.dev.mode_watch += 1;
        self.dev.restart_late = false;
        if !restarting {
            self.app.update(cx, |s, cx| s.load_keys(cx));
            self.devices_load(cx);
            return;
        }
        self.dev.restarting = true;
        let ticket = self.dev.mode_watch;
        let was = self.app.read(cx).keys.server.as_ref().map(|r| r.keys);
        cx.spawn(async move |this, cx| {
            for _ in 0..RESTART_TICKS {
                cx.background_executor().timer(TICK).await;
                let go_on = this.update(cx, |v, cx| {
                    if v.dev.mode_watch != ticket {
                        return false;
                    }
                    if v.app.read(cx).keys.server.as_ref().map(|r| r.keys) != was {
                        v.dev.restarting = false;
                        v.dev.devices = None;
                        v.devices_load(cx);
                        cx.notify();
                        return false;
                    }
                    v.app.update(cx, |s, cx| s.load_keys(cx));
                    true
                });
                if !go_on.unwrap_or(false) {
                    return;
                }
            }
            let _ = this.update(cx, |v, cx| {
                if v.dev.mode_watch == ticket {
                    v.dev.restarting = false;
                    v.dev.restart_late = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Change, on the relay line: opens the field, holding the relay that was set here.
    fn devices_relay_change(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        let app = self.app.read(cx);
        let mine = app
            .keys
            .node
            .as_ref()
            .filter(|n| n.relay_source.as_deref() == Some("settings"))
            .and_then(|n| n.relay.clone())
            .unwrap_or_default();
        self.dev.relay_open = true;
        self.dev.relay_err = None;
        self.dev_relay.update(cx, |f, cx| {
            f.set_value(mine, window, cx);
            f.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn devices_relay_edited(&mut self, cx: &mut Context<Self>) {
        if self.dev.relay_err.take().is_some() {
            cx.notify();
        }
    }

    /// Save, on the relay's field: the node on this home computer takes the address (or none) and reconnects.
    pub(super) fn devices_relay_save(&mut self, cx: &mut Context<Self>) {
        if self.dev.relay_busy || !self.dev.relay_open {
            return;
        }
        let url = match keys::relay_input(&self.dev_relay.read(cx).value()) {
            Ok(u) => u,
            Err(e) => {
                self.dev.relay_err = Some(e);
                cx.notify();
                return;
            }
        };
        self.dev.relay_busy = true;
        self.dev.relay_err = None;
        cx.notify();
        let node = self.app.read(cx).node.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { node.set_relay(&url) }).await;
            let _ = this.update(cx, |v, cx| {
                v.dev.relay_busy = false;
                match res {
                    Ok(()) => {
                        v.dev.relay_open = false;
                        v.app.update(cx, |s, cx| s.load_keys(cx));
                    }
                    Err(e) => v.dev.relay_err = Some(format!("Couldn't change the relay. {e}")),
                }
                cx.notify();
            });
            // The node reconnects with the new relay: one more look, so the line says whether it connected.
            cx.background_executor().timer(Duration::from_secs(3)).await;
            let _ = this.update(cx, |v, cx| v.app.update(cx, |s, cx| s.load_keys(cx)));
        })
        .detach();
    }

    /// Pair this computer: the app is connected by address already, so it asks the home computer for a code and
    /// hands it to this computer's node itself. Nothing to type.
    fn devices_pair_here(&mut self, cx: &mut Context<Self>) {
        if self.dev.pairing {
            return;
        }
        self.dev.pairing = true;
        self.dev.pair_err = None;
        let replace = std::mem::take(&mut self.dev.pair_replace);
        cx.notify();
        let (core, node) = {
            let app = self.app.read(cx);
            (app.core.clone(), app.node.clone())
        };
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let offer = core.pair_device().map_err(|e| (format!("Couldn't get a code. {e}"), false))?;
                    // The link carries where home is; the code alone has to find it first.
                    let input = if offer.link.is_empty() { keys::group_code(&offer.code) } else { offer.link };
                    let name = keys::this_computer();
                    let (tries, pause) = (keys::HOME_HERE_TRIES, keys::HOME_HERE_PAUSE);
                    keys::join_home(&*node, &input, name.as_deref(), &[], replace, tries, pause).map_err(|e| {
                        let p = keys::join_problem(&e);
                        (p.says(), p == keys::JoinProblem::OtherHome)
                    })
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                v.dev.pairing = false;
                match res {
                    Ok((_, here)) => {
                        v.devices_load(cx);
                        v.app.update(cx, |s, cx| {
                            s.load_keys(cx);
                            // From now on the node carries the home computer to a port on this computer (the one
                            // it says): the app knows that address too, so it keeps working once the network port
                            // closes.
                            let _ = s.use_server(&here, cx);
                        });
                    }
                    Err((e, other_home)) => (v.dev.pair_err, v.dev.pair_replace) = (Some(e), other_home),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---------- the page ----------

    pub(super) fn devices(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let now = AppState::now();
        let k = self.app.read(cx).keys.clone();
        // While keys are off nothing is paired: the line says what turning them on brings.
        let sub = keys::devices_intro(k.on());
        let page = div().child(Self::head(t, "Devices", sub));
        if !k.on() && (k.can_turn_on() || self.dev.turning || self.dev.restarting) {
            return page.child(self.devices_turn_on_card(t, cx)).into_any_element();
        }
        let held = self.dev.held.as_ref().filter(|_| k.joined()).map(|h| self.devices_held(t, h, now, cx));
        if !k.on() && held.is_some() {
            // This computer joined a home and the home computer has not been asked yet this run (it is away):
            // what this computer holds is all there is to show.
            return div()
                .child(Self::head(t, "Devices", keys::DEVICES_INTRO_AWAY))
                .child(div().mt(px(-14.)).children(held))
                .into_any_element();
        }
        if !k.on() {
            // The page was open when keys went off (or the server changed): nothing to list.
            return page
                .child(
                    ui::wrap("This server connects by address only, so it has no paired devices.")
                        .text_size(px(13.))
                        .text_color(t.dim),
                )
                .into_any_element();
        }
        let rows = match &self.dev.devices {
            Some(Ok(list)) => Some(keys::device_rows(list, now, k.node.as_ref())),
            _ => None,
        };
        page.children(self.dev.add.as_ref().map(|a| self.devices_add_card(t, a, now, cx)))
            .child(self.devices_head(t, rows.as_deref(), cx))
            .child(self.devices_list(t, rows.as_deref(), cx))
            .children(k.node.as_ref().filter(|_| k.home_here()).and_then(|n| self.devices_relay(t, n, now, cx)))
            .when(k.offer_join(), |d| d.child(self.devices_here(t, cx)))
            .children(held)
            .children(k.server.as_ref().and_then(|r| keys::moving(r, now)).map(|m| self.devices_moving(t, &m, now, cx)))
            .when(k.only(), |d| {
                d.child(
                    div()
                        .mt(px(18.))
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .text_size(px(12.5))
                        .text_color(t.dim)
                        .child(div().pt(px(2.)).child(ui::icon("lock", 13., t.faint)))
                        .child(
                            ui::wrap("The network port is closed. This home computer answers paired devices only.")
                                .line_height(relative(1.45)),
                        ),
                )
            })
            .into_any_element()
    }

    /// On a computer that joined a home: the list of devices it holds, how old its copy is, and Block or
    /// Unblock on each device it could answer. It is what this computer goes by while the home computer is
    /// away, so it shows then too.
    fn devices_held(&self, t: &Theme, held: &HeldList, now: f64, cx: &Context<Self>) -> AnyElement {
        let mut col = div().w_full().flex().flex_col();
        for (i, d) in held.devices.iter().enumerate() {
            col = col.child(self.devices_held_row(t, d, i, cx));
        }
        if held.devices.iter().any(HeldDevice::can_block) {
            col = col.child(
                ui::wrap(keys::BLOCK_EXPLAINS)
                    .mt(px(10.))
                    .text_size(px(12.5))
                    .line_height(relative(1.5))
                    .text_color(t.dim),
            );
        }
        col = col.children(self.dev.block_err.as_ref().map(|e| div().mt(px(8.)).child(self.devices_trouble(t, e))));
        let body = keys::held_line(held, now);
        div()
            .mt(px(22.))
            .child(ui::setup_card(t, "laptop", keys::HELD_TITLE, body, Some(col.into_any_element())))
            .into_any_element()
    }

    fn devices_held_row(&self, t: &Theme, d: &HeldDevice, i: usize, cx: &Context<Self>) -> AnyElement {
        let busy = self.dev.blocking.as_deref() == Some(d.id.as_str());
        let (id, block) = (d.id.clone(), !d.blocked);
        let label = if d.blocked { "Unblock" } else { "Block" };
        let button = ui::ghost(t, SharedString::from(format!("dev-block-{i}")), Some(label), None)
            .h(px(28.))
            .px(px(8.))
            .when(busy, |b| b.opacity(0.6))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.devices_block(id.clone(), block, cx)));
        let tag = if d.home {
            Some("Home computer")
        } else if d.here {
            Some("This computer")
        } else if d.blocked {
            Some("Blocked here")
        } else {
            None
        };
        div()
            .min_h(px(36.))
            .py(px(4.))
            .flex()
            .items_center()
            .gap(px(10.))
            .when(i > 0, |r| r.border_t_1().border_color(t.line))
            .child(ui::icon(if d.phone { "phone" } else { "laptop" }, 16., t.dim))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        ui::wrap(d.name.clone())
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if d.blocked { t.dim } else { t.ink }),
                    )
                    .children(tag.map(|w| ui::tag(t, w))),
            )
            .when(d.can_block(), |r| r.child(button))
            .into_any_element()
    }

    /// Keys are off, and this is the home computer: what keys are, in two sentences, and the button that turns
    /// them on. Then the server restarts and the page becomes the list, with Add a device.
    fn devices_turn_on_card(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let busy = self.dev.turning;
        let mut action = div().w_full().flex().flex_col().items_start().gap(px(10.));
        action = if self.dev.restarting {
            action.child(self.devices_waiting(t, "dev-turn-wait", keys::RESTARTING))
        } else {
            action.child(
                ui::primary_btn(t, "dev-turn-on", 32.)
                    .when(busy, |d| d.opacity(0.6).child(ui::spinner("dev-turn-spin", 13., t.on_accent)))
                    .child(if busy { keys::TURN_ON_BUSY } else { keys::TURN_ON_BUTTON })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.devices_turn_on(cx))),
            )
        };
        action = action
            .children(self.dev.turn_no.as_ref().map(|no| self.devices_refusal(t, no, "dev-turn-cmd", cx)))
            .children(self.dev.turn_err.as_ref().map(|e| self.devices_trouble(t, e)))
            .when(self.dev.restart_late, |d| d.child(self.devices_late(t)));
        ui::setup_card(t, "key", keys::TURN_ON_TITLE, keys::TURN_ON_BODY, Some(action.into_any_element()))
            .into_any_element()
    }

    /// The spinner and what is being waited for.
    fn devices_waiting(&self, t: &Theme, id: &'static str, text: &'static str) -> AnyElement {
        div()
            .flex()
            .items_start()
            .gap(px(8.))
            .text_size(px(12.5))
            .text_color(t.ink2)
            .child(div().pt(px(2.)).child(ui::spinner(id, 13., t.dim)))
            .child(ui::wrap(text).line_height(relative(1.45)))
            .into_any_element()
    }

    /// The server did not come back in the time the page waits.
    fn devices_late(&self, t: &Theme) -> AnyElement {
        ui::wrap("The home computer hasn't answered yet. This page updates when it does.")
            .text_size(px(12.5))
            .line_height(relative(1.45))
            .text_color(t.dim)
            .into_any_element()
    }

    /// What the server said instead of changing how devices connect: its sentence, everything that is in the
    /// way, and the command to run when it cannot restart itself.
    fn devices_refusal(&self, t: &Theme, no: &ModeRefusal, id: &'static str, cx: &Context<Self>) -> AnyElement {
        let now = AppState::now();
        let mut col = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(ui::wrap(no.text.clone()).text_size(px(12.5)).line_height(relative(1.5)).text_color(t.ink2));
        for b in &no.blockers {
            col = col.child(self.devices_bullet(t, b));
        }
        if let Some(command) = &no.command {
            let label = copy_label(self.dev.copied, Copied::Command, now);
            let text = command.clone();
            col = col.child(
                div()
                    .w_full()
                    .pl(px(12.))
                    .pr(px(6.))
                    .py(px(6.))
                    .rounded(px(ui::R_CTRL))
                    .bg(t.panel)
                    .border_1()
                    .border_color(t.line)
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_family(NUM)
                            .text_size(px(12.5))
                            .text_color(t.ink)
                            .child(ui::wrap(command.clone()).line_height(relative(1.45))),
                    )
                    .child(
                        ui::line_btn(t, id, 26.)
                            .text_size(px(12.))
                            .child(ui::icon(if label == "Copied" { "check" } else { "copy" }, 12., t.dim))
                            .child(label)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.devices_copy_text(Copied::Command, text.clone(), cx)
                            })),
                    ),
            );
        }
        col.into_any_element()
    }

    /// One thing in a list of what is left, with its ring.
    fn devices_bullet(&self, t: &Theme, line: &str) -> AnyElement {
        div()
            .flex()
            .items_start()
            .gap(px(9.))
            .child(div().flex_none().mt(px(6.)).size(px(6.)).rounded_full().border_1().border_color(t.dim))
            .child(ui::wrap(line.to_string()).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.ink2))
            .into_any_element()
    }

    /// The relay this home computer uses, where that was set, and Change. Without a relay, this computer's
    /// address as well: a link then needs it typed on the computer that joins.
    fn devices_relay(&self, t: &Theme, n: &NodeStatus, now: f64, cx: &Context<Self>) -> Option<AnyElement> {
        let line = keys::relay_line(n)?;
        let open = self.dev.relay_open && line.can_change;
        let busy = self.dev.relay_busy;
        let mut card = div()
            .mt(px(8.))
            .px(px(14.))
            .py(px(12.))
            .rounded(px(ui::R_CARD))
            .border_1()
            .border_color(t.line)
            .bg(t.bg)
            .flex()
            .flex_col()
            .gap(px(12.));
        card = card.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(12.))
                .gap_y(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(180.))
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        // What a relay is for leads; which one, and where it was set, follow more quietly.
                        .child(
                            ui::wrap(line.label.clone())
                                .text_size(px(13.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .line_height(relative(1.4))
                                .text_color(t.ink),
                        )
                        .child(ui::wrap(line.value.clone()).mt(px(2.)).text_size(px(13.)).text_color(t.ink2))
                        .child(
                            ui::wrap(line.note.clone())
                                .text_size(px(12.5))
                                .line_height(relative(1.45))
                                .text_color(if line.tone == Tone::Bad { t.bad } else { t.dim }),
                        ),
                )
                .when(line.can_change, |d| {
                    d.child(
                        ui::ghost(t, "dev-relay-change", Some("Change"), None)
                            .h(px(28.))
                            .px(px(8.))
                            .when(open, |d| d.invisible())
                            .on_click(
                                cx.listener(|this, _: &ClickEvent, window, cx| this.devices_relay_change(window, cx)),
                            ),
                    )
                }),
        );
        if open {
            let field = div()
                .flex_1()
                .min_w(px(220.))
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(ui::R_CTRL))
                .border_1()
                .border_color(if self.dev.relay_err.is_some() { t.bad } else { t.line2 })
                .bg(t.bg)
                .text_size(px(13.))
                .text_color(t.ink)
                .overflow_hidden()
                .child(div().flex_1().min_w_0().child(gpui_base::Input::new(&self.dev_relay)));
            let save = ui::line_btn(t, "dev-relay-save", 32.)
                .when(busy, |d| d.opacity(0.6))
                .child(if busy { "Saving" } else { "Save" })
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.devices_relay_save(cx)));
            let cancel = ui::ghost(t, "dev-relay-cancel", Some("Cancel"), None)
                .h(px(32.))
                .px(px(10.))
                .text_size(px(12.5))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.dev.relay_open = false;
                    this.dev.relay_err = None;
                    cx.notify();
                }));
            card = card
                .child(div().flex().flex_wrap().items_center().gap(px(6.)).child(field).child(save).child(cancel))
                .children(self.dev.relay_err.as_ref().map(|e| self.devices_trouble(t, e)))
                .child(ui::wrap(keys::RELAY_HINT).text_size(px(12.5)).line_height(relative(1.5)).text_color(t.dim));
        }
        if n.relay.is_none()
            && let Some(addr) = keys::home_address(n)
        {
            card = card.child(self.devices_address(t, &addr, "dev-addr-copy", now, cx));
        }
        Some(
            div()
                .mt(px(22.))
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink2).child("Relay"))
                .child(card)
                .into_any_element(),
        )
    }

    /// This home computer's address, to type on a computer that joins with a link that has no relay in it.
    fn devices_address(&self, t: &Theme, addr: &str, id: &'static str, now: f64, cx: &Context<Self>) -> AnyElement {
        let label = copy_label(self.dev.copied, Copied::Address, now);
        let text = addr.to_string();
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(10.))
            .gap_y(px(6.))
            .text_size(px(12.5))
            .text_color(t.dim)
            .child("This computer's address")
            .child(div().font_family(NUM).text_color(t.ink).child(addr.to_string()))
            .child(
                ui::line_btn(t, id, 26.)
                    .text_size(px(12.))
                    .child(ui::icon(if label == "Copied" { "check" } else { "copy" }, 12., t.dim))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.devices_copy_text(Copied::Address, text.clone(), cx)
                    })),
            )
            .into_any_element()
    }

    /// This computer reaches the server by address and its node has no key with it yet: one button pairs it.
    fn devices_here(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let busy = self.dev.pairing;
        let action = div()
            .w_full()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(8.))
            .child(
                ui::setup_btn(
                    t,
                    "dev-pair-here",
                    match (busy, self.dev.pair_replace) {
                        (true, _) => "Pairing",
                        (false, true) => "Replace",
                        (false, false) => "Pair this computer",
                    },
                )
                .when(busy, |d| d.opacity(0.6))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.devices_pair_here(cx))),
            )
            .children(self.dev.pair_err.as_ref().map(|e| self.devices_trouble(t, e)));
        div()
            .mt(px(22.))
            .child(ui::setup_card(
                t,
                "laptop",
                "This computer connects by address",
                "Pair it and it reaches your home computer with its own key, from any network. Nothing to type: \
                 Switchboard gets a code and uses it here.",
                Some(action.into_any_element()),
            ))
            .into_any_element()
    }

    /// "3 devices" and Add a device.
    fn devices_head(&self, t: &Theme, rows: Option<&[DeviceRow]>, cx: &Context<Self>) -> AnyElement {
        let adding = self.dev.add.is_some();
        let asking = self.dev.asking;
        let add = ui::primary_btn(t, "dev-add", 32.)
            .when(asking, |d| d.opacity(0.6))
            .child(if asking {
                ui::spinner("dev-add-spin", 13., t.on_accent)
            } else {
                ui::icon("plus", 13., t.on_accent).into_any_element()
            })
            .child(if asking { "Getting a code" } else { "Add a device" })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.devices_add(cx)));
        div()
            .when(adding, |d| d.mt(px(22.)))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .min_h(px(32.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink2)
                            .child(match rows {
                                // The empty card below says "No devices yet", so the heading does not say it too.
                                Some(r) if !r.is_empty() => keys::device_count(r.len()),
                                _ => "Paired devices".to_string(),
                            }),
                    )
                    .when(!adding, |d| d.child(add)),
            )
            .children(self.dev.add_err.as_ref().map(|e| self.devices_trouble(t, &format!("Couldn't get a code. {e}"))))
            .children(self.dev.error.as_ref().map(|e| self.devices_trouble(t, e)))
            .into_any_element()
    }

    /// One line that says what went wrong, in the page's red.
    fn devices_trouble(&self, t: &Theme, text: &str) -> AnyElement {
        div()
            .flex()
            .items_start()
            .gap(px(8.))
            .text_size(px(12.5))
            .text_color(t.bad)
            .child(div().pt(px(2.)).child(ui::icon("warn", 13., t.bad)))
            .child(ui::wrap(text.to_string()).line_height(relative(1.45)))
            .into_any_element()
    }

    fn devices_list(&self, t: &Theme, rows: Option<&[DeviceRow]>, cx: &Context<Self>) -> AnyElement {
        let rows = match (&self.dev.devices, rows) {
            (None, _) => {
                return div()
                    .mt(px(8.))
                    .child(ui::loading(t, "dev-loading", "Asking your home computer"))
                    .into_any_element();
            }
            (Some(Err(e)), _) => {
                return div()
                    .mt(px(8.))
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(10.))
                    .child(self.devices_trouble(t, &format!("Couldn't read the list of devices. {e}")))
                    .child(
                        ui::line_btn(t, "dev-retry", 30.)
                            .child("Try again")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.devices_load(cx))),
                    )
                    .into_any_element();
            }
            (_, Some(rows)) => rows,
            _ => return div().into_any_element(),
        };
        if rows.is_empty() {
            return div()
                .mt(px(8.))
                .child(ui::setup_card(
                    t,
                    "key",
                    "No devices yet",
                    "Add a device to get a code. Type the code on another computer and it pairs with this home \
                     computer.",
                    None,
                ))
                .into_any_element();
        }
        let mut card =
            div().mt(px(8.)).rounded(px(ui::R_CARD)).border_1().border_color(t.line).bg(t.bg).flex().flex_col();
        for (i, r) in rows.iter().enumerate() {
            card = card.child(self.devices_row(t, r, i, cx));
        }
        card.into_any_element()
    }

    fn devices_row(&self, t: &Theme, r: &DeviceRow, i: usize, cx: &Context<Self>) -> AnyElement {
        let open = self.dev.confirm.as_deref() == Some(r.id.as_str());
        let id = r.id.clone();
        let status = div()
            .flex_none()
            .flex()
            .flex_col()
            .items_end()
            .gap(px(2.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.5))
                    .text_color(if r.connected { t.ink2 } else { t.dim })
                    .when(r.connected, |d| d.child(div().flex_none().size(px(7.)).rounded_full().bg(t.warm)))
                    .child(r.seen.clone()),
            )
            .children(r.path.map(|p| div().text_size(px(12.)).text_color(t.faint).child(p)));
        let remove = ui::ghost(t, SharedString::from(format!("dev-rm-{i}")), Some("Remove"), None)
            .h(px(28.))
            .px(px(8.))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.dev.confirm = Some(id.clone());
                this.dev.error = None;
                cx.notify();
            }));
        div()
            .px(px(14.))
            .py(px(12.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .when(i > 0, |d| d.border_t_1().border_color(t.line))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(px(12.))
                    .gap_y(px(8.))
                    .child(ui::icon(if r.phone { "phone" } else { "laptop" }, 22., t.dim))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(180.))
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        ui::wrap(r.name.clone())
                                            .text_size(px(14.5))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(t.ink),
                                    )
                                    .when(r.here, |d| d.child(ui::tag(t, "This computer"))),
                            )
                            .child(ui::wrap(format!("{}. {}", r.kind, r.paired)).text_size(px(12.5)).text_color(t.dim)),
                    )
                    .child(status)
                    // The button keeps its place while the row is open, so nothing moves under the pointer.
                    .child(remove.when(open, |d| d.invisible())),
            )
            .when(open, |d| d.child(self.devices_confirm(t, r, i, cx)))
            .into_any_element()
    }

    /// What removing this device does, and the two answers.
    fn devices_confirm(&self, t: &Theme, r: &DeviceRow, i: usize, cx: &Context<Self>) -> AnyElement {
        let busy = self.dev.removing.as_deref() == Some(r.id.as_str());
        let (id, here) = (r.id.clone(), r.here);
        let yes = ui::line_btn(t, SharedString::from(format!("dev-rm-yes-{i}")), 30.)
            .text_color(t.bad)
            .when(busy, |d| d.opacity(0.6))
            .child(if busy { "Removing".to_string() } else { format!("Remove {}", r.name) })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.devices_remove(id.clone(), here, cx)));
        let no = ui::ghost(t, SharedString::from(format!("dev-rm-no-{i}")), Some("Keep it"), None)
            .h(px(30.))
            .px(px(10.))
            .text_size(px(12.5))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.dev.confirm = None;
                cx.notify();
            }));
        div()
            .p(px(12.))
            .rounded(px(ui::R_CTRL))
            .bg(t.bad.alpha(if t.dark { 0.1 } else { 0.06 }))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(ui::wrap(keys::remove_explains(r)).text_size(px(12.5)).line_height(relative(1.5)).text_color(t.ink2))
            .child(div().flex().flex_wrap().items_center().gap(px(6.)).child(yes).child(no))
            .into_any_element()
    }

    /// Add a device: the code, the link and its QR code, and the countdown. Then "paired", or "expired".
    fn devices_add_card(&self, t: &Theme, add: &Add, now: f64, cx: &Context<Self>) -> AnyElement {
        let view = keys::offer_view(&add.offer, add.paired.as_deref(), now);
        let done = |label: &'static str| {
            ui::ghost(t, "dev-add-done", Some(label), None).h(px(30.)).px(px(10.)).text_size(px(12.5)).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.dev.add = None;
                    this.dev.watch += 1;
                    cx.notify();
                    // The computer that just paired: its folder is asked for now that the card has been read.
                    if let Some(machine) = this.dev.folder_wait.release() {
                        this.app.update(cx, |s, _| s.root_was_asked(&machine));
                        this.open_machine(machine, cx);
                    }
                }),
            )
        };
        let card = div().p(px(20.)).rounded(px(ui::R_CARD)).border_1().border_color(t.line).bg(t.bg);
        if let OfferView::Paired(name) = &view {
            return card
                .flex()
                .items_start()
                .gap(px(14.))
                .child(
                    div()
                        .flex_none()
                        .size(px(34.))
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(t.warm.alpha(if t.dark { 0.18 } else { 0.12 }))
                        .child(ui::icon("check", 17., t.warm)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            ui::wrap(format!("{name} is paired"))
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.ink),
                        )
                        .child(
                            ui::wrap("It connects with its own key from now on, from any network.")
                                .text_size(px(12.5))
                                .line_height(relative(1.45))
                                .text_color(t.dim),
                        )
                        .child(
                            div()
                                .mt(px(10.))
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    ui::line_btn(t, "dev-add-more", 30.)
                                        .child("Add another device")
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.devices_add(cx))),
                                )
                                .child(done("Done")),
                        ),
                )
                .into_any_element();
        }
        let expired = view == OfferView::Expired;
        let left = match view {
            OfferView::Waiting(l) => l,
            _ => 0.,
        };
        // A home computer without a relay gives no short code: the link and its QR code are the way in.
        let no_code = add.offer.code.is_empty();
        let (a, b) = code_halves(&add.offer.code);
        let ink = if expired { t.faint } else { t.ink };
        let half = |s: &str| {
            div().flex().children(s.chars().map(|c| div().w(px(27.)).flex().justify_center().child(c.to_string())))
        };
        let code = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .font_family(NUM)
            .text_size(px(36.))
            .line_height(px(44.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ink)
            .child(half(&a))
            .when(!b.is_empty(), |d| {
                // As dark as the letters are when the code has run out, and never fainter than the words under it.
                d.child(div().flex_none().w(px(14.)).h(px(3.)).rounded(px(1.5)).bg(if expired {
                    t.faint
                } else {
                    t.dim
                }))
                .child(half(&b))
            });
        let (words, clock) = keys::offer_parts(left);
        let count = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.5))
            .text_color(t.dim)
            .child(ui::icon("clock", 13., t.faint))
            .child(words)
            .children(clock.map(|c| div().font_family(NUM).text_size(px(12.)).text_color(t.ink2).child(c)));
        let buttons = if expired {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .child(
                    ui::primary_btn(t, "dev-add-new", 30.)
                        .when(self.dev.asking, |d| d.opacity(0.6))
                        .child(if no_code { "Get a new link" } else { "Get a new code" })
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.devices_add(cx))),
                )
                .child(done("Close"))
        } else {
            let copy = |id: &'static str, which: Copied| {
                let label = copy_label(self.dev.copied, which, now);
                ui::line_btn(t, id, 30.)
                    .child(ui::icon(if label == "Copied" { "check" } else { "copy" }, 13., t.dim))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.devices_copy(which, cx)))
            };
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .when(!no_code, |d| d.child(copy("dev-copy-code", Copied::Code)))
                .when(!add.offer.link.is_empty(), |d| d.child(copy("dev-copy-link", Copied::Link)))
                .child(done("Done"))
        };
        let words = div()
            .flex_1()
            .min_w(px(280.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink2)
                            .child(if no_code { "Link for one device" } else { "Code for one device" }),
                    )
                    .when(!no_code, |d| d.child(code))
                    .child(count),
            )
            .child(
                ui::wrap(if expired {
                    "Nothing joined before it ran out."
                } else if no_code {
                    keys::NO_CODE
                } else {
                    keys::OFFER_HOW
                })
                .max_w(px(400.))
                .text_size(px(12.5))
                .line_height(relative(1.5))
                .text_color(t.dim),
            )
            .children(
                self.app
                    .read(cx)
                    .keys
                    .node
                    .as_ref()
                    .filter(|n| no_code && !expired && n.role == keys::Role::Home)
                    .and_then(keys::home_address)
                    .map(|a| self.devices_address(t, &a, "dev-add-addr-copy", now, cx)),
            )
            .child(buttons);
        card.flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(24.))
            .gap_y(px(18.))
            .child(words)
            .children(add.qr.as_ref().map(|qr| {
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(8.))
                    .when(expired, |d| d.opacity(0.25))
                    .child(crate::qr::tile(t, qr, QR_SIDE))
                    .child(div().text_size(px(11.5)).text_color(t.faint).child("The link, as a QR code"))
            }))
            .into_any_element()
    }

    /// Moving to keys: what is left before the network port can close, or that it can.
    fn devices_moving(&self, t: &Theme, m: &Moving, now: f64, cx: &Context<Self>) -> AnyElement {
        let mut col = div().w_full().flex().flex_col().gap(px(8.));
        for line in &m.left {
            col = col.child(self.devices_bullet(t, line));
        }
        let can_switch = self.app.read(cx).keys.server.as_ref().is_some_and(|r| r.switch);
        if m.ready {
            col = col.child(
                ui::wrap(keys::CLOSE_EXPLAINS).text_size(px(12.5)).line_height(relative(1.5)).text_color(t.ink2),
            );
            col = match keys::CLOSE_PORT_ROUTE.filter(|_| can_switch) {
                Some(_) if self.dev.restarting => col.child(self.devices_waiting(t, "dev-close-wait", keys::CLOSING)),
                Some(_) => {
                    let armed = self.dev.close_armed.is_some_and(|at| now - at < super::ARM_FOR);
                    col.child(
                        div().mt(px(2.)).flex().child(
                            ui::line_btn(t, "dev-close-port", 32.)
                                .when(armed, |d| d.text_color(t.bad))
                                .when(self.dev.closing, |d| d.opacity(0.6))
                                .child(ui::icon("lock", 13., if armed { t.bad } else { t.dim }))
                                .child(if armed { keys::CLOSE_BUTTON_ARMED } else { keys::CLOSE_BUTTON })
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.devices_close_port(cx))),
                        ),
                    )
                    .children(self.dev.close_no.as_ref().map(|no| self.devices_refusal(t, no, "dev-close-cmd", cx)))
                    .children(self.dev.close_err.as_ref().map(|e| self.devices_trouble(t, e)))
                    .when(self.dev.restart_late, |d| d.child(self.devices_late(t)))
                }
                // A server from before the route: no button, and how to do it by hand.
                None => col.child(
                    div()
                        .px(px(10.))
                        .py(px(8.))
                        .rounded(px(ui::R_CTRL))
                        .bg(t.panel)
                        .border_1()
                        .border_color(t.line)
                        .child(
                            ui::wrap(keys::CLOSE_BY_HAND)
                                .text_size(px(12.5))
                                .line_height(relative(1.5))
                                .text_color(t.dim),
                        ),
                ),
            };
        }
        div()
            .mt(px(22.))
            .child(ui::setup_card(
                t,
                if m.ready { "check" } else { "key" },
                m.title,
                m.body.clone(),
                Some(col.into_any_element()),
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_question_waits_while_the_paired_card_shows() {
        // Add a device is showing its code, or "is paired": the new computer's page does not take its place.
        let mut wait = FolderWait::default();
        assert!(wait.hold(true, true, "workshop"));
        assert!(wait.hold(true, true, "workshop"), "asked again while it waits: still one question");
        // Done: the card goes, and the question is asked then.
        assert_eq!(wait.release(), Some("workshop".to_string()));
        assert_eq!(wait.release(), None, "once");
        // Nothing showing, or another page: it is asked at once, as before.
        assert!(!wait.hold(true, false, "workshop"));
        assert!(!wait.hold(false, true, "workshop"));
        assert_eq!(wait.release(), None);
        // The page was left with the card up: nothing jumps later, and the question is asked another time.
        assert!(wait.hold(true, true, "tower"));
        wait.forget();
        assert_eq!(wait.release(), None);
    }

    #[test]
    fn a_screenshot_hook_runs_on_the_first_list_only() {
        // SB_SHOT_DEVICES=remove: the list loads every few seconds, and the confirm opened once stays as it is left.
        let mut done = false;
        assert_eq!([first_time(&mut done), first_time(&mut done), first_time(&mut done)], [true, false, false]);
    }

    #[test]
    fn a_copy_button_says_copied_for_a_moment() {
        assert_eq!(copy_label(None, Copied::Code, 10.), "Copy code");
        assert_eq!(copy_label(None, Copied::Link, 10.), "Copy link");
        let pressed = Some((Copied::Code, 10.));
        assert_eq!(copy_label(pressed, Copied::Code, 10.5), "Copied");
        assert_eq!(copy_label(pressed, Copied::Link, 10.5), "Copy link", "only the one that was pressed");
        assert_eq!(copy_label(pressed, Copied::Code, 10.01 + COPIED_FOR), "Copy code");
        assert_eq!(copy_label(pressed, Copied::Code, 9.), "Copy code", "a clock that went back shows nothing odd");
    }

    #[test]
    fn a_code_is_drawn_in_two_halves() {
        assert_eq!(code_halves("ABCD-EFGH"), ("ABCD".to_string(), "EFGH".to_string()));
        assert_eq!(code_halves("abcdefgh"), ("ABCD".to_string(), "EFGH".to_string()));
        assert_eq!(code_halves("A"), ("A".to_string(), String::new()));
        assert_eq!(code_halves(""), (String::new(), String::new()));
    }

    #[test]
    fn the_buttons_that_change_how_devices_connect_use_the_servers_route() {
        // A server says it has the route in its readiness report; for one that does not, the page shows how to do
        // it by hand (`devices_moving`) and Devices stays out of the sidebar while keys are off.
        assert_eq!(keys::CLOSE_PORT_ROUTE, Some("/api/keys/mode"));
        assert_eq!(copy_label(None, Copied::Command, 1.), "Copy");
        assert_eq!(copy_label(Some((Copied::Address, 1.)), Copied::Address, 1.5), "Copied");
    }
}
