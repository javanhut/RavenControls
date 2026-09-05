//! The newline-JSON wire, split out of `main.rs` so it can be driven over a
//! socketpair in a test rather than only by a running daemon on a real machine.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};

use raven_hw::ipc::{Request, Response};

use crate::supervisor::Supervisor;

pub fn serve(stream: UnixStream, supervisor: Arc<Mutex<Supervisor>>) -> anyhow::Result<()> {
    let reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => handle(request, &supervisor),
            Err(e) => Response::error(format!("could not parse that request: {e}")),
        };
        writeln!(writer, "{}", serde_json::to_string(&response)?)?;
        writer.flush()?;
    }
    Ok(())
}

pub fn handle(request: Request, supervisor: &Arc<Mutex<Supervisor>>) -> Response {
    let mut s = match supervisor.lock() {
        Ok(s) => s,
        // A panic in a tick poisoned the lock. The fans were released by
        // `Guard`'s `Drop` on the way through, so the honest answer is to say
        // so rather than pretend to still be in control.
        Err(e) => {
            tracing::error!("the supervisor panicked; fans were released");
            return Response::error(format!(
                "raven-controlsd is in a broken state and has released the fans: {e}"
            ));
        }
    };
    match request {
        Request::Ping => Response::Ok,
        Request::Probe => {
            let knobs = s.hardware().knobs();
            let readings = s.readings();
            // Only diagnose when there is something to explain. On a machine
            // with working fan control this list is empty and the window shows
            // no banner.
            let diagnosis = if knobs.iter().any(|k| {
                k.role == raven_hw::Role::FanDuty || k.role == raven_hw::Role::ThermalProfile
            }) {
                Vec::new()
            } else {
                raven_hw::diagnose::report(s.hardware().root())
                    .suggestions
                    .iter()
                    .map(|d| d.line())
                    .collect()
            };
            Response::State {
                knobs,
                readings,
                driving: s.driving(),
                diagnosis,
                machine: raven_hw::machine(s.hardware().root()),
            }
        }
        Request::Set { knob, value } => match s.set(&knob, &value) {
            Ok(()) => Response::Ok,
            Err(e) => Response::error(e),
        },
        Request::Curve {
            knob,
            sensor,
            curve,
        } => match s.drive(&knob, &sensor, curve) {
            Ok(()) => Response::Ok,
            Err(e) => Response::error(e),
        },
        Request::Release { knob } => match s.release(&knob) {
            Ok(()) => Response::Ok,
            Err(e) => Response::error(e),
        },
        Request::ReleaseAll => {
            s.release_all();
            Response::Ok
        }
    }
}
