//! Core library for aerocosm: domain types and logic shared by all binaries.

pub mod aircraft;
pub mod airline;
pub mod asset;
pub mod error;
pub mod geo;
pub mod ground;
pub mod id;
pub mod ops;
pub mod pacing;
pub mod rng;
pub mod scenario;
pub mod schedule;
pub mod sim;
pub mod time;
pub mod traffic;
pub mod units;
pub mod world;

#[cfg(test)]
mod test_data;

pub use aircraft::{AircraftCatalog, AircraftType, WakeCategory};
pub use error::CoreError;
pub use geo::LatLon;
pub use id::Id;
pub use pacing::{Pacer, SimSpeed, WallClock, WallInstant};
pub use rng::{RngSeed, SimRng, StreamId};
pub use scenario::Scenario;
pub use sim::{SimConfig, Simulation, TICK};
pub use time::{CivilDateTime, SimDuration, SimTime};
