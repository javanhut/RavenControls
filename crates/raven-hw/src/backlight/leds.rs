//! The Linux LED class: `/sys/class/leds/*`.
//!
//! This is the one that makes RavenControls portable. Every laptop keyboard
//! light with a kernel driver lands here, under the naming scheme in
//! `Documentation/leds/leds-class.rst` -- `devicename:colour:function`, with
//! the function `kbd_backlight`. asus, dell, tpacpi, smc (Apple), hp, msi,
//! samsung, system76_acpi, chromeos and platform all spell it the same way,
//! which is why this file contains no vendor names.
//!
//! Two kinds of device turn up:
//!
//!   * single-channel, with `brightness` and `max_brightness`. `max_brightness`
//!     is 1 on a light that is only on or off, 3 on this Zephyrus, 255 on some
//!     Dells. The domain follows it rather than assuming any of the three.
//!   * multicolour, which adds `multi_index` and `multi_intensity` from the
//!     kernel's multicolour LED framework. Those become a separate colour knob
//!     alongside the master brightness, because that is exactly how the kernel
//!     models them: intensity sets the mix, brightness scales the whole thing.
//!
//! Per-key RGB is deliberately not here. It has no kernel interface and no
//! standard -- every vendor invented its own HID report layout -- so it belongs
//! in the declarative device profiles, not in code that claims to be generic.

use crate::model::*;
use crate::provider::Provider;
use crate::sysfs::Root;

pub const CLASS: &str = "/sys/class/leds";

pub struct Leds;

/// The kernel's LED names are `devicename:colour:function`. Match on the
/// function alone, so a light called `tpacpi::kbd_backlight` and one called
/// `rgb:kbd_backlight` are both recognised without either being listed.
fn is_keyboard_backlight(name: &str) -> bool {
    match name.rsplit(':').next() {
        Some(function) => function.eq_ignore_ascii_case("kbd_backlight"),
        None => false,
    }
}

/// `asus::kbd_backlight` and `tpacpi::kbd_backlight` are both just "Keyboard
/// backlight" to the person reading the window; the driver name is kept in
/// `origin` for the probe output. Where a machine really has two of them, the
/// device part disambiguates.
fn label_for(name: &str, multiple: bool) -> String {
    if !multiple {
        return "Keyboard backlight".into();
    }
    let device = name.split(':').next().unwrap_or(name);
    if device.is_empty() {
        "Keyboard backlight".into()
    } else {
        format!("Keyboard backlight ({device})")
    }
}

