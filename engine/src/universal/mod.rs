//! Versioned, protocol-independent replay inputs and immutable evidence.
//!
//! This module deliberately contains no protocol IDs. A record is an
//! observation about a transaction, not a request to run a particular adapter.

pub mod bundle;
pub mod evidence;
pub mod execution;
pub mod fidelity;
pub mod model;
pub mod pipeline;
pub mod resolver;
