//! The daemon's innards, as a library so they can be tested without root.
//!
//! `main.rs` is the socket, the signal handlers and the privilege check --
//! none of which can run in a test. Everything that decides *what* to do lives
//! here and runs against a captured tree like the rest of RavenControls.

pub mod protocol;
pub mod state;
pub mod supervisor;
