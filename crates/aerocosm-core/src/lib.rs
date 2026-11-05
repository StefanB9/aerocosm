//! Core library for aerocosm: domain types and logic shared by all binaries.

pub mod aircraft;
pub mod airline;
pub mod error;
pub mod geo;
pub mod id;
pub mod pacing;
pub mod rng;
pub mod sim;
pub mod time;
pub mod traffic;
pub mod units;
pub mod world;

pub use aircraft::{AircraftCatalog, AircraftType, WakeCategory};
pub use error::CoreError;
pub use geo::LatLon;
pub use id::Id;
pub use pacing::{Pacer, SimSpeed, WallClock, WallInstant};
pub use rng::{RngSeed, SimRng, StreamId};
pub use sim::{SimConfig, Simulation, TICK};
pub use time::{CivilDateTime, SimDuration, SimTime};
