//! Documented vendor sysfs knobs, expressed as data.
//!
//! Some laptops publish a fan control that is neither hwmon nor a platform
//! profile: ASUS has `throttle_thermal_policy` and `fan_boost_mode`, ThinkPads
//! have `/proc/acpi/ibm/fan`. These are *kernel* interfaces with entries in
//! `Documentation/ABI/testing/` -- not vendor SDKs, not reverse-engineered
//! blobs -- so RavenControls can drive them without knowing anything
//! proprietary.
//!
//! They are still machine-specific, and that is the point of this file: they
//! live in a table, so supporting one more laptop is a row rather than a
//! provider, and nothing above this line ever learns a brand name.
//!
//! Note what the table does *not* key on: the platform device's directory name.
//! Kernel drivers get renamed -- `asus-nb-wmi` has been `asus_wmi` and
//! `asus-wmi` in living memory -- so an attribute is found by looking for the
//! attribute across every platform device. A machine that exposes
//! `throttle_thermal_policy` gets a throttle control whatever the driver ended
//! up being called.

use crate::model::*;
use crate::provider::Provider;
use crate::sysfs::Root;

const PLATFORM: &str = "/sys/devices/platform";

/// One documented attribute, and what its integers mean.
struct VendorAttr {
    /// The attribute's filename, searched for across platform devices.
    file: &'static str,
    label: &'static str,
    /// `(value written to sysfs, name shown)`, in the order to offer them.
    /// Ordered coolest-first, which is the order these read best in a list.
    modes: &'static [(u32, &'static str)],
    /// Where the ABI is written down, for the probe output and for anyone
    /// wondering whether this was guessed.
    reference: &'static str,
}

/// Adding a laptop means adding a row here.
const ATTRS: &[VendorAttr] = &[
    VendorAttr {
        file: "throttle_thermal_policy",
        label: "Throttle policy",
        modes: &[(2, "Silent"), (0, "Balanced"), (1, "Turbo")],
        reference: "Documentation/ABI/testing/sysfs-platform-asus-wmi",
    },
    VendorAttr {
        file: "fan_boost_mode",
        label: "Fan boost",
        modes: &[(2, "Silent"), (0, "Normal"), (1, "Overboost")],
        reference: "Documentation/ABI/testing/sysfs-platform-asus-wmi",
    },
];

/// ThinkPad's fan lives in procfs with a format of its own, so it gets a few
/// lines rather than a table row.
const THINKPAD_FAN: &str = "/proc/acpi/ibm/fan";

pub struct VendorKnobs;

/// Every platform device exposing `file`, as `(device name, attribute path)`.
fn find_attr(root: &Root, file: &str) -> Vec<(String, String)> {
    root.list(PLATFORM)
        .into_iter()
        .filter_map(|(name, dir)| {
            let path = format!("{dir}/{file}");
            root.exists(&path).then_some((name, path))
        })
        .collect()
}

/// `level:` from `/proc/acpi/ibm/fan`, which is `auto`, `full-speed`,
/// `disengaged`, or a digit 0-7.
fn thinkpad_level(text: &str) -> Option<String> {
    text.lines()
        .find_map(|l| l.split_once(':'))
        .filter(|(k, _)| k.trim() == "level")
        .map(|(_, v)| v.trim().to_string())
        .or_else(|| {
            text.lines()
                .filter_map(|l| l.split_once(':'))
                .find(|(k, _)| k.trim() == "level")
                .map(|(_, v)| v.trim().to_string())
        })
}

fn thinkpad_modes() -> Vec<String> {
    let mut m = vec!["Automatic".to_string()];
    m.extend((0..=7).map(|n| format!("Level {n}")));
    m.push("Full speed".to_string());
    m
}

fn thinkpad_label(level: &str) -> String {
    match level {
        "auto" => "Automatic".into(),
        "full-speed" | "disengaged" => "Full speed".into(),
        n => match n.parse::<u32>() {
            Ok(n) if n <= 7 => format!("Level {n}"),
            _ => "Automatic".into(),
        },
    }
}

fn thinkpad_command(label: &str) -> Option<String> {
    match label {
        "Automatic" => Some("level auto".into()),
        "Full speed" => Some("level disengaged".into()),
        other => {
            let n: u32 = other.strip_prefix("Level ")?.parse().ok()?;
            (n <= 7).then(|| format!("level {n}"))
        }
    }
}

