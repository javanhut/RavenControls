//! The contract every backend implements, and the registry that asks each one
//! what it can see.
//!
//! Providers are ordered generic-first. `hwmon` before `asus-nb-wmi`, the LED
//! class before anything vendor-shaped, because the generic interface is the
//! one that keeps working when the vendor driver is renamed, rewritten, or
//! folded into the kernel -- all three of which have happened to laptop fan
//! drivers in the last few years.

use crate::model::{Knob, Reading, Setting};
use crate::sysfs::Root;

pub trait Provider: Send + Sync {
    /// Short, stable, and printed in `--probe`.
    fn id(&self) -> &'static str;

    /// Re-read the hardware. Called on every refresh, so it must be cheap:
    /// sysfs reads only, no probing, no module loading.
    fn knobs(&self, root: &Root) -> Vec<Knob>;

    fn readings(&self, _root: &Root) -> Vec<Reading> {
        Vec::new()
    }

    /// Apply a value to one of this provider's knobs.
    fn set(&self, root: &Root, knob_id: &str, value: &Setting) -> anyhow::Result<()>;
}

/// Everything this machine turned out to have.
pub struct Hardware {
    root: Root,
    providers: Vec<Box<dyn Provider>>,
}

impl Hardware {
    /// Ask every provider what it sees. Providers that see nothing are kept
    /// rather than dropped, so `--probe` can report "hwmon: no PWM channels"
    /// -- which is a different and much more useful fact than silence.
    pub fn discover(root: Root) -> Self {
        let providers: Vec<Box<dyn Provider>> = vec![
            // Keyboard backlight, generic first.
            Box::new(crate::backlight::leds::Leds),
            #[cfg(feature = "upower")]
            Box::new(crate::backlight::upower::UPower::new()),
            // Fans, generic first.
            Box::new(crate::fan::hwmon::Hwmon),
            Box::new(crate::fan::profile::PlatformProfile),
            Box::new(crate::fan::cooling::CoolingDevices),
            Box::new(crate::fan::vendor::VendorKnobs),
        ];
        Self { root, providers }
    }

    pub fn root(&self) -> &Root {
        &self.root
    }

    pub fn providers(&self) -> impl Iterator<Item = &dyn Provider> {
        self.providers.iter().map(|p| p.as_ref())
    }

    pub fn knobs(&self) -> Vec<Knob> {
        let mut out: Vec<Knob> = self
            .providers
            .iter()
            .flat_map(|p| p.knobs(&self.root))
            .collect();
        // Two providers can legitimately reach the same hardware -- UPower and
        // the LED class are the same keyboard light seen twice. First wins,
        // and the ordering above makes that the generic one.
        let mut seen = std::collections::HashSet::new();
        out.retain(|k| seen.insert(k.id.clone()));
        out
    }

    pub fn readings(&self) -> Vec<Reading> {
        self.providers
            .iter()
            .flat_map(|p| p.readings(&self.root))
            .collect()
    }

    pub fn knobs_for(&self, role: crate::model::Role) -> Vec<Knob> {
        self.knobs()
            .into_iter()
            .filter(|k| k.role == role)
            .collect()
    }

    pub fn knob(&self, id: &str) -> Option<Knob> {
        self.knobs().into_iter().find(|k| k.id == id)
    }

    /// Route a set to whichever provider owns the knob, after checking the
    /// value against the domain that provider published.
    pub fn set(&self, knob_id: &str, value: &Setting) -> anyhow::Result<()> {
        let knob = self
            .knob(knob_id)
            .ok_or_else(|| anyhow::anyhow!("no control called {knob_id:?} on this machine"))?;
        value.check(&knob.domain)?;
        if !knob.writable {
            anyhow::bail!(
                "{} is not writable by this account ({})",
                knob.label,
                knob.origin
            );
        }
        let provider = self
            .providers
            .iter()
            .find(|p| p.id() == knob.provider)
            .ok_or_else(|| anyhow::anyhow!("provider {} vanished", knob.provider))?;
        provider.set(&self.root, knob_id, value)
    }
}
