//! Temperature-to-duty curves.
//!
//! No I/O here at all, which is what lets the awkward parts -- hysteresis, the
//! stall floor, the critical override -- be tested in milliseconds rather than
//! by listening to a laptop.
//!
//! Three behaviours that a naive `interpolate(temp)` gets wrong, and that a
//! person notices within a minute of using it:
//!
//!   * **Oscillation.** A CPU sitting on a curve knee ticks between 61 and 62
//!     degrees forever, and an honest interpolation turns that into a fan that
//!     audibly surges once a second. The fix is asymmetric: follow temperature
//!     up immediately, follow it down only after it has fallen by the
//!     hysteresis band.
//!   * **Stall.** Most fans will not start from rest below roughly a fifth of
//!     full duty, and a fan commanded to 8% does not spin slowly -- it sits
//!     still while the curve believes it is cooling. So there is a floor, and
//!     it applies to every non-zero output.
//!   * **The panic case.** If a sensor passes the temperature its own driver
//!     calls critical, the curve is no longer the authority. Full duty, and say
//!     why.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub temp_c: f64,
    pub percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    /// Sorted by temperature on construction. At least one point.
    pub points: Vec<Point>,
    /// How far the temperature must fall before the duty is allowed to.
    pub hysteresis_c: f64,
    /// The lowest duty this fan will actually turn at. Outputs between zero and
    /// this are raised to it; a curve that asks for zero still gets zero, so a
    /// fan can stop.
    pub min_percent: f64,
}

impl Default for Curve {
    /// A quiet-until-it-matters shape that is safe on hardware nobody here has
    /// measured: silent to 45, audible only past 65, everything by 85.
    fn default() -> Self {
        Self {
            points: vec![
                Point {
                    temp_c: 40.0,
                    percent: 0.0,
                },
                Point {
                    temp_c: 55.0,
                    percent: 30.0,
                },
                Point {
                    temp_c: 70.0,
                    percent: 55.0,
                },
                Point {
                    temp_c: 80.0,
                    percent: 80.0,
                },
                Point {
                    temp_c: 90.0,
                    percent: 100.0,
                },
            ],
            hysteresis_c: 3.0,
            min_percent: 20.0,
        }
    }
}

impl Curve {
    pub fn new(mut points: Vec<Point>) -> anyhow::Result<Self> {
        if points.is_empty() {
            anyhow::bail!("a fan curve needs at least one point");
        }
        points.sort_by(|a, b| a.temp_c.total_cmp(&b.temp_c));
        if points
            .iter()
            .any(|p| !p.temp_c.is_finite() || !p.percent.is_finite())
        {
            anyhow::bail!("a fan curve point must be a real temperature and percentage");
        }
        if let Some(p) = points.iter().find(|p| !(0.0..=100.0).contains(&p.percent)) {
            anyhow::bail!("{}% is not a duty cycle", p.percent);
        }
        Ok(Self {
            points,
            ..Default::default()
        })
    }

    /// The duty this curve asks for at `temp_c`, before hysteresis and floor.
    ///
    /// Flat outside the outermost points rather than extrapolated: a curve
    /// drawn between 40 and 90 degrees says nothing about 120, and a linear
    /// extension would confidently answer 160%.
    pub fn raw_duty_at(&self, temp_c: f64) -> f64 {
        let first = self.points[0];
        let last = self.points[self.points.len() - 1];
        if temp_c <= first.temp_c {
            return first.percent;
        }
        if temp_c >= last.temp_c {
            return last.percent;
        }
        for pair in self.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if temp_c >= a.temp_c && temp_c <= b.temp_c {
                let span = b.temp_c - a.temp_c;
                if span <= 0.0 {
                    return b.percent;
                }
                let t = (temp_c - a.temp_c) / span;
                return a.percent + t * (b.percent - a.percent);
            }
        }
        last.percent
    }

    /// Raise a non-zero duty to the stall floor. Zero stays zero.
    pub fn apply_floor(&self, percent: f64) -> f64 {
        if percent <= 0.0 {
            0.0
        } else {
            percent.max(self.min_percent).min(100.0)
        }
    }
}