impl Provider for VendorKnobs {
    fn id(&self) -> &'static str {
        "vendor"
    }

    fn knobs(&self, root: &Root) -> Vec<Knob> {
        let mut out = Vec::new();

        for attr in ATTRS {
            for (device, path) in find_attr(root, attr.file) {
                let Some(current) = root.read_u32(&path) else {
                    continue;
                };
                let options: Vec<String> = attr
                    .modes
                    .iter()
                    .map(|(_, name)| name.to_string())
                    .collect();
                // A firmware sitting on a value the table does not know is a
                // firmware we should not silently move. Say so instead.
                let name = attr
                    .modes
                    .iter()
                    .find(|(v, _)| *v == current)
                    .map(|(_, n)| n.to_string());
                let Some(name) = name else {
                    tracing::debug!(
                        "{} reports mode {current}, which {} does not list; leaving it alone",
                        path,
                        attr.reference
                    );
                    continue;
                };
                out.push(Knob {
                    id: format!("vendor/{device}/{}", attr.file),
                    label: attr.label.into(),
                    role: Role::ThermalProfile,
                    domain: Domain::Modes { options },
                    value: Setting::Mode { name },
                    writable: root.writable(&path),
                    provider: "vendor".into(),
                    origin: format!("{} ({})", root.path(&path).display(), attr.reference),
                });
            }
        }

        if let Some(text) = root.read(THINKPAD_FAN) {
            if let Some(level) = thinkpad_level(&text) {
                out.push(Knob {
                    id: "vendor/thinkpad/fan".into(),
                    label: "Fan level".into(),
                    role: Role::ThermalProfile,
                    domain: Domain::Modes {
                        options: thinkpad_modes(),
                    },
                    value: Setting::Mode {
                        name: thinkpad_label(&level),
                    },
                    writable: root.writable(THINKPAD_FAN),
                    provider: "vendor".into(),
                    origin: format!(
                        "{} (thinkpad-acpi; needs fan_control=1)",
                        root.path(THINKPAD_FAN).display()
                    ),
                });
            }
        }

        out
    }

    fn readings(&self, root: &Root) -> Vec<Reading> {
        // ThinkPads report fan RPM in the same file, on machines whose
        // thinkpad_acpi has no hwmon node.
        let Some(text) = root.read(THINKPAD_FAN) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.trim() == "speed")
            .and_then(|(_, v)| v.trim().parse::<f64>().ok())
            .map(|rpm| {
                vec![Reading {
                    id: "vendor/thinkpad/fan".into(),
                    label: "Fan".into(),
                    unit: Unit::Rpm,
                    value: rpm,
                    critical: None,
                    origin: root.path(THINKPAD_FAN).display().to_string(),
                }]
            })
            .unwrap_or_default()
    }

    fn set(&self, root: &Root, knob_id: &str, value: &Setting) -> anyhow::Result<()> {
        let Setting::Mode { name } = value else {
            anyhow::bail!("this control takes a mode, not {value:?}");
        };

        if knob_id == "vendor/thinkpad/fan" {
            let command = thinkpad_command(name)
                .ok_or_else(|| anyhow::anyhow!("{name:?} is not a ThinkPad fan level"))?;
            // procfs, so no read-back to verify against.
            return root.write(THINKPAD_FAN, &command).map_err(|e| {
                anyhow::anyhow!(
                    "{THINKPAD_FAN}: {e}. thinkpad_acpi refuses fan writes unless it \
                     was loaded with fan_control=1."
                )
            });
        }

        let rest = knob_id
            .strip_prefix("vendor/")
            .ok_or_else(|| anyhow::anyhow!("{knob_id} is not a vendor control"))?;
        let (device, file) = rest
            .rsplit_once('/')
            .ok_or_else(|| anyhow::anyhow!("{knob_id} names no attribute"))?;
        let attr = ATTRS
            .iter()
            .find(|a| a.file == file)
            .ok_or_else(|| anyhow::anyhow!("{file} is not a control RavenControls knows"))?;
        let raw = attr
            .modes
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(v, _)| *v)
            .ok_or_else(|| anyhow::anyhow!("{name:?} is not a {} mode", attr.label))?;
        root.write_verified(&format!("{PLATFORM}/{device}/{file}"), &raw.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "status:\t\tenabled\nspeed:\t\t2500\nlevel:\t\tauto\n";

    #[test]
    fn the_thinkpad_procfs_format_parses() {
        assert_eq!(thinkpad_level(SAMPLE).as_deref(), Some("auto"));
        assert_eq!(
            thinkpad_level("status:\t\tenabled\nspeed:\t\t0\nlevel:\t\t3\n").as_deref(),
            Some("3")
        );
    }

    #[test]
    fn thinkpad_levels_round_trip_to_the_commands_the_driver_takes() {
        assert_eq!(thinkpad_command("Automatic").as_deref(), Some("level auto"));
        assert_eq!(thinkpad_command("Level 7").as_deref(), Some("level 7"));
        assert_eq!(
            thinkpad_command("Full speed").as_deref(),
            Some("level disengaged")
        );
        // There is no level 8, and asking for one must not become "level 8".
        assert_eq!(thinkpad_command("Level 8"), None);
        assert_eq!(thinkpad_command("Turbo"), None);
    }

    #[test]
    fn a_reported_level_maps_back_to_a_menu_entry() {
        for level in ["auto", "0", "7", "full-speed", "disengaged"] {
            let label = thinkpad_label(level);
            assert!(
                thinkpad_modes().contains(&label),
                "{level} produced {label}, which is not on the menu"
            );
        }
    }

    #[test]
    fn vendor_modes_are_listed_coolest_first() {
        for attr in ATTRS {
            let first = attr.modes[0].1;
            assert!(
                first == "Silent" || first == "Quiet",
                "{} leads with {first}",
                attr.file
            );
        }
    }
}
