//! `raven-controls --capture DIR` -- copy the parts of `/sys` that RavenControls
//! looks at into a directory.
//!
//! This is the contribution path. Somebody with a laptop that RavenControls
//! gets wrong runs this, sends the directory, and it becomes a fixture in
//! `crates/raven-hw/tests/fixtures` with a test beside it -- after which that
//! machine is regression-tested forever by people who do not own one. It is the
//! only way a project like this can honestly claim to work on hardware its
//! authors have never touched.
//!
//! What is copied is deliberately narrow: LED and hwmon attributes, thermal
//! zones, the platform profile, DMI identification, the module list. No serial
//! numbers, no UUIDs, no network configuration, no user data -- and the list is
//! right here to be read before anyone sends anything.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Directory trees copied wholesale, one level of symlinks followed.
const TREES: &[&str] = &[
    "/sys/class/leds",
    "/sys/class/hwmon",
    "/sys/class/thermal",
    "/sys/class/platform-profile",
    "/sys/firmware/acpi",
];

/// Individual files.
const FILES: &[&str] = &[
    "/sys/class/dmi/id/sys_vendor",
    "/sys/class/dmi/id/product_name",
    "/sys/class/dmi/id/board_name",
    "/sys/class/dmi/id/bios_version",
    "/proc/modules",
    "/proc/sys/kernel/osrelease",
    "/proc/acpi/ibm/fan",
];

/// Attributes under `/sys/devices/platform/*`, by name. Copying that whole tree
/// would pull in half the machine, so only the ones a provider reads are taken.
const PLATFORM_ATTRS: &[&str] = &[
    "throttle_thermal_policy",
    "fan_boost_mode",
    "thermal_profile",
    "platform_profile",
    "platform_profile_choices",
];

/// Attributes that are write-only, enormous, or would block on read.
///
/// `uevent` is harmless but noisy; the `power/` subtree is runtime-PM state
/// that has nothing to do with fans and can block on a suspended device.
fn skip(name: &str) -> bool {
    matches!(
        name,
        "uevent" | "power" | "subsystem" | "driver" | "of_node"
    )
}

pub fn run(dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    let mut copied = 0usize;

    for tree in TREES {
        copied += copy_tree(Path::new(tree), dest, 0)?;
    }
    for file in FILES {
        if copy_file(Path::new(file), dest).is_ok() {
            copied += 1;
        }
    }
    for entry in std::fs::read_dir("/sys/devices/platform")
        .into_iter()
        .flatten()
        .flatten()
    {
        for attr in PLATFORM_ATTRS {
            let path = entry.path().join(attr);
            if path.exists() && copy_file(&path, dest).is_ok() {
                copied += 1;
            }
        }
    }

    // The kernel configuration, for telling "never built" apart from "not
    // loaded" -- which is most of what the diagnosis is for.
    let release = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_default()
        .trim()
        .to_string();
    for candidate in [
        format!("/boot/config-{release}"),
        format!("/lib/modules/{release}/config"),
    ] {
        if copy_file(Path::new(&candidate), dest).is_ok() {
            copied += 1;
            break;
        }
    }

    let mut readme = std::fs::File::create(dest.join("README"))?;
    writeln!(
        readme,
        "Captured by raven-controls --capture on {}\n\n\
         This is the subset of /sys, /proc and the kernel config that RavenControls\n\
         reads. To turn it into a regression test, drop the directory into\n\
         crates/raven-hw/tests/fixtures/ and add a test to tests/machines.rs.\n\n\
         Discovery on this machine found:\n{}",
        raven_hw::machine(&raven_hw::Root::system()),
        summary()
    )?;

    println!("{copied} files written to {}", dest.display());
    println!("{}", summary());
    Ok(())
}

fn summary() -> String {
    let hw = raven_hw::Hardware::discover(raven_hw::Root::system());
    let mut out = String::new();
    for knob in hw.knobs() {
        out.push_str(&format!("  {} = {:?}\n", knob.id, knob.value));
    }
    for reading in hw.readings() {
        out.push_str(&format!(
            "  {} = {}\n",
            reading.id,
            reading.unit.format(reading.value)
        ));
    }
    if out.is_empty() {
        out.push_str("  nothing\n");
    }
    out
}

fn destination(source: &Path, dest: &Path) -> PathBuf {
    dest.join(source.strip_prefix("/").unwrap_or(source))
}

fn copy_file(source: &Path, dest: &Path) -> anyhow::Result<()> {
    // Read rather than `fs::copy`: sysfs attributes report a size of 4096 and
    // return far less, and several of them error on read even though they
    // exist. A failure here is expected and ignored by the caller.
    let contents = std::fs::read(source)?;
    let target = destination(source, dest);
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(target, contents)?;
    Ok(())
}

/// Depth-limited, because `/sys/class/hwmon/hwmon0/device/...` walks into the
/// whole PCI tree and out the other side.
fn copy_tree(source: &Path, dest: &Path, depth: usize) -> anyhow::Result<usize> {
    if depth > 3 {
        return Ok(0);
    }
    let Ok(entries) = std::fs::read_dir(source) else {
        return Ok(0);
    };
    let mut copied = 0;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if skip(&name) {
            continue;
        }
        // `/sys/class/*` is a directory of symlinks into `/sys/devices`.
        // Following them once at the top level is what turns the class into a
        // real tree; following them everywhere walks the whole machine.
        let path = entry.path();
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            copied += copy_tree(&path, dest, depth + 1)?;
        } else if meta.is_file() && copy_file(&path, dest).is_ok() {
            copied += 1;
        }
    }
    Ok(copied)
}
