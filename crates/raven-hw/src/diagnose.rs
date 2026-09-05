//! Why this machine has no fan control, and what would give it some.
//!
//! This file is the difference between RavenControls working "on any laptop"
//! and merely *claiming* to. Discovery finding nothing is the common case on
//! hardware nobody has tried, and an empty page that says "not supported" tells
//! its owner nothing about whether the machine cannot do it or whether a module
//! is simply not loaded.
//!
//! This Zephyrus is the worked example. It has an `asus::kbd_backlight`, no
//! `platform_profile`, and no PWM anywhere -- not because the hardware lacks
//! them but because `asus_nb_wmi` is built and not loaded, and because
//! `CONFIG_SENSORS_ASUS_EC` was never turned on. Both are answerable, and
//! neither is guessable from an empty window.
//!
//! Nothing here is a fix. It reads `/proc/modules` and the kernel config and
//! reports; loading a module is the owner's decision, and the line to type is
//! printed rather than run.

use crate::sysfs::Root;

/// A driver that would expose fan or thermal control on some machines.
struct Candidate {
    module: &'static str,
    /// Matched case-insensitively against the DMI system vendor. Empty means
    /// "any machine" -- the Super-I/O chips on desktop boards are not tied to
    /// the brand on the case.
    vendor: &'static str,
    gives: &'static str,
    /// The kernel config symbol, so a kernel that never built it can be told
    /// apart from one that merely has not loaded it.
    config: &'static str,
}

const CANDIDATES: &[Candidate] = &[
    Candidate {
        module: "asus_nb_wmi",
        vendor: "asus",
        // Which of these a given model actually publishes varies -- this
        // Zephyrus got the first two and per-fan modes, but no fan_boost_mode.
        // The wording promises the driver, not a fixed set of files.
        gives: "platform profile, throttle policy and fan modes",
        config: "CONFIG_ASUS_NB_WMI",
    },
    Candidate {
        module: "asus_ec_sensors",
        vendor: "asus",
        gives: "fan speeds and temperatures",
        config: "CONFIG_SENSORS_ASUS_EC",
    },
    Candidate {
        module: "asus_wmi_sensors",
        vendor: "asus",
        gives: "fan speeds on ASUS boards",
        config: "CONFIG_SENSORS_ASUS_WMI",
    },
    Candidate {
        module: "thinkpad_acpi",
        vendor: "lenovo",
        gives: "fan levels and speed (load it with fan_control=1 to set them)",
        config: "CONFIG_THINKPAD_ACPI",
    },
    Candidate {
        module: "ideapad_laptop",
        vendor: "lenovo",
        gives: "platform profile",
        config: "CONFIG_IDEAPAD_LAPTOP",
    },
    Candidate {
        module: "dell_smm_hwmon",
        vendor: "dell",
        gives: "fan speeds and PWM control",
        config: "CONFIG_SENSORS_DELL_SMM",
    },
    Candidate {
        module: "hp_wmi",
        vendor: "hp",
        gives: "platform profile",
        config: "CONFIG_HP_WMI",
    },
    Candidate {
        module: "applesmc",
        vendor: "apple",
        gives: "fan speeds and minimum-speed control",
        config: "CONFIG_SENSORS_APPLESMC",
    },
    Candidate {
        module: "msi_ec",
        vendor: "micro-star",
        gives: "platform profile and fan modes",
        config: "CONFIG_MSI_EC",
    },
    Candidate {
        module: "acer_wmi",
        vendor: "acer",
        gives: "platform profile",
        config: "CONFIG_ACER_WMI",
    },
    Candidate {
        module: "gigabyte_wmi",
        vendor: "gigabyte",
        gives: "board temperatures",
        config: "CONFIG_GIGABYTE_WMI",
    },
    Candidate {
        module: "framework_laptop",
        vendor: "framework",
        gives: "fan speeds and thermal control",
        config: "CONFIG_FRAMEWORK_LAPTOP",
    },
    Candidate {
        module: "nct6775",
        vendor: "",
        gives: "PWM fan control on most desktop boards",
        config: "CONFIG_SENSORS_NCT6775",
    },
    Candidate {
        module: "it87",
        vendor: "",
        gives: "PWM fan control on ITE Super-I/O boards",
        config: "CONFIG_SENSORS_IT87",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Built as a module and not loaded. `modprobe` away.
    NotLoaded,
    /// Loaded already, and still gave us nothing -- so the hardware, not the
    /// software, is the limit. Worth saying: it stops someone chasing it.
    Loaded,
    /// The running kernel never built it.
    NotBuilt,
    /// No kernel config to read, so we genuinely do not know.
    Unknown,
}

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub module: String,
    pub gives: String,
    pub availability: Availability,
    pub config: String,
}