/// What one tick decided, and why. The reason is shown in the window, so a fan
/// that is loud has a legible explanation rather than a number.
#[derive(Debug, Clone, PartialEq)]
pub struct Tick {
    pub percent: f64,
    pub reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// Following the curve at the temperature shown.
    Curve,
    /// Temperature fell, but not by more than the hysteresis band, so the duty
    /// was held where it was.
    Held,
    /// A sensor is at or past its driver's critical temperature.
    Critical,
}

/// A curve plus the memory that hysteresis needs.
#[derive(Debug, Clone)]
pub struct Runner {
    curve: Curve,
    /// The temperature the current duty was chosen for. Not the last observed
    /// temperature -- that is what makes the band work.
    reference_c: Option<f64>,
    last_percent: f64,
}

impl Runner {
    pub fn new(curve: Curve) -> Self {
        Self {
            curve,
            reference_c: None,
            last_percent: 0.0,
        }
    }

    pub fn curve(&self) -> &Curve {
        &self.curve
    }

    /// Replace the curve, keeping no memory: a curve the person just edited
    /// should take effect at the next tick, not once the temperature has
    /// wandered outside the old band.
    pub fn set_curve(&mut self, curve: Curve) {
        self.curve = curve;
        self.reference_c = None;
    }

    /// One control step.
    ///
    /// `critical_c` is the driver's own critical temperature for this sensor,
    /// where it publishes one. It is not a curve point and is not editable:
    /// the point of it is to be the thing that still works when the curve is
    /// wrong.
    pub fn tick(&mut self, temp_c: f64, critical_c: Option<f64>) -> Tick {
        if let Some(critical) = critical_c {
            if temp_c >= critical {
                self.reference_c = Some(temp_c);
                self.last_percent = 100.0;
                return Tick {
                    percent: 100.0,
                    reason: Reason::Critical,
                };
            }
        }

        let reference = match self.reference_c {
            // Up is immediate.
            Some(previous) if temp_c > previous => temp_c,
            // Down only once it has fallen out of the band.
            Some(previous) if temp_c < previous - self.curve.hysteresis_c => temp_c,
            Some(previous) => previous,
            None => temp_c,
        };
        let held = self.reference_c == Some(reference) && reference != temp_c;
        self.reference_c = Some(reference);

        let percent = self.curve.apply_floor(self.curve.raw_duty_at(reference));
        self.last_percent = percent;
        Tick {
            percent,
            reason: if held { Reason::Held } else { Reason::Curve },
        }
    }

