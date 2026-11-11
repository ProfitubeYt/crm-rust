//! Everything the desktop client knows that is not pixels: server access,
//! session handling, the local cache, filtering and display formatting.
//!
//! No UI type may appear here. The CRM is expected to reuse this crate.

pub mod api;
pub mod columns;
pub mod config;
pub mod entitlement;
pub mod export;
pub mod format;
pub mod links;
pub mod model;
pub mod query;
pub mod secret;
pub mod store;
pub mod sync;
pub mod updates;

pub use reqwest;
