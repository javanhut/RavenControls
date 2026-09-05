//! What the daemon is actually doing to the fans, and the loop that does it.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use raven_hw::curve::{Curve, Reason, Runner};
use raven_hw::guard::Guard;
use raven_hw::ipc::Driving;
use raven_hw::model::{percent_to_raw, Reading, Setting};
use raven_hw::{Hardware, Root};

use crate::state::{SavedCurve, State};

/// How often the curve loop runs.
///
/// Two seconds. Faster does not help -- die temperature is already smoothed by
/// the heatsink, and a fan cannot change speed meaningfully faster than this --
/// and slower makes the response to a sudden load feel broken.
pub const TICK: Duration = Duration::from_secs(2);

/// A curve loop that has not run for this long is presumed wedged, and the
/// fans are handed back. Generous enough that a machine paging heavily under
/// load does not trip it, short enough that nobody's laptop cooks.
pub const WATCHDOG: Duration = Duration::from_secs(20);

struct Driven {
    sensor: String,
    runner: Runner,
    /// The hwmon `pwmN` path, cached so a tick is not a rediscovery.
    duty_attr: String,
    last_percent: f64,
    last_temp: f64,
    last_reason: Reason,
}

pub struct Supervisor {
    hw: Hardware,
    guard: Guard,
    driven: BTreeMap<String, Driven>,
    state_path: std::path::PathBuf,
    /// Seconds since the epoch of the last completed tick, read by the
    /// watchdog thread. An atomic rather than a lock, because the watchdog must
    /// not be able to block on the thing it is watching.
    heartbeat: std::sync::Arc<AtomicU64>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

impl Supervisor {
    pub fn new(root: Root, state_path: std::path::PathBuf) -> Self {
        Self {
            hw: Hardware::discover(root.clone()),
            guard: Guard::new(root),
            driven: BTreeMap::new(),
            state_path,
            heartbeat: std::sync::Arc::new(AtomicU64::new(now_secs())),
        }
    }

    pub fn heartbeat(&self) -> std::sync::Arc<AtomicU64> {
        self.heartbeat.clone()
    }

    pub fn hardware(&self) -> &Hardware {
        &self.hw
    }

    /// Re-apply the curves saved by a previous run.
    ///
    /// A saved curve whose fan is no longer present is kept in the file rather
    /// than dropped: the far more likely explanation is a module that has not
    /// loaded yet, and silently discarding someone's tuning because a driver
    /// probed late would be its own kind of bug.
    pub fn restore_saved(&mut self) {
        let state = State::load(&self.state_path);
        for (knob, saved) in state.curves {
            match self.drive(&knob, &saved.sensor, saved.curve.clone()) {
                Ok(()) => tracing::info!("restored the saved curve on {knob}"),
                Err(e) => tracing::warn!("saved curve for {knob} not applied: {e}"),
            }
        }
    }

    fn save(&self) {
        let state = State {
            curves: self
                .driven
                .iter()
                .map(|(knob, d)| {
                    (
                        knob.clone(),
                        SavedCurve {
                            sensor: d.sensor.clone(),
                            curve: d.runner.curve().clone(),
                        },
                    )
                })
                .collect(),
        };
        if let Err(e) = state.save(&self.state_path) {
            tracing::warn!(
                "could not save curves to {}: {e}",
                self.state_path.display()
            );
        }
    }

    /// The `pwmN` sysfs path behind a knob id, and a check that it is a fan
    /// duty rather than something else that would accept a percentage.
    fn duty_attr(&self, knob_id: &str) -> anyhow::Result<String> {
        let knob = self
            .hw
            .knob(knob_id)
            .ok_or_else(|| anyhow::anyhow!("no control called {knob_id:?} on this machine"))?;
        if knob.role != raven_hw::Role::FanDuty {
            anyhow::bail!("{} is not a fan duty cycle", knob.label);
        }
        if knob.provider != "hwmon" {
            anyhow::bail!(
                "{} cannot be driven from a curve: only hwmon PWM channels take a \
                 continuous duty. Use its modes instead.",
                knob.label
            );
        }
        // `origin` is the absolute path on the live machine; the guard and the
        // tick both want it relative to the root, which for a live root is the
        // same string.
        Ok(knob
            .origin
            .strip_prefix(
                self.hw
                    .root()
                    .base()
                    .to_string_lossy()
                    .trim_end_matches('/'),
            )
            .unwrap_or(&knob.origin)
            .to_string())
    }

    pub fn readings(&self) -> Vec<Reading> {
        self.hw.readings()
    }

