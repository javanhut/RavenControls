//! `org.freedesktop.UPower.KbdBacklight` on the system bus.
//!
//! The LED class in `leds.rs` finds the same hardware, so why carry this at
//! all? Because on a stock distribution `/sys/class/leds/*/brightness` is
//! root-owned, and UPower is the one interface that will set the keyboard light
//! for an ordinary session without a udev rule, a helper or a password. Raven
//! does not run UPower, but RavenControls is not only for Raven, and "install
//! this udev rule first" is a bad first impression on a machine that already
//! had a working answer.
//!
//! So this provider defers: it publishes nothing when the sysfs node is already
//! writable, and steps in when it is not.

use crate::model::*;
use crate::provider::Provider;
use crate::sysfs::Root;

#[zbus::proxy(
    interface = "org.freedesktop.UPower.KbdBacklight",
    default_service = "org.freedesktop.UPower",
    default_path = "/org/freedesktop/UPower/KbdBacklight"
)]
trait KbdBacklight {
    fn get_max_brightness(&self) -> zbus::Result<i32>;
    fn get_brightness(&self) -> zbus::Result<i32>;
    fn set_brightness(&self, value: i32) -> zbus::Result<()>;
}

pub struct UPower {
    /// Connected once. A bus round trip per UI refresh is affordable; a bus
    /// *connection* per refresh is not.
    proxy: Option<KbdBacklightProxyBlocking<'static>>,
}

impl UPower {
    pub fn new() -> Self {
        let proxy = zbus::blocking::Connection::system()
            .ok()
            .and_then(|c| KbdBacklightProxyBlocking::new(&c).ok())
            // Having the proxy proves nothing -- D-Bus will happily hand out a
            // proxy for a name nobody owns. One real call is the test.
            .filter(|p| p.get_max_brightness().is_ok());
        Self { proxy }
    }

    /// True when the kernel already offers this light on terms we prefer.
    fn sysfs_has_it_covered(root: &Root) -> bool {
        root.list(super::leds::CLASS)
            .into_iter()
            .filter(|(name, _)| name.ends_with("kbd_backlight"))
            .any(|(_, dir)| root.writable(&format!("{dir}/brightness")))
    }
}

impl Default for UPower {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for UPower {
    fn id(&self) -> &'static str {
        "upower"
    }

    fn knobs(&self, root: &Root) -> Vec<Knob> {
        // A captured tree has no bus behind it, and driving the *live* session's
        // keyboard light from a fixture would be a genuinely surprising bug.
        if !root.is_live() {
            return Vec::new();
        }
        let Some(proxy) = &self.proxy else {
            return Vec::new();
        };
        if Self::sysfs_has_it_covered(root) {
            return Vec::new();
        }
        let (Ok(max), Ok(current)) = (proxy.get_max_brightness(), proxy.get_brightness()) else {
            return Vec::new();
        };
        if max <= 0 {
            return Vec::new();
        }
        let (max, current) = (max as u32, current.max(0) as u32);
        vec![Knob {
            id: "upower/kbd_backlight".into(),
            label: "Keyboard backlight".into(),
            role: Role::KeyboardBacklight,
            domain: if max <= 1 {
                Domain::Switch
            } else {
                Domain::Percent { raw_max: max }
            },
            value: if max <= 1 {
                Setting::Switch { on: current >= 1 }
            } else {
                Setting::Percent {
                    percent: raw_to_percent(current, max),
                }
            },
            // UPower does its own authorisation and does not need ours.
            writable: true,
            provider: "upower".into(),
            origin: "org.freedesktop.UPower.KbdBacklight".into(),
        }]
    }

    fn set(&self, _root: &Root, knob_id: &str, value: &Setting) -> anyhow::Result<()> {
        let proxy = self
            .proxy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("UPower is not running"))?;
        if knob_id != "upower/kbd_backlight" {
            anyhow::bail!("{knob_id} is not a UPower control");
        }
        let max = proxy.get_max_brightness()?;
        let raw = match value {
            Setting::Percent { percent } => percent_to_raw(*percent, max.max(0) as u32) as i32,
            Setting::Switch { on } => i32::from(*on),
            other => anyhow::bail!("UPower cannot take {other:?}"),
        };
        proxy.set_brightness(raw)?;
        Ok(())
    }
}
