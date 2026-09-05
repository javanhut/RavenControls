//! The wire between `raven-controls` (the window) and `raven-controlsd` (the
//! half that is allowed to write).
//!
//! Newline-delimited JSON on a Unix socket, which is how `cawd`, `raven-powerd`
//! and `raven-timed` already talk on Raven. One request per line, one response
//! per line, no framing to get wrong and a protocol you can drive from `socat`
//! while debugging.

use serde::{Deserialize, Serialize};

use crate::curve::Curve;
use crate::model::{Knob, Reading, Setting};

/// Group `video` -- the one the session already holds for DRM -- so no new
/// group and no new membership to explain.
pub const SOCKET: &str = "/run/raven-controls/ctl";
pub const GROUP: &str = "video";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    /// Everything discovered, plus what the daemon is currently doing.
    Probe,
    /// Set one knob.
    Set { knob: String, value: Setting },
    /// Drive a fan from a curve. Replaces any curve already on that fan.
    ///
    /// `sensor` is chosen by the caller because the kernel does not say which
    /// temperature belongs to which fan; hwmon has no such relation, and a
    /// guess would be wrong on any machine with a discrete GPU.
    Curve {
        knob: String,
        sensor: String,
        curve: Curve,
    },
    /// Stop driving a fan and hand it back to firmware.
    Release { knob: String },
    /// Hand every fan back.
    ///
    /// Not what the window does when it closes: a curve is a setting, and
    /// someone who tuned their laptop to be quiet expects it to stay quiet
    /// after closing the window that tuned it. This is the deliberate "stop,
    /// firmware takes over" that a person asks for, and the panic button from
    /// a shell.
    ReleaseAll,
    /// Is the daemon alive and answering? Nothing more.
    ///
    /// The watchdog inside the daemon watches its own control loop, not its
    /// clients -- a window closing must not stop a fan curve, so client
    /// liveness is not a safety signal and is not treated as one.
    Ping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "kebab-case")]
pub enum Response {
    Ok,
    Error {
        message: String,
    },
    State {
        knobs: Vec<Knob>,
        readings: Vec<Reading>,
        /// One entry per fan the daemon is currently driving.
        driving: Vec<Driving>,
        /// Present only when nothing useful was found, so the window can say
        /// what would help rather than shrugging.
        diagnosis: Vec<String>,
        machine: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Driving {
    pub knob: String,
    pub sensor: String,
    pub curve: Curve,
    pub percent: f64,
    pub temp_c: f64,
    /// `"curve"`, `"held"` or `"critical"`, from `curve::Reason`.
    pub reason: String,
}

impl Response {
    pub fn error(e: impl std::fmt::Display) -> Self {
        Response::Error {
            message: e.to_string(),
        }
    }
}
