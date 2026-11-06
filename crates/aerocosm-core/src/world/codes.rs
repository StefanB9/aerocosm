//! Fixed-vocabulary aviation identifiers, stored inline as validated ASCII.

use std::{fmt, str::FromStr};

use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::CoreError;

/// Defines a `Copy` code type holding up to `$cap` ASCII bytes inline.
macro_rules! ascii_code {
    ($(#[$doc:meta])* $name:ident, $cap:literal, $kind:literal, $valid:expr) => {
        $(#[$doc])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name {
            // Zero-padded, so the derived ordering matches string ordering.
            bytes: [u8; $cap],
            len: u8,
        }

        impl $name {
            /// The code as text.
            pub fn as_str(&self) -> &str {
                // SAFETY: construction only admits ASCII, which is valid UTF-8, so the
                // fallback is unreachable.
                std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or_default()
            }
        }

        impl FromStr for $name {
            type Err = anyhow::Error;

            fn from_str(input: &str) -> Result<Self> {
                let raw = input.as_bytes();
                let valid: fn(&[u8]) -> bool = $valid;
                if raw.len() > $cap || !valid(raw) {
                    return Err(CoreError::InvalidCode {
                        kind: $kind,
                        input: input.to_owned(),
                    }
                    .into());
                }
                let mut bytes = [0; $cap];
                bytes[..raw.len()].copy_from_slice(raw);
                // SAFETY: `raw.len() <= $cap`, and every capacity is far below 256.
                #[allow(clippy::cast_possible_truncation)]
                let len = raw.len() as u8;
                Ok(Self { bytes, len })
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({:?})", stringify!($name), self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let text = <&str>::deserialize(deserializer)?;
                text.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

fn all_uppercase(raw: &[u8], len: usize) -> bool {
    raw.len() == len && raw.iter().all(u8::is_ascii_uppercase)
}

ascii_code!(
    /// An aerodrome's identifier: its ICAO code where it has one (`EDDF`),
    /// otherwise the `OurAirports` local identifier (`DE-0062`). 1–8 characters
    /// of `A-Z`, `0-9`, `-`.
    AirportIdent,
    8,
    "airport identifier",
    |raw| {
        !raw.is_empty()
            && raw
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'-')
    }
);

ascii_code!(
    /// An ICAO location indicator, e.g. `EDDF`: four letters `A-Z`.
    IcaoCode,
    4,
    "ICAO airport code",
    |raw| all_uppercase(raw, 4)
);

ascii_code!(
    /// An IATA airport code, e.g. `FRA`: three letters `A-Z`.
    IataCode,
    3,
    "IATA airport code",
    |raw| all_uppercase(raw, 3)
);

ascii_code!(
    /// An ICAO aircraft type designator, e.g. `A20N`: 2–4 characters, a letter
    /// then letters or digits.
    AircraftTypeCode,
    4,
    "aircraft type designator",
    |raw| {
        raw.len() >= 2
            && raw[0].is_ascii_uppercase()
            && raw[1..]
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    }
);

ascii_code!(
    /// An ISO 3166-1 alpha-2 country code, e.g. `DE`.
    CountryCode,
    2,
    "country code",
    |raw| all_uppercase(raw, 2)
);

ascii_code!(
    /// An ICAO airline designator, e.g. `DLH`: three letters `A-Z`.
    AirlineIcao,
    3,
    "ICAO airline designator",
    |raw| all_uppercase(raw, 3)
);

ascii_code!(
    /// An IATA airline designator, e.g. `LH` or `U2`: two letters or digits.
    AirlineIata,
    2,
    "IATA airline designator",
    |raw| raw.len() == 2
        && raw
            .iter()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
);

ascii_code!(
    /// A name on an airport's ground: a stand (`V155`), taxiway (`N7`), runway
    /// (`07C/25C`) or apron (`T1`). 1–16 printable ASCII characters without
    /// leading or trailing spaces.
    GroundName,
    16,
    "ground name",
    |raw| !raw.is_empty()
        && raw.iter().all(|b| (b' '..=b'~').contains(b))
        && raw.first() != Some(&b' ')
        && raw.last() != Some(&b' ')
);

impl From<IcaoCode> for AirportIdent {
    fn from(icao: IcaoCode) -> Self {
        let mut bytes = [0; 8];
        bytes[..4].copy_from_slice(&icao.bytes);
        Self { bytes, len: 4 }
    }
}

#[cfg(test)]
mod tests {
    use anyhow::{Context, Result};
    use proptest::prelude::*;

    use super::*;
    use crate::CoreError;

    fn rejection<T: FromStr<Err = anyhow::Error> + std::fmt::Debug>(
        input: &str,
    ) -> Result<CoreError> {
        let error = input
            .parse::<T>()
            .err()
            .with_context(|| format!("{input:?} must be rejected"))?;
        error
            .downcast::<CoreError>()
            .context("expected a CoreError")
    }

    #[test]
    fn test_icao_code_accepts_four_letters() -> Result<()> {
        let code = "EDDF".parse::<IcaoCode>()?;
        assert_eq!(code.as_str(), "EDDF");
        assert_eq!(code.to_string(), "EDDF");
        Ok(())
    }

    #[test]
    fn test_icao_code_rejects_malformed() -> Result<()> {
        for input in ["", "EDD", "EDDFX", "eddf", "ED1F", "ED F"] {
            assert_eq!(
                rejection::<IcaoCode>(input)?,
                CoreError::InvalidCode {
                    kind: "ICAO airport code",
                    input: input.to_owned()
                }
            );
        }
        Ok(())
    }

    #[test]
    fn test_iata_code_rules() -> Result<()> {
        assert_eq!("FRA".parse::<IataCode>()?.as_str(), "FRA");
        for input in ["FR", "FRAX", "fra", "F1A"] {
            assert!(input.parse::<IataCode>().is_err(), "{input:?}");
        }
        Ok(())
    }

    #[test]
    fn test_airport_ident_accepts_icao_and_local_forms() -> Result<()> {
        assert_eq!("EDDF".parse::<AirportIdent>()?.as_str(), "EDDF");
        assert_eq!("DE-0062".parse::<AirportIdent>()?.as_str(), "DE-0062");
        for input in ["", "TOOLONG99", "de-0062", "DE 0062", "DE_0062"] {
            assert!(input.parse::<AirportIdent>().is_err(), "{input:?}");
        }
        Ok(())
    }

    #[test]
    fn test_aircraft_type_code_rules() -> Result<()> {
        for input in ["A20N", "B38M", "E190", "DH8D", "AT76", "A388", "C1"] {
            assert_eq!(input.parse::<AircraftTypeCode>()?.as_str(), input);
        }
        for input in ["A", "A320X", "320", "a20n", "A-20"] {
            assert!(input.parse::<AircraftTypeCode>().is_err(), "{input:?}");
        }
        Ok(())
    }

    #[test]
    fn test_country_code_rules() -> Result<()> {
        assert_eq!("DE".parse::<CountryCode>()?.as_str(), "DE");
        for input in ["D", "DEU", "de", "D1"] {
            assert!(input.parse::<CountryCode>().is_err(), "{input:?}");
        }
        Ok(())
    }

    #[test]
    fn test_codes_serde_as_strings() -> Result<()> {
        let code = "EDDM".parse::<IcaoCode>()?;
        let json = serde_json::to_string(&code).context("serialise")?;
        assert_eq!(json, "\"EDDM\"");
        assert_eq!(
            serde_json::from_str::<IcaoCode>(&json).context("deserialise")?,
            code
        );
        assert!(serde_json::from_str::<IcaoCode>("\"eddm\"").is_err());
        Ok(())
    }

    #[test]
    fn test_codes_order_and_hash_by_text() -> Result<()> {
        let a = "DE-0062".parse::<AirportIdent>()?;
        let b = "EDDF".parse::<AirportIdent>()?;
        assert!(a < b);
        let set: std::collections::HashSet<_> = [a, b, "EDDF".parse::<AirportIdent>()?].into();
        assert_eq!(set.len(), 2);
        Ok(())
    }

    #[test]
    fn test_icao_code_converts_to_airport_ident() -> Result<()> {
        let icao = "EDDH".parse::<IcaoCode>()?;
        assert_eq!(AirportIdent::from(icao).as_str(), "EDDH");
        Ok(())
    }

    #[test]
    fn test_ground_name_rules() -> Result<()> {
        assert_eq!("V 117".parse::<GroundName>()?.as_str(), "V 117");
        assert_eq!("07C/25C".parse::<GroundName>()?.to_string(), "07C/25C");
        for bad in [
            "",
            " A1",
            "A1 ",
            "Vorfeld S\u{fc}d",
            "ABCDEFGHIJKLMNOPQ",
            "A\tB",
        ] {
            assert!(matches!(
                rejection::<GroundName>(bad)?,
                CoreError::InvalidCode {
                    kind: "ground name",
                    ..
                }
            ));
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(1000))]

        #[test]
        fn test_airport_ident_round_trip(text in "[A-Z0-9-]{1,8}") {
            let ident = text.parse::<AirportIdent>()
                .map_err(|e| TestCaseError::fail(e.to_string()))?;
            prop_assert_eq!(ident.as_str(), text.as_str());
        }

        #[test]
        fn test_icao_code_accepts_exactly_valid(text in "\\PC{0,6}") {
            let valid = text.len() == 4 && text.bytes().all(|b| b.is_ascii_uppercase());
            prop_assert_eq!(text.parse::<IcaoCode>().is_ok(), valid);
        }
    }
}
