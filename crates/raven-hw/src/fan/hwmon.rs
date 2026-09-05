//! `/sys/class/hwmon/hwmon*`, the kernel's one real fan interface.
//!
//! Everything with a fan the kernel can drive shows up here under the ABI in
//! `Documentation/hwmon/sysfs-interface.rst`: desktop Super-I/O chips through
//! `nct6775` and `it87`, discrete GPUs through `amdgpu` and `nouveau`, Dell
//! laptops through `dell-smm-hwmon`, ThinkPads through `thinkpad_acpi`, Apple
//! through `applesmc`, ASUS boards through `asus-ec-sensors`. One provider,
//! and no vendor named in it.
//!
//! The attributes that matter:
//!
//!   pwm<N>          duty, 0-255, on every driver that has one
//!   pwm<N>_enable   0 = no control (full speed), 1 = us, 2+ = firmware
//!   fan<N>_input    speed in RPM, read-only
//!   temp<N>_input   millidegrees, with temp<N>_crit where published
//!
//! `pwm<N>_enable` is the dangerous one and the reason `raven-controlsd`
//! exists. Writing `pwm<N>` while enable is 2 does nothing at all -- firmware
//! is still driving -- so setting a duty means taking control first, and having
//! taken it, something must be alive to give it back. See `guard.rs`.

use crate::model::*;
use crate::provider::Provider;
use crate::sysfs::Root;

pub const CLASS: &str = "/sys/class/hwmon";

/// hwmon PWM is a byte, on every driver, by ABI.
pub const PWM_RAW_MAX: u32 = 255;

pub struct Hwmon;

/// One hwmon directory, with the name we key its knobs on.
pub struct Chip {
    /// `nct6798`, `amdgpu`, `thinkpad`. From the driver, not from the
    /// enumeration index -- `hwmon3` is whatever probed third this boot, and
    /// keying a saved fan curve on it would apply the CPU curve to the GPU
    /// after an unrelated module load.
    pub key: String,
    pub name: String,
    pub dir: String,
}

pub fn chips(root: &Root) -> Vec<Chip> {
    let mut found: Vec<(String, String, Option<String>)> = Vec::new();
    for (_, dir) in root.list(CLASS) {
        let Some(name) = root.read(&format!("{dir}/name")) else {
            continue;
        };
        // Where two chips share a name -- two identical GPUs -- the bus address
        // behind the `device` symlink separates them, and is as stable as the
        // slot the card is in.
        let address = std::fs::read_link(root.path(&format!("{dir}/device")))
            .ok()
            .and_then(|p| Some(p.file_name()?.to_string_lossy().into_owned()));
        found.push((name, dir, address));
    }
    let mut out = Vec::new();
    for (name, dir, address) in &found {
        let duplicated = found.iter().filter(|(n, _, _)| n == name).count() > 1;
        let key = match (duplicated, address) {
            (true, Some(address)) => format!("{name}@{address}"),
            _ => name.clone(),
        };
        out.push(Chip {
            key,
            name: name.clone(),
            dir: dir.clone(),
        });
    }
    out
}

/// PWM channel numbers: the bare `pwmN` attributes.
///
/// Bare, because `pwm1`, `pwm1_enable` and `pwm1_auto_point1_pwm` all share the
/// prefix and only the first is the duty cycle.
fn pwm_channels(root: &Root, dir: &str) -> Vec<u32> {
    let mut out: Vec<u32> = root
        .list(dir)
        .into_iter()
        .filter_map(|(name, _)| name.strip_prefix("pwm")?.parse::<u32>().ok())
        .collect();
    out.sort_unstable();
    out
}

/// Channel numbers that have a `pwmN_enable`.
///
/// Not the same set as `pwm_channels`. `asus_wmi` publishes `pwm1_enable` and
/// `pwm2_enable` with no duty attribute at all -- the driver can hand a fan
/// between firmware and full speed but cannot set a percentage -- and a laptop
/// like that has real, writable fan control that looking only for `pwmN` walks
/// straight past.
fn enable_channels(root: &Root, dir: &str) -> Vec<u32> {
    let mut out: Vec<u32> = root
        .list(dir)
        .into_iter()
        .filter_map(|(name, _)| {
            name.strip_prefix("pwm")?
                .strip_suffix("_enable")?
                .parse::<u32>()
                .ok()
        })
        .collect();
    out.sort_unstable();
    out
}

