//! The wire, driven over a real socket against a captured machine.
//!
//! The daemon proper refuses to run as anything but root, and rightly so, which
//! would leave the protocol tested only by hand on a live machine. Lifting
//! `serve` into the library means the request and response shapes -- the part
//! most likely to drift as the protocol grows -- are exercised by `cargo test`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use raven_controlsd::protocol;
use raven_controlsd::supervisor::Supervisor;
use raven_hw::ipc::{Request, Response};
use raven_hw::Root;

/// A connected pair with the daemon's `serve` on one end.
fn served(fixture: &str) -> UnixStream {
    let root = Root::at(format!(
        "{}/../raven-hw/tests/fixtures/{fixture}",
        env!("CARGO_MANIFEST_DIR")
    ));
    let state = std::env::temp_dir().join(format!("raven-protocol-test-{fixture}.json"));
    let _ = std::fs::remove_file(&state);
    let supervisor = Arc::new(Mutex::new(Supervisor::new(root, state)));

    let (client, server) = UnixStream::pair().unwrap();
    std::thread::spawn(move || {
        let _ = protocol::serve(server, supervisor);
    });
    client
}

fn ask(stream: &mut UnixStream, request: Request) -> Response {
    writeln!(stream, "{}", serde_json::to_string(&request).unwrap()).unwrap();
    stream.flush().unwrap();
    let mut line = String::new();
    BufReader::new(stream.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn a_probe_returns_the_machine_the_daemon_is_looking_at() {
    let mut c = served("thinkpad-t14");
    let Response::State {
        knobs,
        readings,
        driving,
        diagnosis,
        machine,
    } = ask(&mut c, Request::Probe)
    else {
        panic!("probe did not answer with a state");
    };
    assert!(machine.contains("LENOVO"), "{machine}");
    assert!(knobs.iter().any(|k| k.id == "leds/tpacpi::kbd_backlight"));
    assert!(readings.iter().any(|r| r.value == 3100.0));
    assert!(driving.is_empty());
    // This machine has fan control, so there is nothing to explain.
    assert!(diagnosis.is_empty(), "{diagnosis:?}");
}

#[test]
fn a_machine_with_nothing_gets_its_diagnosis_over_the_wire() {
    let mut c = served("zephyrus-gu502du");
    let Response::State { diagnosis, .. } = ask(&mut c, Request::Probe) else {
        panic!()
    };
    assert!(
        diagnosis.iter().any(|d| d.contains("modprobe asus_nb_wmi")),
        "{diagnosis:?}"
    );
}

#[test]
fn ping_is_answered() {
    let mut c = served("bare-machine");
    assert!(matches!(ask(&mut c, Request::Ping), Response::Ok));
}

#[test]
fn a_request_that_cannot_be_carried_out_comes_back_as_an_error_not_a_dropped_connection() {
    let mut c = served("bare-machine");
    let response = ask(
        &mut c,
        Request::Release {
            knob: "hwmon/nothing/pwm1".into(),
        },
    );
    let Response::Error { message } = response else {
        panic!("expected an error, got {response:?}");
    };
    assert!(message.contains("not being driven"), "{message}");

    // And the connection is still good afterwards, which is the half that
    // breaks when an error path closes the stream.
    assert!(matches!(ask(&mut c, Request::Ping), Response::Ok));
}

#[test]
fn a_line_that_is_not_a_request_is_answered_rather_than_killing_the_connection() {
    let mut c = served("bare-machine");
    writeln!(c, "{{\"cmd\":\"detonate\"}}").unwrap();
    c.flush().unwrap();
    let mut line = String::new();
    BufReader::new(c.try_clone().unwrap())
        .read_line(&mut line)
        .unwrap();
    let response: Response = serde_json::from_str(&line).unwrap();
    let Response::Error { message } = response else {
        panic!("expected an error, got {response:?}")
    };
    assert!(message.contains("could not parse"), "{message}");
    assert!(matches!(ask(&mut c, Request::Ping), Response::Ok));
}

#[test]
fn releasing_everything_is_always_accepted() {
    // The panic button has to work even when there is nothing to release --
    // it is what someone types when they are not sure what state a machine is
    // in.
    for fixture in ["bare-machine", "desktop-nct6798"] {
        let mut c = served(fixture);
        assert!(
            matches!(ask(&mut c, Request::ReleaseAll), Response::Ok),
            "{fixture}"
        );
    }
}
