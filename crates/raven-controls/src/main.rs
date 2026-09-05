//! `raven-controls` -- keyboard backlight and fan control for Raven Linux.
//!
//!   raven-controls              the window
//!   raven-controls --probe      what this machine exposes, and what it does not
//!   raven-controls --capture D  the sysfs it exposes, as a test fixture

mod capture;
mod client;
mod ui;

use gtk::glib;
use gtk4 as gtk;

fn main() -> glib::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!(
            "raven-controls -- keyboard backlight and fan control\n\n\
             Usage:\n\
             \x20 raven-controls                  open the window\n\
             \x20 raven-controls --probe          print what this machine exposes\n\
             \x20 raven-controls --capture DIR    save this machine's sysfs as a test fixture\n\n\
             Fan curves and manual fan speeds need raven-controlsd running as root; the\n\
             keyboard backlight does not, given the udev rule from the README.\n"
        );
        return glib::ExitCode::SUCCESS;
    }

    if args.iter().any(|a| a == "--probe") {
        probe();
        return glib::ExitCode::SUCCESS;
    }

    if let Some(i) = args.iter().position(|a| a == "--capture") {
        let Some(dir) = args.get(i + 1) else {
            eprintln!("--capture needs a directory to write to");
            return glib::ExitCode::FAILURE;
        };
        return match capture::run(std::path::Path::new(dir)) {
            Ok(()) => glib::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("capture failed: {e}");
                glib::ExitCode::FAILURE
            }
        };
    }

    ui::run()
}

/// `--probe`: every provider, what it found, and -- when that is nothing -- the
/// reason.
///
/// The first thing to run on a machine RavenControls gets wrong, and the thing
/// to paste into a bug report. It needs no display, no daemon and no root.
fn probe() {
    let root = raven_hw::Root::system();
    let hw = raven_hw::Hardware::discover(root.clone());
    let client = client::Client::new();

    println!("{}", raven_hw::machine(&root));
    println!(
        "raven-controlsd: {}",
        if client.daemon_available() {
            "running"
        } else {
            "not running (fan curves and manual duty unavailable)"
        }
    );

    println!("\n== controls");
    let knobs = hw.knobs();
    if knobs.is_empty() {
        println!("  none");
    }
    for knob in &knobs {
        println!(
            "  [{:?}] {}\n      id       {}\n      domain   {:?}\n      value    {:?}\n      \
             writable {}\n      from     {} via {}",
            knob.role,
            knob.label,
            knob.id,
            knob.domain,
            knob.value,
            knob.writable,
            knob.origin,
            knob.provider
        );
    }

    println!("\n== sensors");
    let readings = hw.readings();
    if readings.is_empty() {
        println!("  none");
    }
    for reading in &readings {
        let critical = reading
            .critical
            .map(|c| format!(" (critical {c:.0} °C)"))
            .unwrap_or_default();
        println!(
            "  {:<28} {}{critical}\n      {}",
            reading.id,
            reading.unit.format(reading.value),
            reading.origin
        );
    }

    println!("\n== providers");
    for provider in hw.providers() {
        let found = provider.knobs(&root).len() + provider.readings(&root).len();
        println!("  {:<18} {found} found", provider.id());
    }

    let has_thermal = knobs.iter().any(|k| {
        matches!(
            k.role,
            raven_hw::Role::FanDuty | raven_hw::Role::ThermalProfile
        )
    });
    if !has_thermal {
        println!("\n== why there is no fan control here");
        let report = raven_hw::diagnose::report(&root);
        if report.suggestions.is_empty() {
            println!("  Nothing known would help. Capture this machine with:");
            println!("      raven-controls --capture ~/machine");
        }
        for suggestion in &report.suggestions {
            println!("  {}", suggestion.line().replace('\n', "\n  "));
        }
    }
}
