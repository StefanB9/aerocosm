//! Core library for aerocosm: domain types and logic shared by all binaries.

pub mod error;
pub mod geo;
pub mod id;
pub mod rng;
pub mod time;
pub mod units;

pub use error::CoreError;
pub use geo::LatLon;
pub use id::Id;
pub use rng::{RngSeed, SimRng, StreamId};
pub use time::{CivilDateTime, SimDuration, SimTime};
