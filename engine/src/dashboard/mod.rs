//! Milestone 16: a read-only local developer dashboard over one project's
//! `.eplyx/` run store. The engine artifacts stay the single source of
//! analytical truth; this module selects, counts and compares their fields,
//! reuses the engine gate evaluator for policy views, and serves them on
//! loopback only. It never replays, executes, edits or uploads anything.
pub mod assets;
pub mod migration;
pub mod server;
pub mod store;
pub mod view;

pub use server::Dashboard;
