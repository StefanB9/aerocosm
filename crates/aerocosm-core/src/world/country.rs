//! Countries: population and economic output, for the traffic model's
//! fallback (plan M5 D109).

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::codes::CountryCode;
use crate::CoreError;

/// A country as stored in the world file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountryDefinition {
    /// ISO 3166-1 alpha-2 code.
    pub code: CountryCode,
    /// Population estimate.
    pub population: u64,
    /// Gross domestic product, millions of US dollars.
    pub gdp_musd: u64,
}

/// A validated country.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Country {
    definition: CountryDefinition,
}

impl Country {
    /// Validates that population and output are positive.
    pub fn new(definition: CountryDefinition) -> Result<Self> {
        if definition.population == 0 || definition.gdp_musd == 0 {
            return Err(CoreError::InvalidCountry {
                code: definition.code.to_string(),
                reason: "population and GDP must be positive",
            }
            .into());
        }
        Ok(Self { definition })
    }

    /// ISO 3166-1 alpha-2 code.
    pub const fn code(&self) -> CountryCode {
        self.definition.code
    }

    /// Population estimate.
    pub const fn population(&self) -> u64 {
        self.definition.population
    }

    /// Gross domestic product per head, US dollars.
    pub fn gdp_per_capita(&self) -> f64 {
        // Integer inputs far below 2^53: the conversion is exact enough.
        #[allow(clippy::cast_precision_loss)]
        let (gdp, people) = (
            self.definition.gdp_musd as f64,
            self.definition.population as f64,
        );
        gdp * 1e6 / people
    }

    /// The definition this country was built from.
    pub const fn definition(&self) -> &CountryDefinition {
        &self.definition
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use anyhow::{Context, Result};

    use super::*;

    pub(crate) fn germany() -> Result<CountryDefinition> {
        Ok(CountryDefinition {
            code: "DE".parse()?,
            population: 83_132_799,
            gdp_musd: 3_861_123,
        })
    }

    #[test]
    fn test_country_gdp_per_capita() -> Result<()> {
        let germany = Country::new(germany()?)?;
        assert_eq!(germany.code().as_str(), "DE");
        assert_eq!(germany.population(), 83_132_799);
        assert!((germany.gdp_per_capita() - 46_445.0).abs() < 1.0);
        Ok(())
    }

    #[test]
    fn test_country_rejects_zero_figures() -> Result<()> {
        for broken in [
            CountryDefinition {
                population: 0,
                ..germany()?
            },
            CountryDefinition {
                gdp_musd: 0,
                ..germany()?
            },
        ] {
            let error = Country::new(broken).err().context("must be rejected")?;
            assert_eq!(
                error.downcast_ref::<CoreError>(),
                Some(&CoreError::InvalidCountry {
                    code: "DE".into(),
                    reason: "population and GDP must be positive"
                })
            );
        }
        Ok(())
    }
}
