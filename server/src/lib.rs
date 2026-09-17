//! Hosted Eplyx CI API, as a library.
//!
//! The binary is a thin wrapper over this so the HTTP surface and the run
//! lifecycle can be driven from integration tests directly, without a port, a
//! process or a sleep. What those tests exercise is then the same router the
//! service serves, not a re-creation of it.

pub mod api;
pub mod config;
pub mod project;
pub mod registry;
pub mod storage;
pub mod worker;