    /// Start (or replace) a curve on a fan.
    pub fn drive(&mut self, knob_id: &str, sensor_id: &str, curve: Curve) -> anyhow::Result<()> {
        let duty_attr = self.duty_attr(knob_id)?;
        if !self.hw.readings().iter().any(|r| r.id == sensor_id) {
            anyhow::bail!("no sensor called {sensor_id:?} on this machine");
        }

        if let Some(existing) = self.driven.get_mut(knob_id) {
            // Already ours. Swap the curve without letting go of the channel,
            // so editing a curve does not make the fan blip back to firmware.
            existing.runner.set_curve(curve);
            existing.sensor = sensor_id.to_string();
        } else {
            self.guard.take(&duty_attr)?;
            self.driven.insert(
                knob_id.to_string(),
                Driven {
                    sensor: sensor_id.to_string(),
                    runner: Runner::new(curve),
                    duty_attr,
                    last_percent: 0.0,
                    last_temp: f64::NAN,
                    last_reason: Reason::Curve,
                },
            );
        }
        self.save();
        // Do not wait up to two seconds to act on what someone just asked for.
        self.tick();
        Ok(())
    }

    pub fn release(&mut self, knob_id: &str) -> anyhow::Result<()> {
        let Some(driven) = self.driven.remove(knob_id) else {
            anyhow::bail!("{knob_id} is not being driven");
        };
        self.guard.release(&driven.duty_attr);
        self.save();
        Ok(())
    }

    pub fn release_all(&mut self) {
        self.driven.clear();
        self.guard.release_all();
        self.save();
    }

    /// A one-off set, for everything that is not a curve.
    ///
    /// A manual fan duty goes through the guard too. Someone who drags the
    /// slider to 30% and then kills the window has still handed the fan to us,
    /// and the daemon staying alive is what makes that safe.
    pub fn set(&mut self, knob_id: &str, value: &Setting) -> anyhow::Result<()> {
        if let Ok(duty_attr) = self.duty_attr(knob_id) {
            // Setting a fan by hand ends any curve on it. Two things writing
            // one PWM would fight, and the curve would win, which is not what
            // dragging a slider looks like it should do.
            if self.driven.remove(knob_id).is_some() {
                tracing::info!("{knob_id} was on a curve; the manual value replaces it");
                self.save();
            }
            self.guard.take(&duty_attr)?;
        }
        self.hw.set(knob_id, value)
    }

    /// One pass of the control loop.
    pub fn tick(&mut self) {
        let readings = self.hw.readings();
        let root = self.hw.root().clone();

        for driven in self.driven.values_mut() {
            let Some(sensor) = readings.iter().find(|r| r.id == driven.sensor) else {
                // The sensor went away -- a GPU in runtime suspend, a module
                // unloaded. Without a temperature the curve has nothing to say,
                // and guessing is how a laptop gets cooked, so hand this fan
                // to firmware for as long as that lasts.
                tracing::warn!(
                    "{} disappeared; {} goes back to firmware until it returns",
                    driven.sensor,
                    driven.duty_attr
                );
                let _ = root.write(&format!("{}_enable", driven.duty_attr), "2");
                continue;
            };

            let tick = driven.runner.tick(sensor.value, sensor.critical);
            driven.last_temp = sensor.value;
            driven.last_reason = tick.reason.clone();

            // Firmware takes the channel back across a suspend/resume cycle and
            // after some driver resets, silently. Re-asserting every tick is
            // cheaper than listening for the events that cause it, and catches
            // the ones nobody thought of.
            let enable_attr = format!("{}_enable", driven.duty_attr);
            if root.exists(&enable_attr) && root.read(&enable_attr).as_deref() != Some("1") {
                tracing::info!("{enable_attr} was taken back by firmware; reclaiming");
                if let Err(e) = root.write_verified(&enable_attr, "1") {
                    tracing::warn!("could not reclaim {enable_attr}: {e}");
                    continue;
                }
            }

            // Only write when the byte would change. A PWM write is cheap, but
            // some embedded controllers audibly click on every one.
            let raw = percent_to_raw(tick.percent, raven_hw::fan::hwmon::PWM_RAW_MAX);
            let current = root.read_u32(&driven.duty_attr);
            if current != Some(raw) {
                if let Err(e) = root.write_verified(&driven.duty_attr, &raw.to_string()) {
                    tracing::warn!("{}: {e}", driven.duty_attr);
                    continue;
                }
            }
            if (driven.last_percent - tick.percent).abs() > f64::EPSILON {
                tracing::debug!(
                    "{} -> {:.0}% at {:.1} °C ({:?})",
                    driven.duty_attr,
                    tick.percent,
                    sensor.value,
                    tick.reason
                );
            }
            driven.last_percent = tick.percent;
        }

        self.heartbeat.store(now_secs(), Ordering::Relaxed);
    }

    pub fn driving(&self) -> Vec<Driving> {
        self.driven
            .iter()
            .map(|(knob, d)| Driving {
                knob: knob.clone(),
                sensor: d.sensor.clone(),
                curve: d.runner.curve().clone(),
                percent: d.last_percent,
                temp_c: d.last_temp,
                reason: match d.last_reason {
                    Reason::Curve => "curve",
                    Reason::Held => "held",
                    Reason::Critical => "critical",
                }
                .into(),
            })
            .collect()
    }

    pub fn is_driving_anything(&self) -> bool {
        !self.driven.is_empty()
    }
}

/// A deadline for the next tick that does not drift.
pub fn next_deadline(last: Instant) -> Instant {
    let next = last + TICK;
    let now = Instant::now();
    if next < now {
        now
    } else {
        next
    }
}
