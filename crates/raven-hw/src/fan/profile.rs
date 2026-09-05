//! The ACPI platform profile: `low-power`, `balanced`, `performance` and the
//! rest, from `Documentation/ABI/testing/sysfs-platform_profile`.
//!
//! This is the second most portable thing in RavenControls and, on a modern
//! laptop, usually the *only* thing. It is vendor-neutral by construction --
//! ASUS, Lenovo, HP, Dell, Framework, MSI and Microsoft Surface all register
//! with the same core -- and it is what this Zephyrus would expose if
//! `asus_nb_wmi` were loaded. It does not give you a duty cycle or an RPM. It
//! gives you the firmware's own fan behaviour, chosen from a list, and for most
//! laptops that is the honest ceiling.
//!
//! Two layouts, because the kernel grew a second one in 6.14 when a machine
//! turned out to be able to have more than one handler:
//!
//!   /sys/firmware/acpi/platform_profile{,_choices}      the original
//!   /sys/class/platform-profile/platform-profile-N/     the multi-handler one

use crate::model::*;
use crate::provider::Provider;
use crate::sysfs::Root;

const LEGACY: &str = "/sys/firmware/acpi/platform_profile";
const LEGACY_CHOICES: &str = "/sys/firmware/acpi/platform_profile_choices";
const CLASS: &str = "/sys/class/platform-profile";

pub struct PlatformProfile;

/// `low-power` is what the kernel calls it; "Low power" is what a person calls
/// it. The mapping is one-way on purpose -- the kernel name is what gets
/// written, and is kept as the `Setting`.
fn pretty(name: &str) -> String {
    let mut c = name.replace('-', " ");
    if let Some(first) = c.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    c
}

struct Handler {
    id: String,
    label: String,
    profile_attr: String,
    choices_attr: String,
}

fn handlers(root: &Root) -> Vec<Handler> {
    // The class directory, where the kernel is new enough to have one.
    let mut out: Vec<Handler> = root
        .list(CLASS)
        .into_iter()
        .filter(|(name, _)| name.starts_with("platform-profile-"))
        .map(|(name, dir)| {
            let driver = root.read(&format!("{dir}/name")).unwrap_or_default();
            Handler {
                id: format!("platform-profile/{name}"),
                label: if driver.is_empty() {
                    "Platform profile".into()
                } else {
                    format!("Platform profile ({driver})")
                },
                profile_attr: format!("{dir}/profile"),
                choices_attr: format!("{dir}/choices"),
            }
        })
        .collect();

    // The original path. On a kernel that has both, these are the same handler
    // seen twice, and the legacy file is the compatibility alias -- so it is
    // only used when the class turned up nothing.
    if out.is_empty() && root.exists(LEGACY) {
        out.push(Handler {
            id: "platform-profile/acpi".into(),
            label: "Platform profile".into(),
            profile_attr: LEGACY.into(),
            choices_attr: LEGACY_CHOICES.into(),
        });
    }
    out
}

impl Provider for PlatformProfile {
    fn id(&self) -> &'static str {
        "platform-profile"
    }

    fn knobs(&self, root: &Root) -> Vec<Knob> {
        handlers(root)
            .into_iter()
            .filter_map(|h| {
                let choices = root.read(&h.choices_attr)?;
                let options: Vec<String> = choices.split_whitespace().map(pretty).collect();
                if options.is_empty() {
                    return None;
                }
                let current = root.read(&h.profile_attr)?;
                Some(Knob {
                    id: h.id,
                    label: h.label,
                    role: Role::ThermalProfile,
                    domain: Domain::Modes { options },
                    value: Setting::Mode {
                        name: pretty(&current),
                    },
                    writable: root.writable(&h.profile_attr),
                    provider: "platform-profile".into(),
                    origin: root.path(&h.profile_attr).display().to_string(),
                })
            })
            .collect()
    }

    fn set(&self, root: &Root, knob_id: &str, value: &Setting) -> anyhow::Result<()> {
        let handler = handlers(root)
            .into_iter()
            .find(|h| h.id == knob_id)
            .ok_or_else(|| anyhow::anyhow!("{knob_id} is no longer present"))?;
        let Setting::Mode { name } = value else {
            anyhow::bail!("a platform profile takes a mode, not {value:?}");
        };
        // Back to the kernel's spelling: match against the published choices
        // rather than un-prettifying, so a choice with an unexpected shape
        // still round-trips.
        let choices = root
            .read(&handler.choices_attr)
            .ok_or_else(|| anyhow::anyhow!("the driver stopped publishing its choices"))?;
        let kernel_name = choices
            .split_whitespace()
            .find(|c| pretty(c) == *name)
            .ok_or_else(|| anyhow::anyhow!("{name:?} is not offered by this firmware"))?;
        root.write_verified(&handler.profile_attr, kernel_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_names_become_readable_without_losing_the_original() {
        assert_eq!(pretty("low-power"), "Low power");
        assert_eq!(pretty("balanced"), "Balanced");
        assert_eq!(pretty("performance"), "Performance");
        assert_eq!(pretty("quiet"), "Quiet");
        assert_eq!(pretty("balanced-performance"), "Balanced performance");
    }
}