impl Provider for Leds {
    fn id(&self) -> &'static str {
        "leds"
    }

    fn knobs(&self, root: &Root) -> Vec<Knob> {
        let names: Vec<(String, String)> = root
            .list(CLASS)
            .into_iter()
            .filter(|(name, _)| is_keyboard_backlight(name))
            .collect();
        let multiple = names.len() > 1;
        let mut out = Vec::new();

        for (name, dir) in names {
            let brightness_attr = format!("{dir}/brightness");
            let Some(max) = root.read_u32(&format!("{dir}/max_brightness")) else {
                // A LED with no maximum is a driver mid-teardown. Skip it
                // rather than publishing a control with no range.
                continue;
            };
            let Some(current) = root.read_u32(&brightness_attr) else {
                continue;
            };
            let writable = root.writable(&brightness_attr);
            let label = label_for(&name, multiple);

            // A light with two states is a switch, not a slider that happens to
            // have two positions. The distinction is the difference between a
            // toggle and a scale a person can leave stranded at 50%.
            let (domain, value) = if max <= 1 {
                (Domain::Switch, Setting::Switch { on: current >= 1 })
            } else {
                (
                    Domain::Percent { raw_max: max },
                    Setting::Percent {
                        percent: raw_to_percent(current, max),
                    },
                )
            };

            out.push(Knob {
                id: format!("leds/{name}"),
                label: label.clone(),
                role: Role::KeyboardBacklight,
                domain,
                value,
                writable,
                provider: "leds".into(),
                origin: root.path(&brightness_attr).display().to_string(),
            });

            // The multicolour framework, where the driver implements it.
            let index_attr = format!("{dir}/multi_index");
            let intensity_attr = format!("{dir}/multi_intensity");
            if let (Some(index), Some(intensity)) =
                (root.read(&index_attr), root.read_u32_list(&intensity_attr))
            {
                let channels: Vec<String> =
                    index.split_whitespace().map(|s| s.to_string()).collect();
                if !channels.is_empty() && channels.len() == intensity.len() {
                    out.push(Knob {
                        id: format!("leds/{name}/color"),
                        label: format!("{label} colour"),
                        role: Role::KeyboardBacklight,
                        // The kernel documents intensities as sharing the LED's
                        // `max_brightness` scale.
                        domain: Domain::Color {
                            channels,
                            raw_max: max,
                        },
                        value: Setting::Color {
                            intensities: intensity,
                        },
                        writable: root.writable(&intensity_attr),
                        provider: "leds".into(),
                        origin: root.path(&intensity_attr).display().to_string(),
                    });
                }
            }
        }
        out
    }

    fn set(&self, root: &Root, knob_id: &str, value: &Setting) -> anyhow::Result<()> {
        let rest = knob_id
            .strip_prefix("leds/")
            .ok_or_else(|| anyhow::anyhow!("{knob_id} is not an LED"))?;
        let (name, attr) = match rest.strip_suffix("/color") {
            Some(name) => (name, "multi_intensity"),
            None => (rest, "brightness"),
        };
        let path = format!("{CLASS}/{name}/{attr}");

        let text = match value {
            Setting::Percent { percent } => {
                let max = root
                    .read_u32(&format!("{CLASS}/{name}/max_brightness"))
                    .ok_or_else(|| anyhow::anyhow!("{name} disappeared while being set"))?;
                percent_to_raw(*percent, max).to_string()
            }
            Setting::Switch { on } => u32::from(*on).to_string(),
            Setting::Color { intensities } => intensities
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(" "),
            other => anyhow::bail!("an LED cannot take {other:?}"),
        };

        // A `trigger` other than `none` means something else -- a timer, a
        // heartbeat, the capslock key -- owns this LED and will overwrite
        // whatever we write within the second. Take ownership first.
        let trigger_attr = format!("{CLASS}/{name}/trigger");
        if let Some(trigger) = root.read(&trigger_attr) {
            // The attribute reads as the full menu with the active one in
            // brackets: "none rfkill0 [timer] heartbeat".
            let active = trigger
                .split_whitespace()
                .find(|t| t.starts_with('['))
                .map(|t| t.trim_matches(['[', ']']));
            if matches!(active, Some(a) if a != "none") && root.writable(&trigger_attr) {
                let _ = root.write(&trigger_attr, "none");
            }
        }

        root.write_verified(&path, &text)
    }
}

/// The udev rule that lets the session set the keyboard light without root.
///
/// Matched on the LED's function rather than a device name, so it covers the
/// same set of machines this provider does. `video` is the group the Raven
/// session already holds for DRM, so no new group and no new membership.
pub const UDEV_RULE: &str = r#"ACTION=="add", SUBSYSTEM=="leds", KERNEL=="*kbd_backlight", RUN+="/bin/chgrp video /sys/class/leds/%k/brightness", RUN+="/bin/chmod g+w /sys/class/leds/%k/brightness""#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_vendors_spelling_of_the_function_is_recognised() {
        for name in [
            "asus::kbd_backlight",
            "dell::kbd_backlight",
            "tpacpi::kbd_backlight",
            "smc::kbd_backlight",
            "chromeos::kbd_backlight",
            "platform::kbd_backlight",
            "system76_acpi::kbd_backlight",
            "hp::kbd_backlight",
            "rgb:kbd_backlight",
        ] {
            assert!(is_keyboard_backlight(name), "missed {name}");
        }
    }

    #[test]
    fn other_leds_on_the_same_machine_are_left_alone() {
        for name in [
            "input21::capslock",
            "input21::numlock",
            "rtw88-0000:03:00.0",
            "phy0-led",
            // The screen backlight is a different class entirely, but a
            // keyboard-backlight matcher that caught it would be a disaster.
            "intel_backlight",
        ] {
            assert!(!is_keyboard_backlight(name), "wrongly claimed {name}");
        }
    }

    #[test]
    fn a_single_light_is_not_labelled_with_its_driver() {
        assert_eq!(
            label_for("asus::kbd_backlight", false),
            "Keyboard backlight"
        );
        assert_eq!(
            label_for("asus::kbd_backlight", true),
            "Keyboard backlight (asus)"
        );
    }
}
