//! Switchboard for the desktop: the Swimlanes board for Claude Code sessions on every machine.
//! The first frame paints from the snapshot saved by the last run; live data replaces it as soon as the server
//! answers. Nothing on screen waits on the network.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod about;
mod archive;
mod askcard;
mod browser;
mod chrome;
mod firstrun;
mod flagsui;
mod history;
mod lanes;
mod place;
mod rename;
mod stop;
mod clean;
mod notices;
mod files;
mod frame;
mod assets;
mod claude_app;
mod inspector;
mod library;
mod link;
mod menu;
mod palette;
mod panics;
mod perf;
mod settings;
mod sheet;
mod tabgroups;
mod state;
mod term;
mod theme;
mod ui;
mod update;
mod usage;
mod view;
mod detached;
mod wake;

use gpui::{App, Menu, MenuItem, WindowAppearance};

use crate::state::AppState;
use crate::theme::{Mode, Theme};

/// When main() began, for SB_TIMING.
pub static STARTED: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

fn main() {
    STARTED.get_or_init(std::time::Instant::now);
    panics::install();
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("Switchboard {}", sb_core::version());
        return;
    }
    #[cfg(target_os = "windows")]
    update::sweep_old();
    gpui_platform::application().with_assets(assets::Assets).run(|cx: &mut App| {
        assets::load_fonts(cx);
        gpui_base::init(cx);

        let app = AppState::init(cx);
        // The theme before the first window, so the first frame is already the right one.
        let mode = std::env::var("SB_THEME").ok().map(|v| Mode::parse(Some(v.as_str()))).unwrap_or_else(|| Mode::parse(app.read(cx).settings.theme.as_deref()));
        let dark = match mode {
            Mode::Dark => true,
            Mode::Light => false,
            Mode::System => !matches!(cx.window_appearance(), WindowAppearance::Light | WindowAppearance::VibrantLight),
        };
        if mode != Mode::System {
            app.update(cx, |s, _| s.settings.theme = mode.name().map(str::to_string));
        }
        (if dark { Theme::dark() } else { Theme::light() }).install(cx);

        view::bind_keys(cx);
        perf::watch(cx);
        cx.set_menus(vec![
            Menu::new("Switchboard").items([
                MenuItem::action("About Switchboard", view::About),
                MenuItem::separator(),
                MenuItem::action("Settings...", view::Settings),
                MenuItem::separator(),
                MenuItem::action("Quit Switchboard", view::Quit),
            ]),
            Menu::new("View").items([
                MenuItem::action("History", view::Browse),
                MenuItem::action("Switch light and dark", view::ToggleTheme),
            ]),
            Menu::new("Terminal").items([
                MenuItem::action("Bigger text", term::TermBigger),
                MenuItem::action("Smaller text", term::TermSmaller),
                MenuItem::action("Actual size", term::TermSizeReset),
                MenuItem::separator(),
                MenuItem::action("Redraw", term::TermRedraw),
            ]),
        ]);

        // Clicking a needs-you notification brings the board forward with that chat selected.
        let a2 = app.clone();
        cx.on_system_notification_response(move |r, cx| {
            cx.activate(true);
            let key = r.tag.to_string();
            a2.update(cx, |s, cx| s.select(key, cx));
        });

        // The main window closing quits; a torn-off lane closing does not.
        if let Some(main) = view::open_window(None, cx) {
            cx.on_window_closed(move |cx, id| {
                if id == main || cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        }
        cx.activate(true);
    });
}
