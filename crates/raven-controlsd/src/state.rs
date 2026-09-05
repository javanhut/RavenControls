//! Fan curves that survive a restart.
//!
//! A curve is a setting, not a session: someone who tuned their laptop to be
//! quiet expects it to still be quiet after a reboot, and expects an upgrade of
//! this daemon to be invisible. So curves are written out whenever they change
//! and re-applied at start.
//!
//! Knob ids are the key, which is why `hwmon` builds them from the driver name
//! rather than the enumeration index -- a saved curve keyed on `hwmon3` would
//! land on a different fan after an unrelated module load.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use raven_hw::curve::Curve;
use serde::{Deserialize, Serialize};

pub const DEFAULT_PATH: &str = "/var/lib/raven-controls/state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedCurve {
    pub sensor: String,
    pub curve: Curve,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    /// Ordered so the file has a stable diff between writes.
    #[serde(default)]
    pub curves: BTreeMap<String, SavedCurve>,
}

impl State {
    /// A missing or unreadable state file is an empty state, not an error. The
    /// daemon must start and hand the fans to firmware even when whatever wrote
    /// this file produced nonsense.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!("{}: {e}; starting with no saved curves", path.display());
                State::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
            Err(e) => {
                tracing::warn!("{}: {e}; starting with no saved curves", path.display());
                State::default()
            }
        }
    }

    /// Written through a temporary file and renamed, so a daemon killed
    /// mid-write leaves the previous state rather than half a JSON object that
    /// would be discarded at next boot.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = PathBuf::from(format!("{}.new", path.display()));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}
