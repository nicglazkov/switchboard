//! Systems with no backend of their own yet (Linux, and Windows until it gets one): installs are plain folder copies
//! into ~/Applications, and nothing outside that folder and the app's own settings is touched. Also the sandbox the
//! tests and screenshots run in.

use std::io;
use std::path::Path;

use super::{Applied, Dir, Platform, home};
use crate::uninstall::Layout;

pub struct Plain;

impl Platform for Plain {
    fn install_dirs(&self) -> Vec<Dir> {
        vec![Dir { path: home().join("Applications"), label: "Applications in your home folder".into() }]
    }

    fn set_login_item(&self, _app: &Path, _on: bool) -> io::Result<Applied> {
        Ok(Applied::Skipped("login items are not supported on this system yet".into()))
    }

    fn set_in_dock(&self, _app: &Path, _on: bool) -> io::Result<Applied> {
        Ok(Applied::Skipped("there is no Dock on this system".into()))
    }

    fn open_app(&self, _app: &Path) -> io::Result<()> {
        Err(io::Error::other("opening apps is not supported on this system yet"))
    }

    fn layout(&self, app: &Path) -> Layout {
        Layout {
            app: app.to_path_buf(),
            login_item: None,
            in_dock: false,
            caches: sb_core::cache::default_dir().into_iter().collect(),
            data: self.settings_dir().into_iter().collect(),
        }
    }
}
