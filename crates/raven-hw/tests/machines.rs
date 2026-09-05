//! Discovery, run against ten machines.
//!
//! Nine of these are not the machine this was written on, and that is the
//! point. `scripts/make-fixtures.sh` builds the trees; `Root::at` points the
//! providers at one instead of at `/sys`; and the assertions below are what
//! "RavenControls works on a laptop nobody here owns" actually means in
//! practice, rather than as a claim in a README.
//!
//! A machine that turns out to be wrong gets captured with
//! `raven-controls --capture`, dropped into `tests/fixtures/`, and given a test
//! here. That is the whole contribution path, and it needs no hardware from
//! anybody else.

use raven_hw::model::{Domain, Role, Setting};
use raven_hw::{Hardware, Root};

fn machine(name: &str) -> Hardware {
    let dir = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    assert!(
        std::path::Path::new(&dir).is_dir(),
        "missing fixture {name}: run scripts/make-fixtures.sh"
    );
    Hardware::discover(Root::at(dir))
}

fn backlights(hw: &Hardware) -> Vec<raven_hw::Knob> {
    hw.knobs_for(Role::KeyboardBacklight)
}

// ---------------------------------------------------------------------------
// The keyboard light, across every driver that spells it the kernel's way.
// ---------------------------------------------------------------------------

#[test]
fn one_matcher_finds_the_keyboard_light_on_four_different_vendors() {
    for (name, expected_max) in [
        ("zephyrus-gu502du", 3u32),
        ("thinkpad-t14", 2),
        ("macbook-applesmc", 255),
        ("rgb-keyboard", 255),
    ] {
        let hw = machine(name);
        let found = backlights(&hw);
        let brightness = found
            .iter()
            .find(|k| !k.id.ends_with("/color"))
            .unwrap_or_else(|| panic!("{name}: no keyboard backlight found"));
        assert_eq!(
            brightness.domain,
            Domain::Percent {
                raw_max: expected_max
            },
            "{name} got the wrong range"
        );
        // Not one of these labels names a vendor.
        assert_eq!(brightness.label, "Keyboard backlight", "{name}");
    }
}

#[test]
fn the_other_leds_on_the_keyboard_are_not_mistaken_for_the_backlight() {
    // This Zephyrus publishes ten capslock/numlock/compose LEDs and a Wi-Fi
    // LED alongside the one that matters.
    let hw = machine("zephyrus-gu502du");
    let found = backlights(&hw);
    assert_eq!(found.len(), 1, "found {found:#?}");
    assert_eq!(found[0].id, "leds/asus::kbd_backlight");
}

#[test]
fn a_multicolour_keyboard_gets_a_colour_control_as_well_as_a_brightness_one() {
    let hw = machine("rgb-keyboard");
    let found = backlights(&hw);
    assert_eq!(found.len(), 2, "{found:#?}");
    let colour = found.iter().find(|k| k.id.ends_with("/color")).unwrap();
    assert_eq!(
        colour.domain,
        Domain::Color {
            channels: vec!["red".into(), "green".into(), "blue".into()],
            raw_max: 255
        }
    );
    assert_eq!(
        colour.value,
        Setting::Color {
            intensities: vec![255, 128, 0]
        }
    );
}

#[test]
fn a_desktop_with_no_keyboard_light_simply_has_none() {
    // Not an error, not an empty control: nothing to show.
    assert!(backlights(&machine("desktop-nct6798")).is_empty());
}

// ---------------------------------------------------------------------------
// Fans.
// ---------------------------------------------------------------------------

#[test]
fn a_super_io_boards_five_headers_and_its_gpu_all_appear() {
    let hw = machine("desktop-nct6798");
    let duties = hw.knobs_for(Role::FanDuty);
    assert_eq!(
        duties.len(),
        6,
        "{:#?}",
        duties.iter().map(|k| &k.id).collect::<Vec<_>>()
    );
    assert!(duties.iter().any(|k| k.id == "hwmon/nct6798/pwm1"));
    assert!(duties.iter().any(|k| k.id == "hwmon/amdgpu/pwm1"));
    // The board's own label wins over the PWM number.
    assert!(duties.iter().any(|k| k.label.starts_with("CPU fan")));

    let readings = hw.readings();
    assert_eq!(
        readings
            .iter()
            .filter(|r| r.unit == raven_hw::Unit::Rpm)
            .count(),
        6
    );
    // A critical temperature from the driver, which the curve engine treats as
    // the point at which it stops being the authority.
    let hot = readings
        .iter()
        .find(|r| r.id == "hwmon/nct6798/temp1")
        .unwrap();
    assert_eq!(hot.value, 41.0);
    assert_eq!(hot.critical, Some(95.0));
}