/// Fan and temperature channel numbers, from `fanN_input` and `tempN_input`.
///
/// These have no bare attribute at all -- hwmon spells the measurement
/// `fan1_input` -- so they cannot be found the way PWM channels are. Looking
/// for `fanN` on a MacBook finds nothing, which is how applesmc came to report
/// no fans at all until a captured tree said otherwise.
fn input_channels(root: &Root, dir: &str, family: &str) -> Vec<u32> {
    let mut out: Vec<u32> = root
        .list(dir)
        .into_iter()
        .filter_map(|(name, _)| {
            name.strip_prefix(family)?
                .strip_suffix("_input")?
                .parse::<u32>()
                .ok()
        })
        .collect();
    out.sort_unstable();
    out
}

/// hwmon's `pwmN_enable`, in words.
///
/// 0, 1 and 2 are fixed by the ABI. Anything above 2 is a driver's own
/// automatic algorithm -- `nct6775` numbers four of them -- so it is reported
/// rather than renamed, and never silently rewritten to 2.
pub fn enable_label(value: u32) -> String {
    match value {
        0 => "Full speed".into(),
        1 => "Manual".into(),
        2 => "Automatic".into(),
        n => format!("Automatic (driver mode {n})"),
    }
}

pub fn enable_value(label: &str) -> Option<u32> {
    match label {
        "Full speed" => Some(0),
        "Manual" => Some(1),
        "Automatic" => Some(2),
        other => other
            .strip_prefix("Automatic (driver mode ")?
            .strip_suffix(')')?
            .parse()
            .ok(),
    }
}

/// The modes to offer for a channel currently sitting at `current`.
///
/// The kernel publishes no list of the values a driver accepts, so the ABI's
/// own are offered, plus whatever the driver is actually using if it is
/// something else -- which is how a machine keeps its way back to its own
/// algorithm instead of being pushed onto a generic one it may not have.
///
/// `has_duty` is why this takes an argument. "Manual" means "I will set the
/// duty myself", and on a driver with no `pwmN` there is no duty to set: the
/// fan would be taken off firmware control and left at whatever the embedded
/// controller happened to leave in the register, with nothing able to change
/// it. Offering that is offering a way to make a laptop worse.
pub fn enable_modes(current: u32, has_duty: bool) -> Vec<String> {
    let mut modes = vec![enable_label(2)];
    if has_duty {
        modes.push(enable_label(1));
    }
    modes.push(enable_label(0));
    if current > 2 {
        modes.insert(0, enable_label(current));
    }
    // A driver already sitting in manual with no duty attribute is a state we
    // cannot offer but must still display, or the row shows the wrong mode as
    // selected.
    if current == 1 && !has_duty {
        modes.insert(0, enable_label(1));
    }
    modes
}

fn fan_label(root: &Root, dir: &str, n: u32) -> String {
    root.read(&format!("{dir}/fan{n}_label"))
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| format!("Fan {n}"))
}

