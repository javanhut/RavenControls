//! Where the window's writes actually go.
//!
//! Two backends, and which one is in use changes what the window will let you
//! do:
//!
//! **The daemon**, when `/run/raven-controls/ctl` is there. Everything works,
//! including curves, because something is alive to hand the fans back.
//!
//! **Straight to sysfs**, when it is not. The keyboard light works if a udev
//! rule has been installed, and coarse thermal modes work if they are writable
//! -- those are firmware-managed and safe to leave set. Manual fan duty does
//! *not* work, and that is deliberate rather than a limitation: a window that
//! writes `pwm1_enable=1` and is then closed leaves a fan pinned wherever it
//! was, with nothing left running to notice. Refusing is the only honest
//! behaviour, and the window says why and what to start.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use raven_hw::curve::Curve;
use raven_hw::ipc::{Driving, Request, Response, SOCKET};
use raven_hw::model::{Knob, Reading, Role, Setting};
use raven_hw::{Hardware, Root};

/// One read of the machine, everything the window draws from.
pub struct Snapshot {
    pub knobs: Vec<Knob>,
    pub readings: Vec<Reading>,
    pub driving: Vec<Driving>,
    pub diagnosis: Vec<String>,
    pub machine: String,
    pub via: Via,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Daemon,
    Sysfs,
}

impl Snapshot {
    pub fn knobs_for(&self, role: Role) -> Vec<&Knob> {
        self.knobs.iter().filter(|k| k.role == role).collect()
    }

    pub fn driving(&self, knob_id: &str) -> Option<&Driving> {
        self.driving.iter().find(|d| d.knob == knob_id)
    }

    /// Temperature sensors, which is what a curve can be driven from.
    pub fn temperatures(&self) -> Vec<&Reading> {
        self.readings
            .iter()
            .filter(|r| r.unit == raven_hw::Unit::Celsius)
            .collect()
    }
}

pub struct Client {
    /// Kept for the sysfs path and for reading, even when the daemon is in
    /// charge of writing: a read does not need root and a round trip per
    /// refresh is worth avoiding.
    hw: Hardware,
    socket: String,
}

impl Client {
    pub fn new() -> Self {
        Self {
            hw: Hardware::discover(Root::system()),
            socket: std::env::var("RAVEN_CONTROLS_SOCKET").unwrap_or_else(|_| SOCKET.into()),
        }
    }

    fn connect(&self) -> Option<UnixStream> {
        let stream = UnixStream::connect(&self.socket).ok()?;
        // A daemon that has wedged must not wedge the window with it.
        let timeout = Some(Duration::from_secs(3));
        stream.set_read_timeout(timeout).ok()?;
        stream.set_write_timeout(timeout).ok()?;
        Some(stream)
    }

    pub fn daemon_available(&self) -> bool {
        self.connect().is_some()
    }

