//! What this machine can do about its keyboard light and its fans.
//!
//! The whole crate is built on one refusal: never ask what laptop this is.
//!
//! A keyboard-backlight control written for a Zephyrus is fifteen lines and
//! works on a Zephyrus. The same fifteen lines written against
//! `/sys/class/leds/*::kbd_backlight` work on every laptop whose keyboard light
//! has a kernel driver, because the kernel already did the per-vendor work and
//! published one name for the result. The same is true of fans and `hwmon`, of
//! coarse thermal modes and the ACPI platform profile.
//!
//! So: providers detect interfaces, not machines. Each publishes `Knob`s
//! described by a `Domain` -- a percentage, ordered steps, named modes, a
//! switch, a colour -- and the window renders domains. A laptop nobody here has
//! seen gets a working window if its kernel driver speaks any of these, and a
//! specific, actionable explanation from `diagnose` if it does not.
//!
//! Vendor-specific interfaces do exist, in `fan::vendor`, and they are a table
//! of documented kernel ABI entries rather than code. Adding a machine is a row.
//!
//! ```no_run
//! use raven_hw::{Hardware, sysfs::Root, model::{Role, Setting}};
//!
//! let hw = Hardware::discover(Root::system());
//! for knob in hw.knobs_for(Role::KeyboardBacklight) {
//!     println!("{}: {:?}", knob.label, knob.value);
//! }
//! ```

pub mod backlight;
pub mod curve;
pub mod diagnose;
pub mod fan;
pub mod guard;
pub mod ipc;
pub mod model;
pub mod provider;
pub mod sysfs;

pub use model::{Domain, Knob, Reading, Role, Setting, Unit};
pub use provider::{Hardware, Provider};
pub use sysfs::Root;

/// A one-line description of the machine, for logs and bug reports.
pub fn machine(root: &Root) -> String {
    let vendor = diagnose::vendor(root);
    let product = diagnose::product(root);
    let kernel = root.read("/proc/sys/kernel/osrelease").unwrap_or_default();
    match (vendor.is_empty(), product.is_empty()) {
        (true, true) => format!("unknown machine, Linux {kernel}"),
        _ => format!("{vendor} {product}, Linux {kernel}"),
    }
}
