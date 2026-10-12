//! Optional hosted identity, sync and RPC access. Analysis and replay never
//! read credentials.
pub mod access;
pub mod client;
pub mod commands;
pub mod contract;
pub mod credentials;
pub mod local;
pub mod privacy;
pub const SERVER_ENV: &str = "EPLYX_URL";
pub const PROJECT_ENV: &str = "EPLYX_PROJECT_ID";
pub const TOKEN_ENV: &str = "EPLYX_TOKEN";
