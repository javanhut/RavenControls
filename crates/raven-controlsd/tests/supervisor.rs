//! The daemon's decisions, against captured machines and a temporary state
//! file. No root, no sockets, no fans.

use std::path::PathBuf;

use raven_controlsd::state::{SavedCurve, State};
use raven_controlsd::supervisor::Supervisor;
use raven_hw::curve::{Curve, Point};
use raven_hw::model::Setting;
use raven_hw::Root;

fn fixture(name: &str) -> Root {
    Root::at(format!(
        "{}/../raven-hw/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("raven-controlsd-test-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("state.json")
}

#[test]
fn a_curve_can_only_be_put_on_something_that_takes_a_duty_cycle() {
    let mut s = Supervisor::new(fixture("thinkpad-t14"), scratch("roles"));

    // A ThinkPad's procfs fan takes levels, not a percentage, so a curve has
    // nothing continuous to drive. Saying so beats writing "level 43".
    let err = s
        .drive(
            "vendor/thinkpad/fan",
            "hwmon/thinkpad/temp1",
            Curve::default(),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("not a fan duty cycle"), "{err}");

    // And neither does a thermal profile.
    let err = s
        .drive(
            "platform-profile/acpi",
            "hwmon/thinkpad/temp1",
            Curve::default(),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("not a fan duty cycle"), "{err}");
}

#[test]
fn a_curve_needs_a_sensor_that_exists() {
    let mut s = Supervisor::new(fixture("thinkpad-t14"), scratch("sensor"));
    let err = s
        .drive(
            "hwmon/thinkpad/pwm1",
            "hwmon/thinkpad/temp9",
            Curve::default(),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("no sensor"), "{err}");
}

#[test]
fn a_captured_machine_is_never_written_to() {
    // The guard has to fail closed. If it did not, running the daemon's tests
    // would drive the fans of the machine running them.
    let mut s = Supervisor::new(fixture("desktop-nct6798"), scratch("readonly"));
    let err = s
        .drive(
            "hwmon/nct6798/pwm1",
            "hwmon/nct6798/temp1",
            Curve::default(),
        )
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("captured tree") || err.contains("could not take control"),
        "{err}"
    );
    assert!(!s.is_driving_anything());
}

#[test]
fn a_set_on_something_that_does_not_exist_says_so_rather_than_panicking() {
    let mut s = Supervisor::new(fixture("bare-machine"), scratch("missing"));
    let err = s
        .set("hwmon/nothing/pwm1", &Setting::Percent { percent: 50.0 })
        .unwrap_err()
        .to_string();
    assert!(err.contains("no control called"), "{err}");
}

#[test]
fn releasing_a_fan_that_is_not_being_driven_is_an_error_not_a_crash() {
    let mut s = Supervisor::new(fixture("thinkpad-t14"), scratch("release"));
    assert!(s.release("hwmon/thinkpad/pwm1").is_err());
    // And releasing everything, always fine, including when there is nothing.
    s.release_all();
    assert!(!s.is_driving_anything());
}

#[test]
fn a_tick_on_a_machine_with_no_curves_does_nothing_and_does_not_panic() {
    for name in ["bare-machine", "macbook-applesmc", "desktop-nct6798"] {
        let mut s = Supervisor::new(fixture(name), scratch(name));
        s.tick();
        assert!(s.driving().is_empty(), "{name}");
    }
}

// ---- the state file -------------------------------------------------------

#[test]
fn curves_survive_a_round_trip_through_the_state_file() {
    let path = scratch("roundtrip");
    let curve = Curve::new(vec![
        Point {
            temp_c: 45.0,
            percent: 0.0,
        },
        Point {
            temp_c: 85.0,
            percent: 100.0,
        },
    ])
    .unwrap();
    let mut state = State::default();
    state.curves.insert(
        "hwmon/nct6798/pwm1".into(),
        SavedCurve {
            sensor: "hwmon/nct6798/temp1".into(),
            curve: curve.clone(),
        },
    );
    state.save(&path).unwrap();

    let back = State::load(&path);
    let saved = back.curves.get("hwmon/nct6798/pwm1").expect("curve lost");
    assert_eq!(saved.sensor, "hwmon/nct6798/temp1");
    assert_eq!(saved.curve, curve);
}

#[test]
fn a_corrupt_state_file_starts_the_daemon_empty_rather_than_stopping_it() {
    // The daemon must come up whatever is in this file: it is the thing that
    // hands the fans back, and refusing to start would leave them wherever the
    // last run left them.
    let path = scratch("corrupt");
    std::fs::write(&path, b"{ this is not json").unwrap();
    assert!(State::load(&path).curves.is_empty());

    let path = scratch("absent");
    assert!(State::load(&path).curves.is_empty());
}

#[test]
fn a_saved_curve_for_a_fan_that_is_not_here_is_kept_not_discarded() {
    // A driver that has not probed yet is far likelier than a fan that has
    // been removed, and silently throwing away someone's tuning because of a
    // boot-order race would be its own bug.
    let path = scratch("absent-fan");
    let mut state = State::default();
    state.curves.insert(
        "hwmon/nct6798/pwm1".into(),
        SavedCurve {
            sensor: "hwmon/nct6798/temp1".into(),
            curve: Curve::default(),
        },
    );
    state.save(&path).unwrap();

    // The ThinkPad has no nct6798, so restoring finds nothing to apply.
    let mut s = Supervisor::new(fixture("thinkpad-t14"), path.clone());
    s.restore_saved();
    assert!(!s.is_driving_anything());

    // And the file still has it, for the boot where the module does load.
    assert!(State::load(&path).curves.contains_key("hwmon/nct6798/pwm1"));
}
