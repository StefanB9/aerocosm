//! The static world: airports and runways.

mod airport;
pub mod codes;
mod designator;
mod runway;

pub use airport::{Airport, AirportDefinition, AirportKind};
pub use designator::{RunwayDesignator, RunwaySide};
pub use runway::{GeometrySource, Runway, RunwayDefinition, RunwayEnd, Surface};