    pub fn last_percent(&self) -> f64 {
        self.last_percent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve() -> Curve {
        Curve {
            points: vec![
                Point {
                    temp_c: 40.0,
                    percent: 0.0,
                },
                Point {
                    temp_c: 60.0,
                    percent: 50.0,
                },
                Point {
                    temp_c: 80.0,
                    percent: 100.0,
                },
            ],
            hysteresis_c: 3.0,
            min_percent: 20.0,
        }
    }

    #[test]
    fn interpolation_is_linear_between_points() {
        let c = curve();
        assert!((c.raw_duty_at(50.0) - 25.0).abs() < 1e-9);
        assert!((c.raw_duty_at(70.0) - 75.0).abs() < 1e-9);
        assert!((c.raw_duty_at(60.0) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn the_curve_is_flat_outside_its_endpoints_rather_than_extrapolated() {
        let c = curve();
        assert_eq!(c.raw_duty_at(10.0), 0.0);
        assert_eq!(c.raw_duty_at(-40.0), 0.0);
        // The one that matters: a runaway sensor must not produce 160%.
        assert_eq!(c.raw_duty_at(120.0), 100.0);
        assert_eq!(c.raw_duty_at(1000.0), 100.0);
    }

    #[test]
    fn a_duty_below_the_stall_floor_is_raised_but_zero_stays_zero() {
        let c = curve();
        // 42 degrees interpolates to 5%, which would leave the fan stationary
        // while the curve believed it was cooling.
        assert!(c.raw_duty_at(42.0) < 20.0);
        assert_eq!(c.apply_floor(c.raw_duty_at(42.0)), 20.0);
        // Below the first point the curve genuinely asks for a stopped fan.
        assert_eq!(c.apply_floor(c.raw_duty_at(30.0)), 0.0);
    }

    #[test]
    fn temperature_rises_are_followed_immediately() {
        let mut r = Runner::new(curve());
        assert_eq!(r.tick(60.0, None).percent, 50.0);
        let t = r.tick(70.0, None);
        assert_eq!(t.percent, 75.0);
        assert_eq!(t.reason, Reason::Curve);
    }

    #[test]
    fn a_small_drop_holds_the_duty_instead_of_surging() {
        let mut r = Runner::new(curve());
        r.tick(70.0, None);
        // The one-degree wobble that makes a naive controller audible.
        for temp in [69.0, 68.5, 69.5, 68.0] {
            let t = r.tick(temp, None);
            assert_eq!(t.percent, 75.0, "{temp} moved the fan");
            assert_eq!(t.reason, Reason::Held);
        }
    }

    #[test]
    fn a_real_drop_past_the_band_lets_the_fan_come_down() {
        let mut r = Runner::new(curve());
        r.tick(70.0, None);
        let t = r.tick(66.0, None);
        assert!((t.percent - 65.0).abs() < 1e-9, "{}", t.percent);
        assert_eq!(t.reason, Reason::Curve);
    }

    #[test]
    fn the_critical_temperature_overrides_the_curve_entirely() {
        let mut r = Runner::new(Curve {
            points: vec![Point {
                temp_c: 0.0,
                percent: 0.0,
            }],
            hysteresis_c: 3.0,
            min_percent: 20.0,
        });
        // A curve that says "never spin", which is exactly the curve someone
        // draws at 2am and then forgets about.
        assert_eq!(r.tick(50.0, Some(100.0)).percent, 0.0);
        let t = r.tick(100.0, Some(100.0));
        assert_eq!(t.percent, 100.0);
        assert_eq!(t.reason, Reason::Critical);
    }

    #[test]
    fn editing_the_curve_takes_effect_at_the_next_tick() {
        let mut r = Runner::new(curve());
        r.tick(70.0, None);
        r.set_curve(Curve {
            points: vec![Point {
                temp_c: 0.0,
                percent: 100.0,
            }],
            hysteresis_c: 3.0,
            min_percent: 20.0,
        });
        assert_eq!(r.tick(69.0, None).percent, 100.0);
    }

    #[test]
    fn points_are_sorted_and_nonsense_is_refused() {
        let c = Curve::new(vec![
            Point {
                temp_c: 80.0,
                percent: 100.0,
            },
            Point {
                temp_c: 40.0,
                percent: 0.0,
            },
        ])
        .unwrap();
        assert_eq!(c.points[0].temp_c, 40.0);
        assert!(Curve::new(vec![]).is_err());
        assert!(Curve::new(vec![Point {
            temp_c: 40.0,
            percent: 140.0
        }])
        .is_err());
        assert!(Curve::new(vec![Point {
            temp_c: f64::NAN,
            percent: 10.0
        }])
        .is_err());
    }

    #[test]
    fn the_default_curve_is_silent_when_cool_and_maximal_when_hot() {
        let c = Curve::default();
        assert_eq!(c.apply_floor(c.raw_duty_at(35.0)), 0.0);
        assert_eq!(c.apply_floor(c.raw_duty_at(95.0)), 100.0);
    }
}
