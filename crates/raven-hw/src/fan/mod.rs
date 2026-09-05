//! Fan control, in order of how much of the world each path covers.
//!
//! `hwmon` first, because it is the only one that gives a duty cycle a curve
//! can drive. Everything below it is a coarser knob offered by machines that
//! have nothing finer.

pub mod cooling;
pub mod hwmon;
pub mod profile;
pub mod vendor;
