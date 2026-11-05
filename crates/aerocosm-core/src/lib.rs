//! Core library for aerocosm: domain types and logic shared by all binaries.

pub mod error;
pub mod id;
pub mod time;

pub use error::CoreError;
pub use id::Id;
pub use time::{CivilDateTime, SimDuration, SimTime};
