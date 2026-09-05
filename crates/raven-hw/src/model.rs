//! The vocabulary the rest of RavenControls speaks.
//!
//! The temptation, writing this, is to model the machine in front of you: a
//! brightness level 0-3, a thermal policy of balanced/turbo/silent. Do that and
//! the ThinkPad's eight fan levels, the desktop's 0-255 PWM and the Framework's
//! named platform profiles each need their own page, their own settings schema
//! and their own bugs.
//!
//! So knobs are described, not enumerated. A provider says "I am a control with
//! this domain and this current value"; the window renders a domain, never a
//! vendor. Adding a machine adds a `Provider`, and no UI code at all.

use serde::{Deserialize, Serialize};

/// Where a knob belongs in the window, and what a curve engine may drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Role {
    /// The light under the keys.
    KeyboardBacklight,
    /// A fan's duty cycle. The only role a curve can drive, because it is the
    /// only one with a continuous, monotonic relationship to airflow.
    FanDuty,
    /// Who is in charge of a fan: firmware, or us.
    FanControlMode,
    /// A coarse machine-wide mode -- ACPI platform profile, ThinkPad fan level,
    /// ASUS throttle policy. Changes fan behaviour without exposing duty.
    ThermalProfile,
}

impl Role {
    pub fn heading(self) -> &'static str {
        match self {
            Role::KeyboardBacklight => "Keyboard backlight",
            Role::FanDuty => "Fans",
            Role::FanControlMode => "Fans",
            Role::ThermalProfile => "Thermal profile",
        }
    }
}

/// What a knob will accept. This is the whole trick: four shapes cover every
/// fan and backlight interface the Linux kernel exposes, so the window needs
/// four widgets rather than one per laptop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Domain {
    /// Continuous. Presented as 0-100% regardless of what the hardware counts
    /// in; `raw_max` is how high the attribute itself goes (255 for hwmon PWM,
    /// 3 for this ASUS keyboard, 24 for some Dells).
    Percent { raw_max: u32 },
    /// Ordered discrete steps, lowest first, addressed by index. ThinkPad fan
    /// levels, ACPI cooling-device states.
    Steps { labels: Vec<String> },
    /// Named modes with no useful ordering. ACPI platform profiles, ASUS
    /// throttle policy. Addressed by name, never by index -- the kernel is free
    /// to reorder `platform_profile_choices` and has.
    Modes { options: Vec<String> },
    /// On or off.
    Switch,
    /// Per-channel intensity, from the kernel's multicolour LED class.
    /// `channels` comes from `multi_index`, so it is "red green blue" on most
    /// hardware and something else on the hardware where it is not.
    Color { channels: Vec<String>, raw_max: u32 },
}

/// A knob's current position, in the same terms as its domain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "as", rename_all = "kebab-case")]
pub enum Setting {
    Percent { percent: f64 },
    Step { index: usize },
    Mode { name: String },
    Switch { on: bool },
    Color { intensities: Vec<u32> },
}

impl Setting {
    /// Reject a value the hardware cannot take, before it reaches a write.
    /// Out-of-range writes to sysfs return `-EINVAL`, which surfaces as an
    /// unhelpful "Invalid argument" three layers up; catching it here means the
    /// window can say what was actually wrong.
    pub fn check(&self, domain: &Domain) -> anyhow::Result<()> {
        match (self, domain) {
            (Setting::Percent { percent }, Domain::Percent { .. }) => {
                if !percent.is_finite() || !(0.0..=100.0).contains(percent) {
                    anyhow::bail!("{percent} is not a percentage between 0 and 100");
                }
                Ok(())
            }
            (Setting::Step { index }, Domain::Steps { labels }) => {
                if *index >= labels.len() {
                    anyhow::bail!("step {index} of {} steps", labels.len());
                }
                Ok(())
            }
            (Setting::Mode { name }, Domain::Modes { options }) => {
                if !options.iter().any(|o| o == name) {
                    anyhow::bail!("{name:?} is not one of: {}", options.join(", "));
                }
                Ok(())
            }
            (Setting::Switch { .. }, Domain::Switch) => Ok(()),
            (Setting::Color { intensities }, Domain::Color { channels, raw_max }) => {
                if intensities.len() != channels.len() {
                    anyhow::bail!(
                        "{} intensities for {} channels ({})",
                        intensities.len(),
                        channels.len(),
                        channels.join(", ")
                    );
                }
                if let Some(bad) = intensities.iter().find(|i| *i > raw_max) {
                    anyhow::bail!("intensity {bad} is above the hardware maximum of {raw_max}");
                }
                Ok(())
            }
            _ => anyhow::bail!("{self:?} does not fit a {domain:?} control"),
        }
    }
}

