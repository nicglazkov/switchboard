//! Everything the app draws with is compiled in: fonts and the stroke icon set. Nothing is read from disk at start.

use std::borrow::Cow;

use gpui::{App, AssetSource, SharedString};

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((concat!("icons/", $name, ".svg"), include_bytes!(concat!("../assets/icons/", $name, ".svg")) as &[u8]),)*]
    };
}

static ICONS: &[(&str, &[u8])] = icons![
    "archive",
    "auto",
    "back",
    "bg",
    "bolt",
    "check",
    "chev",
    "clock",
    "close",
    "cmd",
    "cols",
    "cont",
    "copy",
    "cube",
    "density",
    "dismiss",
    "dots",
    "down",
    "filter",
    "file",
    "fit",
    "folder",
    "for",
    "fresh",
    "gauge",
    "globe",
    "grid",
    "handoff",
    "history",
    "home",
    "mac",
    "inspector",
    "link",
    "list",
    "lock",
    "laptop",
    "mark-dot",
    "mark-links",
    "mark-nodes",
    "max",
    "mem",
    "min",
    "moon",
    "out",
    "pin",
    "plan",
    "plus",
    "remote",
    "restore",
    "resume",
    "rows",
    "search",
    "send",
    "sidebar",
    "spark",
    "stop",
    "sun",
    "swap",
    "term",
    "unlink",
    "unlock",
    "user",
    "tower",
    "window",
    "x",
    "zoomin",
    "zoomout",
    "flag",
    "reply",
    "undo",
    "contrast",
    "warn",
    "info",
    "tab",
    "popout",
    "newhere",
    "gear",
    "upload",
    "attach",
    "update",
    "up",
    "relaunch",
    "star",
    "star-on",
    "group",
    "group-add",
    "ungroup",
    "pencil",
    "spinner",
    "ring",
];

/// The app's own icon (the AppIcon.icns artwork at 512 px, with the macOS rounded square and its margin), for About.
pub const APP_ICON: &str = "app-icon.png";

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path == APP_ICON {
            return Ok(Some(Cow::Borrowed(include_bytes!("../assets/app-icon.png"))));
        }
        Ok(ICONS.iter().find(|(p, _)| *p == path).map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)).collect())
    }
}

/// True when the icon set has `name` (chips and actions name icons from the shared set).
pub fn has_icon(name: &str) -> bool {
    let p = format!("icons/{name}.svg");
    ICONS.iter().any(|(q, _)| *q == p)
}

pub fn load_fonts(cx: &mut App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/InstrumentSans-400.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/InstrumentSans-500.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/InstrumentSans-600.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/InstrumentSans-700.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-400.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/JetBrainsMono-700.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Manrope-Bold.ttf")),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        log::warn!("could not load the bundled fonts: {e:#}");
    }
}