impl Suggestion {
    /// One line, as it appears in the window and in `--probe`.
    pub fn line(&self) -> String {
        match self.availability {
            Availability::NotLoaded => format!(
                "{} would add {} — it is built for this kernel but not loaded:\n    sudo modprobe {}",
                self.module, self.gives, self.module
            ),
            Availability::NotBuilt => format!(
                "{} would add {}, but this kernel was built without {}.",
                self.module, self.gives, self.config
            ),
            Availability::Loaded => format!(
                "{} is loaded and exposes nothing here, so this machine's firmware does not offer {}.",
                self.module, self.gives
            ),
            Availability::Unknown => format!(
                "{} might add {}; this kernel's configuration is not readable, so try:\n    sudo modprobe {}",
                self.module, self.gives, self.module
            ),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Report {
    pub vendor: String,
    pub product: String,
    pub kernel: String,
    pub suggestions: Vec<Suggestion>,
}

pub fn vendor(root: &Root) -> String {
    root.read("/sys/class/dmi/id/sys_vendor")
        .unwrap_or_default()
}

pub fn product(root: &Root) -> String {
    root.read("/sys/class/dmi/id/product_name")
        .unwrap_or_default()
}

/// Module names in `/proc/modules` use underscores, and `modprobe` accepts
/// either, so everything is normalised to underscores before comparing.
fn loaded_modules(root: &Root) -> Vec<String> {
    root.read("/proc/modules")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .map(|m| m.replace('-', "_"))
        .collect()
}

/// The running kernel's configuration.
///
/// Three places, because distributions disagree: `/proc/config.gz` on anything
/// built with `CONFIG_IKCONFIG_PROC` (Arch, and so Raven), `/boot/config-*` on
/// Debian and Fedora, and `/lib/modules/*/config` on a few others. Without it
/// every suggestion degrades from "this is built and not loaded, type this" to
/// "it might help", which is most of the value gone.
fn kernel_config(root: &Root) -> Option<String> {
    let release = root.read("/proc/sys/kernel/osrelease").unwrap_or_default();

    if let Some(text) = read_gzip(root, "/proc/config.gz") {
        if text.contains("CONFIG_") {
            return Some(text);
        }
    }
    for path in [
        format!("/boot/config-{release}"),
        "/boot/config".to_string(),
        format!("/lib/modules/{release}/config"),
    ] {
        if let Some(text) = root.read(&path) {
            if text.contains("CONFIG_") {
                return Some(text);
            }
        }
    }
    None
}

fn read_gzip(root: &Root, absolute: &str) -> Option<String> {
    use std::io::Read as _;
    // Not `Root::read`: this one is bytes, and reading it as UTF-8 first would
    // mangle the compressed stream.
    let file = std::fs::File::open(root.path(absolute)).ok()?;
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

fn availability(config: Option<&str>, symbol: &str, loaded: bool) -> Availability {
    if loaded {
        return Availability::Loaded;
    }
    let Some(config) = config else {
        return Availability::Unknown;
    };
    // `CONFIG_X=m` or `=y` is built; `# CONFIG_X is not set` is not.
    if config.contains(&format!("{symbol}=m")) || config.contains(&format!("{symbol}=y")) {
        Availability::NotLoaded
    } else {
        Availability::NotBuilt
    }
}

/// What this machine could have that it does not.
///
/// Only called when discovery came up short, so it is allowed to be slower than
/// the providers: it reads the whole kernel config.
pub fn report(root: &Root) -> Report {
    let vendor = vendor(root);
    let product = product(root);
    let loaded = loaded_modules(root);
    let config = kernel_config(root);
    let vendor_lower = vendor.to_ascii_lowercase();

    let suggestions = CANDIDATES
        .iter()
        .filter(|c| c.vendor.is_empty() || vendor_lower.contains(c.vendor))
        .map(|c| Suggestion {
            module: c.module.into(),
            gives: c.gives.into(),
            availability: availability(
                config.as_deref(),
                c.config,
                loaded.iter().any(|m| m == c.module),
            ),
            config: c.config.into(),
        })
        // A module that is loaded and generic -- nct6775 on a laptop that has
        // no Super-I/O -- is noise. Keep "loaded" only where it is the vendor's
        // own driver, where "it is loaded and there is still nothing" is the
        // answer someone needs.
        .filter(|s| {
            s.availability != Availability::Loaded
                || CANDIDATES
                    .iter()
                    .any(|c| c.module == s.module && !c.vendor.is_empty())
        })
        .collect();

    Report {
        vendor,
        product,
        kernel: root.read("/proc/sys/kernel/osrelease").unwrap_or_default(),
        suggestions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str =
        "CONFIG_ASUS_NB_WMI=m\n# CONFIG_SENSORS_ASUS_EC is not set\nCONFIG_HID_ASUS=m\n";

    #[test]
    fn a_built_but_unloaded_module_is_told_apart_from_one_never_built() {
        assert_eq!(
            availability(Some(CONFIG), "CONFIG_ASUS_NB_WMI", false),
            Availability::NotLoaded
        );
        assert_eq!(
            availability(Some(CONFIG), "CONFIG_SENSORS_ASUS_EC", false),
            Availability::NotBuilt
        );
    }

    #[test]
    fn a_loaded_module_is_reported_as_the_end_of_the_road() {
        assert_eq!(
            availability(Some(CONFIG), "CONFIG_ASUS_NB_WMI", true),
            Availability::Loaded
        );
    }

    #[test]
    fn an_unreadable_config_is_admitted_rather_than_guessed() {
        assert_eq!(
            availability(None, "CONFIG_ASUS_NB_WMI", false),
            Availability::Unknown
        );
    }

    #[test]
    fn the_unloaded_case_prints_the_command_to_type() {
        let s = Suggestion {
            module: "asus_nb_wmi".into(),
            gives: "platform profile".into(),
            availability: Availability::NotLoaded,
            config: "CONFIG_ASUS_NB_WMI".into(),
        };
        assert!(s.line().contains("modprobe asus_nb_wmi"));
    }

    #[test]
    fn a_config_symbol_is_not_matched_by_a_longer_one_that_contains_it() {
        // CONFIG_ASUS_WMI is a prefix of CONFIG_ASUS_WMI_SENSORS; matching on
        // the bare symbol would report the wrong driver as available.
        let config = "# CONFIG_SENSORS_ASUS_WMI is not set\nCONFIG_ASUS_WMI=m\n";
        assert_eq!(
            availability(Some(config), "CONFIG_SENSORS_ASUS_WMI", false),
            Availability::NotBuilt
        );
        assert_eq!(
            availability(Some(config), "CONFIG_ASUS_WMI", false),
            Availability::NotLoaded
        );
    }
}
