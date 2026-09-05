//! Every filesystem access in this crate goes through a `Root`.
//!
//! That is not ceremony. The claim RavenControls makes -- that it drives the
//! keyboard light and the fans on a machine none of us has ever seen -- is only
//! testable if the discovery code can be pointed at a *captured* `/sys` instead
//! of the live one. `Root::at("tests/fixtures/thinkpad-t14")` is how a ThinkPad
//! is regression-tested from a Zephyrus, and `raven-controls --capture` is how
//! a stranger's machine becomes a fixture.
//!
//! Paths handed to these methods are always written absolute, exactly as they
//! appear in kernel documentation, so the code reads like the ABI it targets.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Root {
    base: PathBuf,
    /// A fixture is never written to, however tempting. Discovery code does not
    /// need to know which kind of root it holds.
    read_only: bool,
}

impl Default for Root {
    fn default() -> Self {
        Self::system()
    }
}

impl Root {
    /// The live machine.
    pub fn system() -> Self {
        Self {
            base: PathBuf::from("/"),
            read_only: false,
        }
    }

    /// A captured tree. Writes against it fail rather than escaping into `/`.
    pub fn at(base: impl Into<PathBuf>) -> Self {
        Self {
            base: base.into(),
            read_only: true,
        }
    }

    pub fn is_live(&self) -> bool {
        !self.read_only
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Join an absolute kernel path onto the root.
    pub fn path(&self, absolute: &str) -> PathBuf {
        self.base.join(absolute.trim_start_matches('/'))
    }

    pub fn exists(&self, absolute: &str) -> bool {
        self.path(absolute).exists()
    }

    /// Trimmed file contents, or `None` for anything unreadable. Sysfs is full
    /// of attributes that exist but return `-EINVAL` on read, and a driver that
    /// refuses to answer is indistinguishable, for our purposes, from one that
    /// is not there.
    pub fn read(&self, absolute: &str) -> Option<String> {
        let raw = fs::read(self.path(absolute)).ok()?;
        Some(String::from_utf8_lossy(&raw).trim().to_string())
    }

    pub fn read_u32(&self, absolute: &str) -> Option<u32> {
        self.read(absolute)?.parse().ok()
    }

    pub fn read_i64(&self, absolute: &str) -> Option<i64> {
        self.read(absolute)?.parse().ok()
    }

    /// Whitespace-separated integers, for the multicolour LED class's
    /// `multi_intensity` and friends.
    pub fn read_u32_list(&self, absolute: &str) -> Option<Vec<u32>> {
        self.read(absolute)?
            .split_whitespace()
            .map(|t| t.parse().ok())
            .collect()
    }

    /// Directory entries, sorted, as `(name, absolute path)`. Sorting matters:
    /// the UI lists devices in this order, and an order that changes between
    /// runs makes a settings window feel broken.
    pub fn list(&self, absolute: &str) -> Vec<(String, String)> {
        let Ok(dir) = fs::read_dir(self.path(absolute)) else {
            return Vec::new();
        };
        let mut out: Vec<(String, String)> = dir
            .flatten()
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let path = format!("{}/{name}", absolute.trim_end_matches('/'));
                (name, path)
            })
            .collect();
        out.sort_by(|a, b| natural_cmp(&a.0, &b.0));
        out
    }

    /// Can this account write the attribute *right now*? Answered by opening
    /// it, because the mode bits lie: a udev rule may have handed the `video`
    /// group access, or a read-only attribute may be 0644 and still reject
    /// every write. The UI uses this to decide between a live control and an
    /// explanation, so a wrong answer here is a wrong-looking window.
    pub fn writable(&self, absolute: &str) -> bool {
        if self.read_only {
            return false;
        }
        fs::OpenOptions::new()
            .write(true)
            .open(self.path(absolute))
            .is_ok()
    }

    pub fn write(&self, absolute: &str, value: &str) -> io::Result<()> {
        if self.read_only {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{absolute}: this Root is a captured tree, not the live machine"),
            ));
        }
        // Sysfs attributes take one write() of the whole value; no truncate, no
        // append, no partial writes to reason about.
        fs::write(self.path(absolute), value.as_bytes())
    }

    /// Write, then read back and compare.
    ///
    /// Firmware lies. Plenty of laptops accept a write to `pwm1`, return
    /// success, and leave the fan exactly where it was because the EC never
    /// gave up control. Silently pretending that worked is how you end up
    /// with a fan curve that does nothing and a user who trusts it.
    pub fn write_verified(&self, absolute: &str, value: &str) -> anyhow::Result<()> {
        self.write(absolute, value)
            .map_err(|e| anyhow::anyhow!("{absolute}: {e}"))?;
        match self.read(absolute) {
            // Not every attribute reads back what was written -- some round,
            // some are write-only -- so only a *parsed* mismatch is a failure.
            Some(back) => match (back.parse::<i64>(), value.parse::<i64>()) {
                (Ok(got), Ok(want)) if got != want => Err(anyhow::anyhow!(
                    "{absolute}: wrote {want}, firmware kept {got}. The embedded \
                     controller is refusing to hand over this channel."
                )),
                _ => Ok(()),
            },
            None => Ok(()),
        }
    }
}

/// `pwm10` sorts after `pwm9`, and `hwmon10` after `hwmon9`. Plain string order
/// gets both wrong, which shuffles fans around in the window on any machine
/// with ten of them.
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| -> (String, Option<u64>) {
        let digits = s.len() - s.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        let (head, tail) = s.split_at(s.len() - digits);
        (head.to_string(), tail.parse().ok())
    };
    let (ah, an) = split(a);
    let (bh, bn) = split(b);
    ah.cmp(&bh).then_with(|| an.cmp(&bn))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_suffixes_sort_numerically() {
        let mut v = vec!["pwm10", "pwm2", "pwm1", "hwmon3"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, vec!["hwmon3", "pwm1", "pwm2", "pwm10"]);
    }

    #[test]
    fn a_fixture_root_refuses_writes() {
        let root = Root::at("/nonexistent-fixture");
        assert!(root.write("/sys/class/leds/x/brightness", "1").is_err());
        assert!(!root.writable("/sys/class/leds/x/brightness"));
    }
}
