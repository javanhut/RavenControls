//! Giving the fans back.
//!
//! Taking manual control of a fan means writing `pwmN_enable=1`, and from that
//! moment the firmware's thermal management is off. If whatever did that stops
//! running -- crashes, is killed, hits a panic, is upgraded out from under
//! itself -- the fan stays exactly where it was left. At 0% on a laptop under
//! load, that is a thermal shutdown at best.
//!
//! So nothing in RavenControls takes control without a `Guard`. It records what
//! each channel was set to before we touched it and puts it back on `Drop`,
//! which covers the ordinary exit, `?` returning early, and unwinding panics
//! alike. The daemon additionally arms it from a signal handler and from a
//! watchdog, because `Drop` does not run for `SIGKILL` or a lost power rail --
//! see `raven-controlsd`.

use std::collections::HashMap;

use crate::sysfs::Root;

/// The channels we have taken over, and what they were.
pub struct Guard {
    root: Root,
    /// `pwmN_enable` attribute path -> the value found there before we wrote.
    original: HashMap<String, String>,
}

impl Guard {
    pub fn new(root: Root) -> Self {
        Self {
            root,
            original: HashMap::new(),
        }
    }

    /// Take manual control of a PWM channel, remembering how to undo it.
    ///
    /// `duty_attr` is the `pwmN` path; the enable attribute is derived, because
    /// hwmon guarantees the naming and deriving it is one fewer thing for a
    /// caller to get wrong.
    pub fn take(&mut self, duty_attr: &str) -> anyhow::Result<()> {
        let enable_attr = format!("{duty_attr}_enable");
        if !self.root.exists(&enable_attr) {
            // A driver with a PWM and no enable attribute is always manual.
            return Ok(());
        }
        let current = self
            .root
            .read(&enable_attr)
            .ok_or_else(|| anyhow::anyhow!("{enable_attr} is unreadable"))?;
        if current == "1" {
            // Already manual. Do not record "1" as the value to restore --
            // that would make releasing a no-op and leave the fan pinned. If
            // something else put it in manual mode, automatic is still the
            // right place to hand it back to.
            self.original
                .entry(enable_attr)
                .or_insert_with(|| "2".to_string());
            return Ok(());
        }
        self.root
            .write_verified(&enable_attr, "1")
            .map_err(|e| anyhow::anyhow!("could not take control of {duty_attr}: {e}"))?;
        self.original.insert(enable_attr, current);
        Ok(())
    }

    pub fn holds(&self, duty_attr: &str) -> bool {
        self.original.contains_key(&format!("{duty_attr}_enable"))
    }

    pub fn held(&self) -> Vec<String> {
        self.original.keys().cloned().collect()
    }

    /// Hand one channel back.
    pub fn release(&mut self, duty_attr: &str) {
        let enable_attr = format!("{duty_attr}_enable");
        if let Some(original) = self.original.remove(&enable_attr) {
            restore(&self.root, &enable_attr, &original);
        }
    }

    /// Hand everything back. Idempotent, and safe to call from a signal
    /// handler's thread as well as from `Drop`.
    pub fn release_all(&mut self) {
        for (attr, original) in std::mem::take(&mut self.original) {
            restore(&self.root, &attr, &original);
        }
    }
}

/// Put a channel back, and if that fails, fall back to firmware control.
///
/// Restoring can genuinely fail -- a GPU that went into runtime suspend while
/// we held its fan will reject the write. Automatic is a worse outcome than the
/// original mode and a far better one than manual-at-whatever-we-left-it, so it
/// is tried second. Both failing is logged as an error, loudly: at that point
/// the fan is in an unknown state and its owner needs to know.
fn restore(root: &Root, enable_attr: &str, original: &str) {
    if root.write(enable_attr, original).is_ok() {
        tracing::info!("{enable_attr} restored to {original}");
        return;
    }
    if root.write(enable_attr, "2").is_ok() {
        tracing::warn!(
            "{enable_attr} could not be set back to {original}; left on firmware automatic"
        );
        return;
    }
    tracing::error!(
        "{enable_attr} could not be handed back to firmware. This fan is still \
         under manual control. Reboot, or write 2 to it as root."
    );
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.release_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixture_guard_takes_nothing_and_releases_cleanly() {
        let mut g = Guard::new(Root::at("/nonexistent"));
        assert!(g.take("/sys/class/hwmon/hwmon0/pwm1").is_ok());
        assert!(!g.holds("/sys/class/hwmon/hwmon0/pwm1"));
        g.release_all();
    }
}
