//! Everything the installer draws with is compiled in: the app's fonts and icons (read from sb-desktop's assets, not
//! copied), the header art and the tour's screenshots (native/packaging).

use std::borrow::Cow;

use gpui::{App, AssetSource, SharedString};

macro_rules! files {
    ($dir:literal, $prefix:literal, $ext:literal: $($name:literal),* $(,)?) => {
        &[$((concat!($prefix, $name, $ext), include_bytes!(concat!($dir, $name, $ext)) as &[u8]),)*]
    };
}

static ICONS: &[(&str, &[u8])] = files!("../../../sb-desktop/assets/icons/", "icons/", ".svg":
    "check", "x", "warn", "folder", "lock", "update", "info", "mark-dot", "mark-links", "mark-nodes", "back",
    "chev", "user", "undo", "archive", "stop", "window", "pin",
);

static ART: &[(&str, &[u8])] = files!("../../../../packaging/", "", ".png":
    "installer/header-light", "installer/header-dark",
    "tour/1-board-light", "tour/1-board-dark", "tour/2-inspector-light", "tour/2-inspector-dark",
    "tour/3-new-light", "tour/3-new-dark", "tour/4-history-light", "tour/4-history-dark",
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS.iter().chain(ART).find(|(p, _)| *p == path).map(|(_, b)| Cow::Borrowed(*b)))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS.iter().chain(ART).filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)).collect())
    }
}

pub fn load_fonts(cx: &mut App) {
    let fonts: Vec<Cow<'static, [u8]>> = vec![
        Cow::Borrowed(include_bytes!("../../../sb-desktop/assets/fonts/InstrumentSans-400.ttf")),
        Cow::Borrowed(include_bytes!("../../../sb-desktop/assets/fonts/InstrumentSans-500.ttf")),
        Cow::Borrowed(include_bytes!("../../../sb-desktop/assets/fonts/InstrumentSans-600.ttf")),
        Cow::Borrowed(include_bytes!("../../../sb-desktop/assets/fonts/InstrumentSans-700.ttf")),
        Cow::Borrowed(include_bytes!("../../../sb-desktop/assets/fonts/JetBrainsMono-400.ttf")),
        Cow::Borrowed(include_bytes!("../../../sb-desktop/assets/fonts/Manrope-Bold.ttf")),
    ];
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        log::warn!("could not load the bundled fonts: {e:#}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_asset_is_there() {
        assert!(super::ART.iter().all(|(_, b)| b.starts_with(b"\x89PNG")));
        assert!(super::ICONS.iter().all(|(_, b)| b.starts_with(b"<svg")));
    }
}
