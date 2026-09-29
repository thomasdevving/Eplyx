//! Token Migration V1: the first fully productized transition type.
//!
//! A proposed migration is a declarative [`spec::TokenMigrationV1`] carried inside a
//! versioned [`crate::change::ChangeSpec`]. Eplyx rehearses it against captured or
//! explicitly synthetic state with the operator's exact candidate program bytes in the
//! existing LiteSVM backend, then reports what works, what breaks, who is affected and
//! why. Observed, synthetic and derived state never mix; a rehearsal never becomes a
//! mainnet transaction, and a candidate mechanism never becomes an official issuer
//! migration.
pub mod account;
pub mod adapter;
pub mod authority;
pub mod capture;
pub mod derive;
pub mod economics;
pub mod execute;
pub mod extensions;
pub mod fixture;
pub mod frozen;
pub mod input;
pub mod invariants;
pub mod order;
pub mod order_store;
pub mod pipeline;
pub mod planner;
pub mod rehearsal;
pub mod report;
pub mod search;
pub mod spec;
pub mod stress;
pub mod unsigned;
pub mod world;

pub mod gate;
pub mod population;
pub mod population_types;
pub mod requirements;

pub mod authority_resolution;
pub mod coherence;
pub mod current;
pub mod current_classify;
pub mod current_select;
pub mod observed_search;

#[cfg(test)]
mod current_tests;

pub mod error;
