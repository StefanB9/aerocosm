//! Physical quantities as `f64` newtypes.
//!
//! Approximate values are acceptable for positions, speeds and distances, so
//! these use `f64` (the precision rule's carve-out). Constructors reject
//! non-finite input; arithmetic is left unchecked because overflow to infinity
//! is not reachable with aviation-scale magnitudes.

use std::{
    fmt,
    ops::{Add, Mul, Sub},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::CoreError;

const METERS_PER_FOOT: f64 = 0.3048;
const METERS_PER_NAUTICAL_MILE: f64 = 1852.0;
const SECONDS_PER_HOUR: f64 = 3600.0;

macro_rules! quantity {
    ($(#[$doc:meta])* $name:ident, $unit:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(f64);

        impl $name {
            /// Wraps a finite value, rejecting NaN and infinities.
            pub fn new(value: f64) -> Result<Self> {
                if value.is_finite() {
                    Ok(Self(value))
                } else {
                    Err(CoreError::NonFiniteQuantity { unit: $unit }.into())
                }
            }

            /// Wraps a value the caller has computed from finite inputs.
            // Not every quantity is built internally, so some expansions are unused.
            #[allow(dead_code)]
            pub(crate) const fn from_raw(value: f64) -> Self {
                Self(value)
            }

            /// The raw value in this unit.
            pub const fn value(self) -> f64 {
                self.0
            }
        }

        impl Add for $name {
            type Output = Self;

            fn add(self, rhs: Self) -> Self {
                Self(self.0 + rhs.0)
            }
        }

        impl Sub for $name {
            type Output = Self;

            fn sub(self, rhs: Self) -> Self {
                Self(self.0 - rhs.0)
            }
        }

        impl Mul<f64> for $name {
            type Output = Self;

            fn mul(self, rhs: f64) -> Self {
                Self(self.0 * rhs)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{} {}", self.0, $unit)
            }
        }
    };
}

quantity!(
    /// Length in metres.
    Meters,
    "m"
);
quantity!(
    /// Length in feet, used for altitudes and elevations.
    Feet,
    "ft"
);
quantity!(
    /// Length in nautical miles, used for route distances.
    NauticalMiles,
    "NM"
);
quantity!(
    /// Speed in knots (nautical miles per hour).
    Knots,
    "kt"
);
quantity!(
    /// Speed in metres per second, used internally for kinematics.
    MetersPerSecond,
    "m/s"
);
quantity!(
    /// Angle in degrees. Not normalised unless [`Degrees::normalized`] is
    /// called.
    Degrees,
    "°"
);

impl Meters {
    /// Converts to feet.
    pub const fn to_feet(self) -> Feet {
        Feet(self.0 / METERS_PER_FOOT)
    }

    /// Converts to nautical miles.
    pub const fn to_nautical_miles(self) -> NauticalMiles {
        NauticalMiles(self.0 / METERS_PER_NAUTICAL_MILE)
    }
}

impl Feet {
    /// Converts to metres.
    pub const fn to_meters(self) -> Meters {
        Meters(self.0 * METERS_PER_FOOT)
    }
}

impl NauticalMiles {
    /// Converts to metres.
    pub const fn to_meters(self) -> Meters {
        Meters(self.0 * METERS_PER_NAUTICAL_MILE)
    }
}

impl Knots {
    /// Converts to metres per second.
    pub const fn to_meters_per_second(self) -> MetersPerSecond {
        MetersPerSecond(self.0 * METERS_PER_NAUTICAL_MILE / SECONDS_PER_HOUR)
    }
}

impl MetersPerSecond {
    /// Converts to knots.
    pub const fn to_knots(self) -> Knots {
        Knots(self.0 * SECONDS_PER_HOUR / METERS_PER_NAUTICAL_MILE)
    }
}

impl Degrees {
    /// The angle in radians.
    pub const fn to_radians(self) -> f64 {
        self.0.to_radians()
    }

    /// The same direction expressed in `[0, 360)`.
    #[must_use]
    pub fn normalized(self) -> Self {
        let wrapped = self.0.rem_euclid(360.0);
        // `rem_euclid` rounds tiny negative inputs up to exactly 360.0.
        Self(if wrapped >= 360.0 { 0.0 } else { wrapped })
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::CoreError;

    const TOLERANCE: f64 = 1e-9;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= TOLERANCE * a.abs().max(b.abs()).max(1.0)
    }

    #[test]
    fn test_quantity_new_rejects_non_finite() -> Result<()> {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let error = Meters::new(value).err().context("must reject")?;
            assert_eq!(
                error.downcast_ref::<CoreError>(),
                Some(&CoreError::NonFiniteQuantity { unit: "m" })
            );
        }
        assert!(Knots::new(f64::NAN).is_err());
        assert!(Degrees::new(f64::INFINITY).is_err());
        Ok(())
    }

    #[test]
    fn test_quantity_known_conversions() -> Result<()> {
        assert!(close(Feet::new(1000.0)?.to_meters().value(), 304.8));
        assert!(close(NauticalMiles::new(1.0)?.to_meters().value(), 1852.0));
        assert!(close(
            Knots::new(3600.0)?.to_meters_per_second().value(),
            1852.0
        ));
        assert!(close(
            Degrees::new(180.0)?.to_radians(),
            std::f64::consts::PI
        ));
        Ok(())
    }

    #[test]
    fn test_quantity_arithmetic_and_display() -> Result<()> {
        let sum = Meters::new(1.5)? + Meters::new(2.0)?;
        assert!(close(sum.value(), 3.5));
        assert!(close((sum - Meters::new(0.5)?).value(), 3.0));
        assert!(close((sum * 2.0).value(), 7.0));
        assert!(Meters::new(1.0)? < Meters::new(2.0)?);
        assert_eq!(Meters::new(12.5)?.to_string(), "12.5 m");
        assert_eq!(Knots::new(250.0)?.to_string(), "250 kt");
        Ok(())
    }

    #[test]
    fn test_degrees_normalized_examples() -> Result<()> {
        assert!(close(Degrees::new(-90.0)?.normalized().value(), 270.0));
        assert!(close(Degrees::new(720.0)?.normalized().value(), 0.0));
        assert!(close(Degrees::new(359.5)?.normalized().value(), 359.5));
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_feet_meters_round_trip(v in -1e7_f64..1e7) {
            let feet = Feet::new(v).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert!(close(feet.to_meters().to_feet().value(), v));
        }

        #[test]
        fn test_nautical_miles_meters_round_trip(v in -1e5_f64..1e5) {
            let nm = NauticalMiles::new(v).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert!(close(nm.to_meters().to_nautical_miles().value(), v));
        }

        #[test]
        fn test_knots_meters_per_second_round_trip(v in -1e4_f64..1e4) {
            let kt = Knots::new(v).map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert!(close(kt.to_meters_per_second().to_knots().value(), v));
        }

        #[test]
        fn test_degrees_normalized_in_range_and_congruent(v in -1e6_f64..1e6) {
            let normalized = Degrees::new(v)
                .map_err(|e| TestCaseError::fail(e.to_string()))?
                .normalized()
                .value();
            prop_assert!((0.0..360.0).contains(&normalized));
            let turns = (v - normalized) / 360.0;
            prop_assert!((turns - turns.round()).abs() < 1e-6);
        }
    }
}
