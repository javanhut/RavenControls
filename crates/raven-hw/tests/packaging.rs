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
    assert!(
        !targets.is_empty(),
        "the Makefile declares no .PHONY targets"
    );

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

/// Destination paths an `imlazy` command writes or removes.
fn paths_in(lazy: &str, command: &str, verb: &str) -> Vec<String> {
    let block = lazy
        .split(&format!("[commands.{command}]"))
        .nth(1)
        .unwrap_or_else(|| panic!("lazy.toml has no [commands.{command}]"));
    // Up to the next command.
    let block = block.split("\n[commands.").next().unwrap_or(block);
    block
        .lines()
        .map(str::trim)
        // The guarded `sh -c` steps are conditional by design; only the
        // unconditional file operations are the contract being checked.
        .filter(|l| !l.contains("sh -c"))
        .filter_map(|line| {
            let at = line.find(verb)? + verb.len();
            // The destination is the last whitespace-separated token before
            // the closing quote.
            line[at..]
                .trim_end_matches(['"', ','])
                .split_whitespace()
                .next_back()
                .map(str::to_string)
        })
        .filter(|p| p.starts_with("{{"))
        .collect()
}

#[test]
fn everything_install_writes_is_something_uninstall_removes() {
    // The failure this prevents is quiet and annoying: a file added to install,
    // forgotten in uninstall, and left behind on every machine that ever tried
    // this application. Verified end to end once by hand with
    //   imlazy install   prefix=/tmp/rc-test sysconfdir=/tmp/rc-test/etc sudo=
    //   imlazy uninstall prefix=/tmp/rc-test sysconfdir=/tmp/rc-test/etc sudo=
    // and kept honest from here by this.
    let lazy = data("../lazy.toml");

    let installed = paths_in(&lazy, "install", "install -Dm644 ");
    let mut installed = installed;
    installed.extend(paths_in(&lazy, "install", "install -Dm755 "));
    assert!(
        installed.len() >= 8,
        "only found {} install destinations; the parser has drifted: {installed:#?}",
        installed.len()
    );

    let mut removed = paths_in(&lazy, "uninstall", "rm -f ");
    removed.extend(paths_in(&lazy, "uninstall", "rm -rf "));
    assert!(!removed.is_empty(), "uninstall removes nothing");

    for dest in &installed {
        // Either removed outright, or inside a directory that is.
        let covered = removed
            .iter()
            .any(|r| dest == r || dest.starts_with(&format!("{r}/")));
        assert!(
            covered,
            "install writes {dest} and uninstall never removes it\nuninstall covers: {removed:#?}"
        );
    }
}

#[test]
fn uninstall_stops_the_daemon_before_deleting_its_service_file() {
    // Order matters here and nowhere else in that command: raven-controlsd
    // hands every fan back to firmware as it exits, so removing the service
    // definition out from under a running daemon would skip that and leave the
    // fans wherever the last curve tick put them.
    let lazy = data("../lazy.toml");
    let block = lazy
        .split("[commands.uninstall]")
        .nth(1)
        .unwrap()
        .split("\n[commands.")
        .next()
        .unwrap();
    let stop = block.find("raven-rc stop controlsd").expect("no stop step");
    let remove = block
        .find("rm -f {{sysconfdir}}/raven/init.d/controlsd.toml")
        .expect("no removal step");
    assert!(
        stop < remove,
        "uninstall deletes the service file before stopping the daemon"
    );
}
