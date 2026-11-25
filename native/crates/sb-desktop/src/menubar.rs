//! The Mac's menu bar: the menus every Mac app has (the app's own with Hide and Services, File, Edit, View, Window,
//! Help) with the keys a Mac user expects of them (Cmd+H, Cmd+M, Cmd+W in every window, Ctrl+Cmd+F), plus the
//! app's own Terminal menu. Titles follow the Mac's menu style: each word capitalised, an ellipsis where a row asks
//! for more before it acts.

use gpui::{App, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, actions};
use gpui_base::input;

use crate::ui::MAC;
use crate::{term, view};

actions!(menubar, [Hide, HideOthers, ShowAll, Minimize, Zoom, FullScreen, Close, Help]);

/// The menu bar, left to right.
pub fn menus() -> Vec<Menu> {
    vec![
        Menu::new("Switchboard").items([
            MenuItem::action("About Switchboard", view::About),
            MenuItem::separator(),
            MenuItem::action("Settings\u{2026}", view::Settings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Switchboard", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Switchboard", view::Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Chat\u{2026}", view::NewSession),
            MenuItem::separator(),
            MenuItem::action("Close", Close),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", input::Undo, OsAction::Undo),
            MenuItem::os_action("Redo", input::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", input::Cut, OsAction::Cut),
            MenuItem::os_action("Copy", input::Copy, OsAction::Copy),
            MenuItem::os_action("Paste", input::Paste, OsAction::Paste),
            MenuItem::os_action("Select All", input::SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Command Palette", view::TogglePalette),
            MenuItem::action("History", view::Browse),
            MenuItem::action("Switch Light and Dark", view::ToggleTheme),
            MenuItem::separator(),
            MenuItem::action("Toggle Full Screen", FullScreen),
        ]),
        Menu::new("Terminal").items([
            MenuItem::action("New Terminal", view::NewTerminal),
            MenuItem::action("Open a Chat in a Terminal\u{2026}", view::ChatPalette),
            MenuItem::separator(),
            MenuItem::action("Bigger Text", term::TermBigger),
            MenuItem::action("Smaller Text", term::TermSmaller),
            MenuItem::action("Actual Size", term::TermSizeReset),
            MenuItem::separator(),
            MenuItem::action("Redraw", term::TermRedraw),
        ]),
        // The Mac adds the list of open windows to the menu called Window, and a search field to the one called Help.
        Menu::new("Window").items([MenuItem::action("Minimize", Minimize), MenuItem::action("Zoom", Zoom)]),
        Menu::new("Help").items([MenuItem::action("Switchboard Help", Help)]),
    ]
}

/// Sets the menu bar, and on a Mac binds the keys a Mac user expects of it: Cmd+H, Option+Cmd+H, Cmd+M, Ctrl+Cmd+F
/// and Cmd+W. The board and a terminal window bind Cmd+W themselves, more closely than this one, so there it still
/// closes the tab first. The rows act on whichever window is in front.
pub fn install(cx: &mut App) {
    cx.set_menus(menus());
    cx.on_action(|_: &Hide, cx: &mut App| cx.hide());
    cx.on_action(|_: &HideOthers, cx: &mut App| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx: &mut App| cx.unhide_other_apps());
    cx.on_action(|_: &Help, cx: &mut App| crate::state::open_link(cx, &sb_core::brand::docs("")));
    cx.on_action(|_: &Minimize, cx: &mut App| front(cx, |window, _| window.minimize_window()));
    cx.on_action(|_: &Zoom, cx: &mut App| front(cx, |window, _| window.zoom_window()));
    cx.on_action(|_: &FullScreen, cx: &mut App| front(cx, |window, _| window.toggle_fullscreen()));
    // Close: a window with tabs or a question of its own (the board, a lane, a terminal) answers Close tab itself;
    // every other window (Settings, About, Files, History, Add a computer) just closes.
    cx.on_action(|_: &Close, cx: &mut App| {
        front(cx, |window, cx| {
            if window.is_action_available(&view::CloseTab, cx) {
                window.dispatch_action(Box::new(view::CloseTab), cx);
            } else {
                window.remove_window();
            }
        })
    });
    if MAC {
        cx.bind_keys([
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("ctrl-cmd-f", FullScreen, None),
            KeyBinding::new("cmd-w", Close, None),
        ]);
    }
}

fn front(cx: &mut App, f: impl FnOnce(&mut gpui::Window, &mut App)) {
    if let Some(w) = cx.active_window() {
        let _ = w.update(cx, |_, window, cx| f(window, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(m: &Menu) -> Vec<String> {
        m.items
            .iter()
            .filter_map(|i| match i {
                MenuItem::Action { name, .. } => Some(name.to_string()),
                MenuItem::SystemMenu(m) => Some(m.name.to_string()),
                MenuItem::Submenu(m) => Some(m.name.to_string()),
                MenuItem::Separator => None,
            })
            .collect()
    }

    #[test]
    fn the_menu_bar_has_the_menus_every_mac_app_has() {
        let names: Vec<String> = menus().iter().map(|m| m.name.to_string()).collect();
        assert_eq!(names, ["Switchboard", "File", "Edit", "View", "Terminal", "Window", "Help"]);
    }

    #[test]
    fn the_app_menu_can_hide_the_app_and_edit_can_copy_and_paste() {
        let all = menus();
        let app = titles(&all[0]);
        let wanted =
            ["About Switchboard", "Services", "Hide Switchboard", "Hide Others", "Show All", "Quit Switchboard"];
        for want in wanted {
            assert!(app.iter().any(|t| t == want), "the app menu has no {want}");
        }
        assert_eq!(titles(&all[2]), ["Undo", "Redo", "Cut", "Copy", "Paste", "Select All"]);
        assert_eq!(titles(&all[5]), ["Minimize", "Zoom"]);
    }

    #[test]
    fn no_title_ends_in_three_dots() {
        for t in menus().iter().flat_map(titles) {
            assert!(!t.ends_with("..."), "{t}: the Mac's menus use a real ellipsis");
            assert!(!t.contains('!') && !t.contains('\u{2014}'), "{t}");
        }
    }
}
