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

#[test]
fn every_make_target_is_a_real_imlazy_command() {
    // The Makefile forwards to imlazy rather than duplicating it, which is only
    // true for as long as every target it advertises exists in lazy.toml. A
    // `make install-service` that forwards to a command nobody defined fails
    // with imlazy's error rather than make's, which is confusing enough to be
    // worth a test.
    let makefile = data("../Makefile");
    let lazy = data("../lazy.toml");

    let commands: Vec<&str> = lazy
        .lines()
        .filter_map(|l| l.trim().strip_prefix("[commands.")?.strip_suffix(']'))
        .collect();
    assert!(!commands.is_empty(), "lazy.toml defines no commands");

    // The .PHONY list, which continues across backslash-escaped newlines.
    let phony: String = makefile
        .lines()
        .skip_while(|l| !l.starts_with(".PHONY:"))
        .take_while(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .replace(".PHONY:", " ")
        .replace('\\', " ");
    let targets: Vec<&str> = phony.split_whitespace().collect();
    assert!(!targets.is_empty(), "the Makefile declares no .PHONY targets");

    for target in targets {
        assert!(
            commands.contains(&target),
            "make target {target:?} has no [commands.{target}] in lazy.toml"
        );
    }
}

#[test]
fn the_makefile_does_not_reimplement_anything() {
    // The moment a recipe here runs cargo or install directly, the two files
    // can disagree about what "install" means -- which is the drift this
    // arrangement exists to prevent.
    let makefile = data("../Makefile");
    for line in makefile.lines().filter(|l| l.starts_with('\t')) {
        let line = line.trim_start_matches(['\t', '@']);
        assert!(
            !line.starts_with("cargo ") && !line.starts_with("install "),
            "the Makefile is doing work instead of forwarding:\n  {line}"
        );
    }
}