impl Provider for Hwmon {
    fn id(&self) -> &'static str {
        "hwmon"
    }

    fn knobs(&self, root: &Root) -> Vec<Knob> {
        let mut out = Vec::new();
        for chip in chips(root) {
            let Chip { key, name, dir } = &chip;
            // The union, not the duty channels alone: a driver may publish
            // either without the other. `asus_wmi` has enables and no duties;
            // the reverse turns up on drivers with a fixed automatic curve.
            let duties = pwm_channels(root, dir);
            let enables = enable_channels(root, dir);
            let mut channels: Vec<u32> = duties.iter().chain(&enables).copied().collect();
            channels.sort_unstable();
            channels.dedup();

            let labelled = input_channels(root, dir, "fan");
            for n in channels {
                // Prefer the fan's own label over the PWM number: a driver that
                // says "cpu_fan" should not be presented as "pwm1".
                let label = if labelled.contains(&n) {
                    format!("{} ({name})", fan_label(root, dir, n))
                } else {
                    format!("Fan {n} ({name})")
                };

                let duty_attr = format!("{dir}/pwm{n}");
                let duty = duties
                    .contains(&n)
                    .then(|| root.read_u32(&duty_attr))
                    .flatten();
                if let Some(raw) = duty {
                    out.push(Knob {
                        id: format!("hwmon/{key}/pwm{n}"),
                        label: label.clone(),
                        role: Role::FanDuty,
                        domain: Domain::Percent {
                            raw_max: PWM_RAW_MAX,
                        },
                        value: Setting::Percent {
                            percent: raw_to_percent(raw, PWM_RAW_MAX),
                        },
                        writable: root.writable(&duty_attr),
                        provider: "hwmon".into(),
                        origin: root.path(&duty_attr).display().to_string(),
                    });
                }

                let enable_attr = format!("{dir}/pwm{n}_enable");
                if let Some(enable) = root.read_u32(&enable_attr) {
                    out.push(Knob {
                        id: format!("hwmon/{key}/pwm{n}_enable"),
                        // With no duty knob beside it this row is the whole
                        // control for that fan, so it is named as one.
                        label: if duty.is_some() {
                            format!("{label} control")
                        } else {
                            label.clone()
                        },
                        role: Role::FanControlMode,
                        domain: Domain::Modes {
                            options: enable_modes(enable, duty.is_some()),
                        },
                        value: Setting::Mode {
                            name: enable_label(enable),
                        },
                        writable: root.writable(&enable_attr),
                        provider: "hwmon".into(),
                        origin: root.path(&enable_attr).display().to_string(),
                    });
                }
            }
        }
        out
    }

    fn readings(&self, root: &Root) -> Vec<Reading> {
        let mut out = Vec::new();
        for Chip { key, name, dir } in chips(root) {
            for n in input_channels(root, &dir, "fan") {
                let attr = format!("{dir}/fan{n}_input");
                let Some(rpm) = root.read_i64(&attr) else {
                    continue;
                };
                out.push(Reading {
                    id: format!("hwmon/{key}/fan{n}"),
                    label: format!("{} ({name})", fan_label(root, &dir, n)),
                    unit: Unit::Rpm,
                    // A stopped fan reports 0, and a driver with nothing
                    // attached reports a negative number.
                    value: rpm.max(0) as f64,
                    critical: None,
                    origin: root.path(&attr).display().to_string(),
                });
            }
            for n in input_channels(root, &dir, "temp") {
                let attr = format!("{dir}/temp{n}_input");
                let Some(milli) = root.read_i64(&attr) else {
                    continue;
                };
                // hwmon temperatures are millidegrees Celsius, always.
                let celsius = milli as f64 / 1000.0;
                let critical = root
                    .read_i64(&format!("{dir}/temp{n}_crit"))
                    // Not every driver publishes _crit; _max is the next best
                    // statement of "past here, stop being clever".
                    .or_else(|| root.read_i64(&format!("{dir}/temp{n}_max")))
                    .map(|m| m as f64 / 1000.0)
                    .filter(|c| *c > 0.0);
                out.push(Reading {
                    id: format!("hwmon/{key}/temp{n}"),
                    label: root
                        .read(&format!("{dir}/temp{n}_label"))
                        .filter(|l| !l.is_empty())
                        .map(|l| format!("{l} ({name})"))
                        .unwrap_or_else(|| format!("Temperature {n} ({name})")),
                    unit: Unit::Celsius,
                    value: celsius,
                    critical,
                    origin: root.path(&attr).display().to_string(),
                });
            }
        }
        out
    }

    fn set(&self, root: &Root, knob_id: &str, value: &Setting) -> anyhow::Result<()> {
        let (chip_key, attr) = split_id(knob_id)?;
        let chip = chips(root)
            .into_iter()
            .find(|c| c.key == chip_key)
            .ok_or_else(|| anyhow::anyhow!("{chip_key} is no longer present"))?;
        let path = format!("{}/{attr}", chip.dir);

        match value {
            Setting::Percent { percent } => {
                // Writing a duty while firmware still owns the channel is
                // accepted and ignored. Take control first, or the slider is a
                // lie.
                let enable_attr = format!("{}_enable", path);
                if root.exists(&enable_attr) && root.read_u32(&enable_attr) != Some(1) {
                    root.write_verified(&enable_attr, "1").map_err(|e| {
                        anyhow::anyhow!("could not take manual control of {attr}: {e}")
                    })?;
                }
                root.write_verified(&path, &percent_to_raw(*percent, PWM_RAW_MAX).to_string())
            }
            Setting::Mode { name } => {
                let raw = enable_value(name)
                    .ok_or_else(|| anyhow::anyhow!("{name:?} is not a fan control mode"))?;
                root.write_verified(&path, &raw.to_string())
            }
            other => anyhow::bail!("hwmon cannot take {other:?}"),
        }
    }
}

