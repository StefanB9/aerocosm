//! Stand limits recomputed from ground assets (plan M6 D149): the same
//! rule as the import's, from the stand points and headings already there.

use aerocosm_core::{ground::GroundDefinition, units::Meters};
use anyhow::{Context, Result};

use super::{
    local::{Plane, Xy},
    stands::wingspan_limits,
};

/// The wingspan of ICAO code C: the narrow-bodies most stands are for, m.
const NARROW_BODY: f64 = 36.0;

/// What re-rating changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rerated {
    /// Stands in all.
    pub stands: usize,
    /// Stands whose limit changed.
    pub changed: usize,
    /// Stands that took a narrow-body (36 m) before.
    pub narrow_before: usize,
    /// Stands that take one now.
    pub narrow_after: usize,
}

/// Sets every stand's wingspan limit in `definition` by the import's rule
/// and reports what changed.
pub fn rerate_stands(definition: &mut GroundDefinition) -> Result<Rerated> {
    let mut report = Rerated::default();
    for airport in &mut definition.airports {
        let Some(origin) = airport.nodes.first().copied() else {
            continue;
        };
        let plane = Plane::new(origin);
        let mut placed = Vec::with_capacity(airport.stands.len());
        for stand in &airport.stands {
            let point = airport
                .nodes
                .get(stand.node as usize)
                .with_context(|| format!("{}: a stand without a node", airport.airport))?;
            let xy: Xy = plane.project(*point);
            placed.push((stand.name, xy, stand.heading.value()));
        }
        let limits = wingspan_limits(&placed);
        for (stand, limit) in airport.stands.iter_mut().zip(limits) {
            let before = stand.max_wingspan.value();
            report.stands += 1;
            report.changed += usize::from((before - limit).abs() > 1e-9);
            report.narrow_before += usize::from(before >= NARROW_BODY);
            report.narrow_after += usize::from(limit >= NARROW_BODY);
            stand.max_wingspan = Meters::new(limit)?;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use aerocosm_core::{
        ground::{AirportGroundDefinition, FORMAT_VERSION, StandDefinition},
        units::Degrees,
    };

    use super::*;

    #[test]
    fn test_rerating_applies_the_import_rule_to_an_asset() -> Result<()> {
        // Five gates 40 m apart along a parallel at 50° north, rated for
        // 24 m by the old rule.
        let east = 40.0 / (111_195.0 * 50.0_f64.to_radians().cos());
        let nodes: Vec<[f64; 2]> = (0..5).map(|i| [50.0, 8.0 + f64::from(i) * east]).collect();
        let stands = (0..5_u32)
            .map(|i| {
                Ok(StandDefinition {
                    name: format!("A{i}").parse()?,
                    node: i,
                    heading: Degrees::new(0.0)?,
                    max_wingspan: Meters::new(24.0)?,
                    area: None,
                    cargo: false,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mut definition = GroundDefinition {
            format_version: FORMAT_VERSION,
            attribution: String::new(),
            airports: vec![AirportGroundDefinition {
                airport: "EDDF".parse()?,
                nodes,
                edges: Vec::new(),
                stands,
                holding_positions: Vec::new(),
                exits: Vec::new(),
                intermediate_holds: Vec::new(),
                aprons: Vec::new(),
            }],
        };
        let report = rerate_stands(&mut definition)?;
        assert_eq!(
            report,
            Rerated {
                stands: 5,
                changed: 5,
                narrow_before: 0,
                narrow_after: 5,
            }
        );
        let limits: Vec<f64> = definition.airports[0]
            .stands
            .iter()
            .map(|s| s.max_wingspan.value())
            .collect();
        assert_eq!(limits, vec![80.0, 65.0, 65.0, 65.0, 80.0]);
        // Again changes nothing.
        assert_eq!(rerate_stands(&mut definition)?.changed, 0);
        Ok(())
    }
}