/// Converting a percentage to the hardware's own units, and back.
///
/// Rounding to nearest, not truncating: on this Zephyrus `raw_max` is 3, so
/// truncation would make 74% dark and only 100% reach the top step. Nearest
/// puts the four steps at the quarters, where a person dragging a slider
/// expects them.
pub fn percent_to_raw(percent: f64, raw_max: u32) -> u32 {
    let clamped = percent.clamp(0.0, 100.0);
    ((clamped / 100.0) * raw_max as f64).round() as u32
}

pub fn raw_to_percent(raw: u32, raw_max: u32) -> f64 {
    if raw_max == 0 {
        return 0.0;
    }
    (raw.min(raw_max) as f64) * 100.0 / raw_max as f64
}

/// One control, as discovered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Knob {
    /// Stable across reboots and across kernels where we can manage it, because
    /// saved settings and fan curves are keyed on it. Built from the driver's
    /// own name rather than an enumeration index: `hwmon/nct6798/pwm2`, not
    /// `hwmon3/pwm2`, since hwmon numbering shuffles between boots.
    pub id: String,
    pub label: String,
    pub role: Role,
    pub domain: Domain,
    pub value: Setting,
    /// False when the attribute exists but this account cannot write it. The
    /// window still shows the control, greyed, with the reason -- a missing
    /// keyboard-light slider is a bug report, a disabled one with "needs the
    /// video group" is an answer.
    pub writable: bool,
    /// Which provider found it, for `--probe` output and bug reports.
    pub provider: String,
    /// The kernel path or bus address behind it. Shown in `--probe`.
    pub origin: String,
}

/// Something to read: a fan's speed, a temperature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub id: String,
    pub label: String,
    pub unit: Unit,
    pub value: f64,
    /// The temperature the driver calls critical, where one is published. The
    /// curve engine overrides everything and goes to full speed here.
    pub critical: Option<f64>,
    pub origin: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unit {
    Rpm,
    Celsius,
}

impl Unit {
    pub fn format(self, value: f64) -> String {
        match self {
            Unit::Rpm => format!("{} RPM", value.round() as i64),
            Unit::Celsius => format!("{:.0} °C", value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentages_land_on_the_nearest_step_of_a_coarse_control() {
        // The four steps of this laptop's keyboard light.
        assert_eq!(percent_to_raw(0.0, 3), 0);
        assert_eq!(percent_to_raw(33.0, 3), 1);
        assert_eq!(percent_to_raw(74.0, 3), 2);
        assert_eq!(percent_to_raw(100.0, 3), 3);
        // And back.
        assert!((raw_to_percent(3, 3) - 100.0).abs() < f64::EPSILON);
        assert!((raw_to_percent(0, 3)).abs() < f64::EPSILON);
    }

    #[test]
    fn a_round_trip_through_a_255_step_pwm_keeps_the_percentage() {
        for p in [0.0, 25.0, 50.0, 100.0] {
            let back = raw_to_percent(percent_to_raw(p, 255), 255);
            assert!((back - p).abs() < 0.5, "{p} came back as {back}");
        }
    }

    #[test]
    fn a_mode_is_checked_against_what_the_kernel_offered() {
        let domain = Domain::Modes {
            options: vec!["quiet".into(), "balanced".into()],
        };
        assert!(Setting::Mode {
            name: "quiet".into()
        }
        .check(&domain)
        .is_ok());
        let err = Setting::Mode {
            name: "turbo".into(),
        }
        .check(&domain)
        .unwrap_err()
        .to_string();
        assert!(err.contains("quiet, balanced"), "unhelpful message: {err}");
    }

    #[test]
    fn colour_intensities_must_match_the_channel_count() {
        let domain = Domain::Color {
            channels: vec!["red".into(), "green".into(), "blue".into()],
            raw_max: 255,
        };
        assert!(Setting::Color {
            intensities: vec![255, 0, 0]
        }
        .check(&domain)
        .is_ok());
        assert!(Setting::Color {
            intensities: vec![255, 0]
        }
        .check(&domain)
        .is_err());
        assert!(Setting::Color {
            intensities: vec![300, 0, 0]
        }
        .check(&domain)
        .is_err());
    }
}