fn split_id(knob_id: &str) -> anyhow::Result<(String, String)> {
    let rest = knob_id
        .strip_prefix("hwmon/")
        .ok_or_else(|| anyhow::anyhow!("{knob_id} is not an hwmon control"))?;
    // The chip key can itself contain '/'? It cannot: it is a driver name
    // optionally suffixed with a bus address. Split on the last separator.
    let (chip, attr) = rest
        .rsplit_once('/')
        .ok_or_else(|| anyhow::anyhow!("{knob_id} names no attribute"))?;
    Ok((chip.to_string(), attr.to_string()))
}

/// The udev rule that lets the `video` group drive fans without root.
///
/// Shipped, documented, and *not* what RavenControls recommends: a session that
/// can write `pwmN` can also leave a fan pinned at zero and walk away. The
/// daemon exists so that something is always alive to hand control back. This
/// is here for the person who has read that sentence and wants the rule anyway.
pub const UDEV_RULE: &str = r#"ACTION=="add", SUBSYSTEM=="hwmon", RUN+="/bin/sh -c 'chgrp video /sys%p/pwm* 2>/dev/null; chmod g+w /sys%p/pwm* 2>/dev/null'""#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_abi_modes_round_trip() {
        for v in [0, 1, 2, 5] {
            assert_eq!(enable_value(&enable_label(v)), Some(v), "mode {v}");
        }
    }

    #[test]
    fn a_drivers_own_automatic_mode_stays_on_the_menu() {
        // nct6775 sits at 5 (Smart Fan IV). Offering only 0/1/2 would strand a
        // board on a mode it never used.
        let modes = enable_modes(5, true);
        assert_eq!(modes[0], "Automatic (driver mode 5)");
        assert!(modes.contains(&"Manual".to_string()));
        assert!(modes.contains(&"Automatic".to_string()));
    }

    #[test]
    fn the_ordinary_case_offers_exactly_three() {
        assert_eq!(
            enable_modes(2, true),
            vec!["Automatic", "Manual", "Full speed"]
        );
    }

    #[test]
    fn manual_is_not_offered_when_there_is_no_duty_to_set() {
        // asus_wmi's shape, found on the machine this was written on: two
        // pwmN_enable attributes and no pwmN at all. Manual there takes the fan
        // off firmware and leaves it wherever the embedded controller had it,
        // with nothing able to move it again.
        let modes = enable_modes(2, false);
        assert_eq!(modes, vec!["Automatic", "Full speed"]);
    }

    #[test]
    fn a_driver_already_sitting_in_manual_can_still_show_it() {
        // We will not offer the move into manual, but a row whose current value
        // is missing from its own menu displays the wrong mode as selected.
        assert_eq!(enable_modes(1, false)[0], "Manual");
    }

    #[test]
    fn a_bare_pwm_is_a_channel_and_its_neighbours_are_not() {
        // Every attribute in an hwmon directory that starts with "pwm".
        let names = [
            "pwm1",
            "pwm1_enable",
            "pwm1_mode",
            "pwm1_auto_point1_pwm",
            "pwm2",
        ];
        let channels: Vec<u32> = names
            .iter()
            .filter_map(|n| n.strip_prefix("pwm")?.parse::<u32>().ok())
            .collect();
        assert_eq!(channels, vec![1, 2]);
    }

    #[test]
    fn ids_split_back_apart_including_bus_qualified_chips() {
        assert_eq!(
            split_id("hwmon/nct6798/pwm2").unwrap(),
            ("nct6798".into(), "pwm2".into())
        );
        assert_eq!(
            split_id("hwmon/amdgpu@0000:03:00.0/pwm1_enable").unwrap(),
            ("amdgpu@0000:03:00.0".into(), "pwm1_enable".into())
        );
        assert!(split_id("leds/x").is_err());
    }
}
