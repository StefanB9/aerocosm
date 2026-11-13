//! How runway designators relate to true headings, learnt from runways with
//! surveyed headings (plan M5 D101). A designator is the magnetic heading
//! when it was painted, so true heading minus ten times its number shows the
//! local declination of that time; the mean over the nearest such runways
//! turns a designator into a true heading anywhere, with no magnetic model.

use std::collections::HashMap;

use aerocosm_core::{LatLon, units::Degrees, world::RunwayDesignator};

/// The mean over this many nearest runways: designators round to 10°, so one
/// runway is up to 5° off, many average out.
const NEAREST: usize = 50;
/// Runways are bucketed in cells this many degrees square.
const CELL: f64 = 5.0;
/// Cells around the globe in longitude.
const COLUMNS: i32 = 72;
/// Offsets larger than this are data errors, degrees.
const MAX_OFFSET: f64 = 60.0;

/// The offset a runway end shows: its true heading minus ten times its
/// designator number, in (-180, 180]; `None` for unparseable designators and
/// implausible offsets.
pub(super) fn offset_of(designator: &str, true_heading: f64) -> Option<f64> {
    let designator: RunwayDesignator = designator.parse().ok()?;
    let offset =
        (true_heading - 10.0 * f64::from(designator.number()) + 540.0).rem_euclid(360.0) - 180.0;
    (offset.abs() <= MAX_OFFSET).then_some(offset)
}

/// Offsets of runways with surveyed headings, by place.
#[derive(Debug, Clone, Default)]
pub(super) struct DesignatorOffsets {
    cells: HashMap<(i32, i32), Vec<(LatLon, f64)>>,
    count: usize,
}

fn cell(location: LatLon) -> (i32, i32) {
    // SAFETY: latitudes and longitudes divided by 5° fit an i32 easily.
    #[allow(clippy::cast_possible_truncation)]
    let row = (location.latitude().value() / CELL).floor() as i32;
    #[allow(clippy::cast_possible_truncation)]
    let column = (location.longitude().value() / CELL).floor() as i32;
    (row, column.rem_euclid(COLUMNS))
}

impl DesignatorOffsets {
    /// Offsets learnt from `samples`: where a runway is and its offset.
    pub(super) fn new(samples: Vec<(LatLon, f64)>) -> Self {
        let count = samples.len();
        let mut cells: HashMap<(i32, i32), Vec<(LatLon, f64)>> = HashMap::new();
        for (location, offset) in samples {
            cells
                .entry(cell(location))
                .or_default()
                .push((location, offset));
        }
        Self { cells, count }
    }

    /// The mean offset of the runways nearest `location`, if any were learnt.
    pub(super) fn at(&self, location: LatLon) -> Option<Degrees> {
        if self.count == 0 {
            return None;
        }
        let (row, column) = cell(location);
        let wanted = NEAREST.min(self.count);
        let mut found: Vec<(f64, f64)> = Vec::with_capacity(wanted * 2);
        let mut enough_at = None;
        for ring in 0..=COLUMNS {
            for dr in -ring..=ring {
                for dc in -ring..=ring {
                    if dr.abs() != ring && dc.abs() != ring {
                        continue;
                    }
                    let key = (row + dr, (column + dc).rem_euclid(COLUMNS));
                    for (place, offset) in self.cells.get(&key).into_iter().flatten() {
                        found.push((place.distance_to(location).value(), *offset));
                    }
                }
            }
            // One more ring once there are enough: the nearest may lie just
            // across a cell edge.
            match enough_at {
                Some(at) if ring > at => break,
                None if found.len() >= wanted => enough_at = Some(ring),
                _ => {}
            }
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.truncate(wanted);
        // SAFETY: at most `NEAREST` samples.
        #[allow(clippy::cast_precision_loss)]
        let mean = found.iter().map(|(_, offset)| offset).sum::<f64>() / found.len() as f64;
        Degrees::new(mean).ok()
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::LatLon;
    use anyhow::{Context, Result};

    use super::*;

    /// Sixty runways (more than the mean takes) around `(lat, lon)` whose
    /// true headings run `offset` degrees from their designators, give or
    /// take the designators' rounding.
    fn region(lat: f64, lon: f64, offset: f64) -> Result<Vec<(LatLon, f64)>> {
        let mut samples = Vec::with_capacity(60);
        for k in 0..60_u32 {
            // Pairs at one spot, rounded up and down by the same amount.
            let spread = f64::from(k / 2) / 30.0;
            let rounding = if k % 2 == 0 { 4.0 } else { -4.0 };
            samples.push((LatLon::new(lat + spread, lon - spread)?, offset + rounding));
        }
        Ok(samples)
    }

    #[test]
    fn test_offsets_come_from_the_nearest_runways() -> Result<()> {
        let mut samples = region(50.0, 8.0, 4.0)?;
        samples.extend(region(-34.0, 151.0, 12.0)?);
        let offsets = DesignatorOffsets::new(samples);
        let frankfurt = offsets.at(LatLon::new(50.03, 8.57)?).context("Europe")?;
        assert!((frankfurt.value() - 4.0).abs() < 0.5, "{frankfurt:?}");
        let sydney = offsets
            .at(LatLon::new(-33.9, 151.2)?)
            .context("Australia")?;
        assert!((sydney.value() - 12.0).abs() < 0.5, "{sydney:?}");
        Ok(())
    }

    #[test]
    fn test_without_samples_there_is_no_offset() -> Result<()> {
        assert!(
            DesignatorOffsets::new(Vec::new())
                .at(LatLon::new(0.0, 0.0)?)
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn test_a_sample_is_true_heading_minus_the_designator() -> Result<()> {
        // 07 painted at a true heading of 69.6°: about -0.4°.
        let sample = offset_of("07C", 69.6).context("a sample")?;
        assert!((sample + 0.4).abs() < 1e-9);
        // 36 at 3°: +3° (not -357°); helipads and odd values are no sample.
        assert!((offset_of("36", 3.0).context("north")? - 3.0).abs() < 1e-9);
        assert_eq!(offset_of("H1", 10.0), None);
        assert_eq!(offset_of("09", 200.0), None);
        Ok(())
    }
}
