//! ACPI cooling devices: `/sys/class/thermal/cooling_device*`.
//!
//! Where firmware exposes a fan as an ACPI object (`PNP0C0B`, or Intel's
//! `INT3404`) rather than through a sensor chip, this is where it appears --
//! with `cur_state` and `max_state` instead of a PWM byte. Some tablets and
//! small-form-factor desktops have nothing else.
//!
//! Most entries here are not fans. The same class carries CPU frequency
//! throttling, backlight dimming and device-specific limits, all of which will
//! happily accept a write and none of which should be presented as a fan, so
//! the `type` string is checked rather than assumed.

use crate::model::*;
use crate::provider::Provider;
use crate::sysfs::Root;

const CLASS: &str = "/sys/class/thermal";

pub struct CoolingDevices;

/// ACPI fans call themselves `Fan`; Intel's DPTF driver registers `INT3404 Fan`
/// and `TFN1`. Processor, LCD and the various `*_thermal` entries are throttles
/// and are left alone.
fn is_fan(kind: &str) -> bool {
    let k = kind.to_ascii_lowercase();
    k == "fan" || k.contains("fan")
}

fn devices(root: &Root) -> Vec<(String, String, String)> {
    root.list(CLASS)
        .into_iter()
        .filter(|(name, _)| name.starts_with("cooling_device"))
        .filter_map(|(name, dir)| {
            let kind = root.read(&format!("{dir}/type"))?;
            is_fan(&kind).then_some((name, dir, kind))
        })
        .collect()
}

impl Provider for CoolingDevices {
    fn id(&self) -> &'static str {
        "cooling-device"
    }

    fn knobs(&self, root: &Root) -> Vec<Knob> {
        devices(root)
            .into_iter()
            .filter_map(|(name, dir, kind)| {
                let max = root.read_u32(&format!("{dir}/max_state"))?;
                let current = root.read_u32(&format!("{dir}/cur_state"))?;
                if max == 0 {
                    return None;
                }
                let cur_attr = format!("{dir}/cur_state");
                // States are ordered and small -- typically 0..=9 -- so they
                // are steps, not a percentage pretending to be continuous.
                let labels = (0..=max)
                    .map(|s| match s {
                        0 => "Off".to_string(),
                        s if s == max => format!("{s} (maximum)"),
                        s => s.to_string(),
                    })
                    .collect();
                Some(Knob {
                    id: format!("cooling/{name}"),
                    label: format!("{kind} ({name})"),
                    role: Role::ThermalProfile,
                    domain: Domain::Steps { labels },
                    value: Setting::Step {
                        index: current.min(max) as usize,
                    },
                    writable: root.writable(&cur_attr),
                    provider: "cooling-device".into(),
                    origin: root.path(&cur_attr).display().to_string(),
                })
            })
            .collect()
    }

    fn readings(&self, root: &Root) -> Vec<Reading> {
        // Some ACPI fans report their speed here even though they are not
        // hwmon devices.
        devices(root)
            .into_iter()
            .filter_map(|(name, dir, _)| {
                let attr = format!("{dir}/fan_speed_rpm");
                let rpm = root.read_i64(&attr)?;
                Some(Reading {
                    id: format!("cooling/{name}/rpm"),
                    label: format!("Fan ({name})"),
                    unit: Unit::Rpm,
                    value: rpm.max(0) as f64,
                    critical: None,
                    origin: root.path(&attr).display().to_string(),
                })
            })
            .collect()
    }

    fn set(&self, root: &Root, knob_id: &str, value: &Setting) -> anyhow::Result<()> {
        let name = knob_id
            .strip_prefix("cooling/")
            .ok_or_else(|| anyhow::anyhow!("{knob_id} is not a cooling device"))?;
        let Setting::Step { index } = value else {
            anyhow::bail!("a cooling device takes a step, not {value:?}");
        };
        root.write_verified(&format!("{CLASS}/{name}/cur_state"), &index.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_fans_are_offered_as_fans() {
        assert!(is_fan("Fan"));
        assert!(is_fan("INT3404 Fan"));
        assert!(is_fan("TFN1 Fan"));
        // The eight entries this Zephyrus actually has, and friends.
        assert!(!is_fan("Processor"));
        assert!(!is_fan("LCD"));
        assert!(!is_fan("intel_powerclamp"));
        assert!(!is_fan("TCPU"));
    }
}
