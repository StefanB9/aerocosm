//! Brand colours.

use std::{fmt, str::FromStr};

use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::CoreError;

/// A brand colour such as `#0A1D3D`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Colour([u8; 3]);

impl Colour {
    /// The colour with red, green and blue bytes `rgb`.
    pub const fn from_rgb(rgb: [u8; 3]) -> Self {
        Self(rgb)
    }

    /// Red, green and blue bytes.
    pub const fn rgb(self) -> [u8; 3] {
        self.0
    }
}

impl FromStr for Colour {
    type Err = anyhow::Error;

    fn from_str(input: &str) -> Result<Self> {
        let invalid = || CoreError::InvalidCode {
            kind: "colour",
            input: input.to_owned(),
        };
        let hex = input.strip_prefix('#').ok_or_else(invalid)?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid().into());
        }
        let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).map_err(|_| invalid());
        Ok(Self([channel(0)?, channel(2)?, channel(4)?]))
    }
}

impl fmt::Display for Colour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [r, g, b] = self.0;
        write!(f, "#{r:02X}{g:02X}{b:02X}")
    }
}

impl Serialize for Colour {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Colour {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <&str>::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_colour_parse_and_display() -> Result<()> {
        let colour: Colour = "#0A1D3D".parse()?;
        assert_eq!(colour.rgb(), [0x0A, 0x1D, 0x3D]);
        assert_eq!(colour.to_string(), "#0A1D3D");
        for bad in ["0A1D3D", "#0A1D3", "#GG0000", "#0A1D3D0"] {
            assert!(bad.parse::<Colour>().is_err(), "{bad}");
        }
        Ok(())
    }
}
