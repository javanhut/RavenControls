//! The shipped data files have to agree with the code.
//!
//! Both of these have drifted in other projects and the failure is quiet: the
//! rule installs, udev applies it, and the keyboard light stays root-owned
//! because the rule matches a name the provider no longer looks for. A test is
//! cheaper than the bug report.

use std::path::PathBuf;

fn data(name: &str) -> String {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "..", "data", name]
        .iter()
        .collect();
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_shipped_udev_rule_matches_the_one_the_code_prints() {
    // `raven-controls` shows this string when a control is read-only, and the
    // README quotes it. All three must be the same rule.
    let shipped = data("90-raven-controls.rules");
    assert!(
        shipped.contains(raven_hw::backlight::leds::UDEV_RULE),
        "data/90-raven-controls.rules no longer contains leds::UDEV_RULE:\n{}",
        raven_hw::backlight::leds::UDEV_RULE
    );
}

#[test]
fn the_shipped_rule_does_not_hand_out_fan_control() {
    // Deliberate, and the kind of thing that gets "helpfully" added later. A
    // session that can write pwmN can leave a fan at zero with nothing running
    // to put it back; that is what raven-controlsd is for.
    let shipped = data("90-raven-controls.rules");
    for line in shipped.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        assert!(
            !line.contains("hwmon") && !line.contains("pwm"),
            "the udev rule grants fan access, which raven-controlsd exists to avoid:\n  {line}"
        );
    }
}

#[test]
fn the_service_file_starts_the_daemon_that_was_built() {
    let service = data("controlsd.toml");
    assert!(service.contains("raven-controlsd"));
    // A daemon that is not restarted leaves a manually-controlled fan pinned
    // for as long as it stays dead.
    assert!(service.contains("restart = true"), "{service}");
}