#[test]
fn a_board_in_its_own_automatic_mode_keeps_that_mode_on_the_menu() {
    // nct6798 sits in Smart Fan IV, which is pwmN_enable = 5. Offering only
    // 0/1/2 would leave no way back to the mode the board shipped in.
    let hw = machine("desktop-nct6798");
    let mode = hw.knob("hwmon/nct6798/pwm1_enable").unwrap();
    assert_eq!(
        mode.value,
        Setting::Mode {
            name: "Automatic (driver mode 5)".into()
        }
    );
    let Domain::Modes { options } = &mode.domain else {
        panic!("{:?}", mode.domain)
    };
    assert_eq!(options[0], "Automatic (driver mode 5)");
    assert!(options.contains(&"Manual".to_string()));
}

#[test]
fn two_identical_gpus_get_two_distinguishable_controls() {
    // The case that breaks any scheme keying a saved fan curve on the driver
    // name -- or, worse, on the hwmon index, which is whichever probed first
    // this boot.
    let hw = machine("dual-gpu");
    let duties = hw.knobs_for(Role::FanDuty);
    assert_eq!(duties.len(), 2);
    let ids: Vec<&str> = duties.iter().map(|k| k.id.as_str()).collect();
    assert!(ids.iter().all(|id| id.contains("0000:")), "{ids:?}");
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn a_fan_the_kernel_can_only_read_gives_a_reading_and_no_control() {
    // applesmc publishes fan speed and no PWM. Inventing a slider that writes
    // nowhere would be worse than showing none.
    let hw = machine("macbook-applesmc");
    assert!(hw.knobs_for(Role::FanDuty).is_empty());
    let rpm = hw
        .readings()
        .iter()
        .find(|r| r.unit == raven_hw::Unit::Rpm)
        .cloned();
    assert_eq!(rpm.map(|r| r.value), Some(2160.0));
}

#[test]
fn an_acpi_fan_becomes_steps_and_the_throttles_beside_it_are_left_alone() {
    let hw = machine("acpi-fan");
    let knobs = hw.knobs_for(Role::ThermalProfile);
    assert_eq!(knobs.len(), 1, "{knobs:#?}");
    let fan = &knobs[0];
    assert!(fan.label.contains("Fan"));
    let Domain::Steps { labels } = &fan.domain else {
        panic!("{:?}", fan.domain)
    };
    // 0..=9 inclusive.
    assert_eq!(labels.len(), 10);
    assert_eq!(labels[0], "Off");
    assert_eq!(labels[9], "9 (maximum)");
    assert_eq!(fan.value, Setting::Step { index: 3 });
    assert!(hw.readings().iter().any(|r| r.value == 2800.0));
}

// ---------------------------------------------------------------------------
// Coarse thermal knobs.
// ---------------------------------------------------------------------------

#[test]
fn the_acpi_platform_profile_is_read_from_either_kernel_layout() {
    for name in ["thinkpad-t14", "multi-handler-profile"] {
        let hw = machine(name);
        let profile = hw
            .knobs_for(Role::ThermalProfile)
            .into_iter()
            .find(|k| k.provider == "platform-profile")
            .unwrap_or_else(|| panic!("{name}: no platform profile"));
        let Domain::Modes { options } = &profile.domain else {
            panic!("{name}: {:?}", profile.domain)
        };
        assert!(
            options.contains(&"Balanced".to_string()),
            "{name}: {options:?}"
        );
        assert_eq!(
            profile.value,
            Setting::Mode {
                name: "Balanced".into()
            }
        );
    }
}

#[test]
fn the_new_kernel_layout_and_its_compatibility_alias_are_one_control_not_two() {
    let hw = machine("multi-handler-profile");
    let profiles: Vec<_> = hw
        .knobs_for(Role::ThermalProfile)
        .into_iter()
        .filter(|k| k.provider == "platform-profile")
        .collect();
    assert_eq!(profiles.len(), 1, "{profiles:#?}");
}

#[test]
fn a_thinkpads_procfs_fan_turns_into_levels_and_a_speed() {
    let hw = machine("thinkpad-t14");
    let fan = hw
        .knob("vendor/thinkpad/fan")
        .expect("no ThinkPad fan control");
    assert_eq!(
        fan.value,
        Setting::Mode {
            name: "Automatic".into()
        }
    );
    let Domain::Modes { options } = &fan.domain else {
        panic!()
    };
    assert_eq!(options.len(), 10); // Automatic, Level 0-7, Full speed
    assert!(hw
        .readings()
        .iter()
        .any(|r| r.id == "vendor/thinkpad/fan" && r.value == 3100.0));
}

#[test]
fn asus_specific_knobs_are_found_by_attribute_not_by_driver_directory_name() {
    let hw = machine("zephyrus-with-module");
    let knobs = hw.knobs_for(Role::ThermalProfile);
    let throttle = knobs.iter().find(|k| k.label == "Throttle policy").unwrap();
    assert_eq!(
        throttle.value,
        Setting::Mode {
            name: "Balanced".into()
        }
    );
    // The reference is carried through so nobody has to wonder whether the
    // numbers were guessed.
    assert!(throttle.origin.contains("sysfs-platform-asus-wmi"));
    assert!(knobs.iter().any(|k| k.label == "Fan boost"));
    assert!(knobs.iter().any(|k| k.provider == "platform-profile"));
}

#[test]
fn a_thinkpad_offers_every_layer_it_has_at_once() {
    // The point of the whole arrangement: one machine, four interfaces, no
    // per-machine code anywhere.
    let hw = machine("thinkpad-t14");
    assert!(!backlights(&hw).is_empty());
    assert!(!hw.knobs_for(Role::FanDuty).is_empty());
    assert!(!hw.knobs_for(Role::FanControlMode).is_empty());
    let coarse = hw.knobs_for(Role::ThermalProfile);
    assert!(coarse.iter().any(|k| k.provider == "platform-profile"));
    assert!(coarse.iter().any(|k| k.provider == "vendor"));
}

// ---------------------------------------------------------------------------
// Nothing found, which is a result and not a failure.
// ---------------------------------------------------------------------------

#[test]
fn a_machine_with_no_fan_interface_is_told_what_would_give_it_one() {
    let hw = machine("bare-machine");
    assert!(hw.knobs_for(Role::FanDuty).is_empty());
    let report = raven_hw::diagnose::report(hw.root());
    assert_eq!(report.vendor, "Dell Inc.");
    let lines: Vec<String> = report.suggestions.iter().map(|s| s.line()).collect();
    let joined = lines.join("\n");
    // Built as a module, not loaded: the actionable case.
    assert!(joined.contains("modprobe dell_smm_hwmon"), "{joined}");
    // Never built: no point telling anyone to modprobe it.
    assert!(joined.contains("CONFIG_SENSORS_NCT6775"), "{joined}");
    assert!(!joined.contains("modprobe nct6775"), "{joined}");
}

#[test]
fn this_very_laptop_is_diagnosed_rather_than_shrugged_at() {
    // The machine RavenControls was written on has a keyboard light and no fan
    // control at all -- and the reason is a module that is built and not
    // loaded, which no amount of staring at an empty window would reveal.
    let hw = machine("zephyrus-gu502du");
    assert_eq!(backlights(&hw).len(), 1);
    assert!(hw.knobs_for(Role::FanDuty).is_empty());
    assert!(hw.knobs_for(Role::ThermalProfile).is_empty());

    let lines: Vec<String> = raven_hw::diagnose::report(hw.root())
        .suggestions
        .iter()
        .map(|s| s.line())
        .collect();
    let joined = lines.join("\n");
    assert!(joined.contains("modprobe asus_nb_wmi"), "{joined}");
    // And the honest half: the sensor driver was never compiled, so no amount
    // of modprobe will produce a fan speed here.
    assert!(joined.contains("CONFIG_SENSORS_ASUS_EC"), "{joined}");
    assert!(!joined.contains("modprobe asus_ec_sensors"), "{joined}");
}

#[test]
fn what_the_diagnosis_promises_is_what_loading_the_module_delivers() {
    // The Zephyrus is told that asus_nb_wmi would add a platform profile, a
    // throttle policy and fan boost. The same tree with the module loaded has
    // exactly those three, which is the assertion that keeps the advice honest.
    let before = machine("zephyrus-gu502du");
    let after = machine("zephyrus-with-module");
    assert!(before.knobs_for(Role::ThermalProfile).is_empty());
    let labels: Vec<String> = after
        .knobs_for(Role::ThermalProfile)
        .into_iter()
        .map(|k| k.label)
        .collect();
    assert!(
        labels.contains(&"Platform profile".to_string()),
        "{labels:?}"
    );
    assert!(
        labels.contains(&"Throttle policy".to_string()),
        "{labels:?}"
    );
    assert!(labels.contains(&"Fan boost".to_string()), "{labels:?}");
}

// ---------------------------------------------------------------------------
// Safety, which has to hold on machines nobody here can test by hand.
// ---------------------------------------------------------------------------

#[test]
fn nothing_in_a_captured_tree_is_writable() {
    for name in ["thinkpad-t14", "desktop-nct6798", "rgb-keyboard"] {
        let hw = machine(name);
        for knob in hw.knobs() {
            assert!(!knob.writable, "{name}: {} claims to be writable", knob.id);
        }
        let err = hw
            .set(
                &hw.knobs()[0].id.clone(),
                &Setting::Percent { percent: 100.0 },
            )
            .unwrap_err();
        assert!(err.to_string().contains("not writable"), "{err}");
    }
}

#[test]
fn a_value_that_does_not_fit_the_hardware_is_refused_before_any_write() {
    let hw = machine("thinkpad-t14");
    // The ThinkPad fan takes modes, not percentages.
    let err = hw
        .set("vendor/thinkpad/fan", &Setting::Percent { percent: 50.0 })
        .unwrap_err();
    assert!(err.to_string().contains("does not fit"), "{err}");

    // And a mode this firmware never offered.
    let err = hw
        .set(
            "platform-profile/acpi",
            &Setting::Mode {
                name: "Turbo".into(),
            },
        )
        .unwrap_err();
    assert!(
        err.to_string().contains("Low power, Balanced, Performance"),
        "{err}"
    );
}