    fn request(&self, request: &Request) -> anyhow::Result<Response> {
        let stream = self
            .connect()
            .ok_or_else(|| anyhow::anyhow!("raven-controlsd is not running"))?;
        let mut writer = stream.try_clone()?;
        writeln!(writer, "{}", serde_json::to_string(request)?)?;
        writer.flush()?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line)?;
        Ok(serde_json::from_str(&line)?)
    }

    pub fn snapshot(&self) -> Snapshot {
        if let Ok(Response::State {
            knobs,
            readings,
            driving,
            diagnosis,
            machine,
        }) = self.request(&Request::Probe)
        {
            return Snapshot {
                knobs,
                readings,
                driving,
                diagnosis,
                machine,
                via: Via::Daemon,
            };
        }

        // No daemon. Everything is still readable -- sysfs attributes are
        // world-readable -- so the window is fully populated and only writes
        // are limited.
        let knobs = self.hw.knobs();
        let diagnosis = if knobs
            .iter()
            .any(|k| k.role == Role::FanDuty || k.role == Role::ThermalProfile)
        {
            Vec::new()
        } else {
            raven_hw::diagnose::report(self.hw.root())
                .suggestions
                .iter()
                .map(|s| s.line())
                .collect()
        };
        Snapshot {
            knobs,
            readings: self.hw.readings(),
            driving: Vec::new(),
            diagnosis,
            machine: raven_hw::machine(self.hw.root()),
            via: Via::Sysfs,
        }
    }

    pub fn set(&self, knob: &Knob, value: &Setting) -> anyhow::Result<()> {
        match self.request(&Request::Set {
            knob: knob.id.clone(),
            value: value.clone(),
        }) {
            Ok(Response::Ok) => return Ok(()),
            Ok(Response::Error { message }) => anyhow::bail!("{message}"),
            Ok(Response::State { .. }) => anyhow::bail!("the daemon answered a set with a state"),
            // No daemon: fall through to the direct path.
            Err(_) => {}
        }

        if let Some(refusal) = unsafe_without_daemon(knob, value) {
            anyhow::bail!("{refusal}");
        }
        self.hw.set(&knob.id, value)
    }

    pub fn drive(&self, knob: &str, sensor: &str, curve: Curve) -> anyhow::Result<()> {
        match self.request(&Request::Curve {
            knob: knob.into(),
            sensor: sensor.into(),
            curve,
        })? {
            Response::Ok => Ok(()),
            Response::Error { message } => anyhow::bail!("{message}"),
            Response::State { .. } => anyhow::bail!("the daemon answered a curve with a state"),
        }
    }

    pub fn release(&self, knob: &str) -> anyhow::Result<()> {
        match self.request(&Request::Release { knob: knob.into() })? {
            Response::Ok => Ok(()),
            Response::Error { message } => anyhow::bail!("{message}"),
            Response::State { .. } => anyhow::bail!("the daemon answered a release with a state"),
        }
    }
}

/// The one thing the window will not do without a daemon behind it, and why.
///
/// Returns the message to show, or `None` when the write is safe to make
/// directly. Split out and tested rather than left inline: getting this wrong
/// in the permissive direction means a fan pinned at 0% on a machine whose
/// owner has closed the window and gone home.
pub fn unsafe_without_daemon(knob: &Knob, value: &Setting) -> Option<String> {
    let taking_control = match (knob.role, value) {
        (Role::FanDuty, _) => true,
        (Role::FanControlMode, Setting::Mode { name }) => name != "Automatic",
        _ => false,
    };
    taking_control.then(|| {
        format!(
            "Taking manual control of {} needs raven-controlsd, which is what hands the \
             fan back to firmware when this window closes or crashes. Start it with:\n\
             \n    sudo raven-controlsd\n\n\
             Firmware-managed modes still work without it.",
            knob.label
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raven_hw::model::Domain;

    fn knob(role: Role) -> Knob {
        Knob {
            id: "test".into(),
            label: "Fan".into(),
            role,
            domain: Domain::Percent { raw_max: 255 },
            value: Setting::Percent { percent: 0.0 },
            writable: true,
            provider: "hwmon".into(),
            origin: "/sys/class/hwmon/hwmon0/pwm1".into(),
        }
    }

    #[test]
    fn a_manual_duty_is_refused_when_nothing_would_hand_the_fan_back() {
        let refusal =
            unsafe_without_daemon(&knob(Role::FanDuty), &Setting::Percent { percent: 30.0 });
        assert!(refusal.unwrap().contains("raven-controlsd"));
    }

    #[test]
    fn switching_a_fan_to_manual_is_refused_for_the_same_reason() {
        for mode in ["Manual", "Full speed"] {
            assert!(
                unsafe_without_daemon(
                    &knob(Role::FanControlMode),
                    &Setting::Mode { name: mode.into() },
                )
                .is_some(),
                "{mode} was allowed"
            );
        }
    }

    #[test]
    fn handing_a_fan_back_to_firmware_never_needs_the_daemon() {
        // The direction that makes a machine safer must always be available,
        // including when the daemon is the thing that has just died.
        assert!(unsafe_without_daemon(
            &knob(Role::FanControlMode),
            &Setting::Mode {
                name: "Automatic".into()
            },
        )
        .is_none());
    }

    #[test]
    fn the_keyboard_light_and_thermal_profiles_are_never_restricted() {
        assert!(unsafe_without_daemon(
            &knob(Role::KeyboardBacklight),
            &Setting::Percent { percent: 100.0 },
        )
        .is_none());
        assert!(unsafe_without_daemon(
            &knob(Role::ThermalProfile),
            &Setting::Mode {
                name: "Performance".into()
            },
        )
        .is_none());
    }
}
