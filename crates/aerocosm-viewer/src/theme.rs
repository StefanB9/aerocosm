//! The viewer's palette: a minimalist dark map with one warm accent.

use bevy::color::Color;

/// Open water and the window background.
pub(crate) const SEA: Color = Color::srgb(0.043, 0.071, 0.125);
/// Land of neighbouring countries.
pub(crate) const LAND: Color = Color::srgb(0.082, 0.110, 0.173);
/// Land of the simulated country.
pub(crate) const LAND_HOME: Color = Color::srgb(0.106, 0.141, 0.216);
/// Lakes.
pub(crate) const LAKE: Color = Color::srgb(0.055, 0.090, 0.157);
/// International borders.
pub(crate) const BORDER: Color = Color::srgb(0.300, 0.370, 0.500);
/// State borders.
pub(crate) const STATE_BORDER: Color = Color::srgb(0.170, 0.215, 0.300);
/// Rivers.
pub(crate) const RIVER: Color = Color::srgb(0.110, 0.200, 0.330);
/// The accent: major airports, selection, active controls.
pub(crate) const ACCENT: Color = Color::srgb(0.961, 0.722, 0.255);
/// Regional airports.
pub(crate) const REGIONAL: Color = Color::srgb(0.910, 0.890, 0.830);
/// Airfields.
pub(crate) const AIRFIELD: Color = Color::srgb(0.540, 0.580, 0.660);
/// Paved runways.
pub(crate) const RUNWAY_PAVED: Color = Color::srgb(0.780, 0.800, 0.840);
/// Grass and unpaved runways.
pub(crate) const RUNWAY_SOFT: Color = Color::srgb(0.310, 0.480, 0.350);
/// Aprons: a quiet surface between land and runways.
pub(crate) const APRON: Color = Color::srgb(0.155, 0.190, 0.262);
/// Taxiway and taxilane centrelines.
pub(crate) const TAXIWAY: Color = Color::srgba(0.961, 0.722, 0.255, 0.32);
/// Stand markers and lead-ins.
pub(crate) const STAND: Color = Color::srgba(0.780, 0.800, 0.840, 0.4);
/// Primary text.
pub(crate) const TEXT: Color = Color::srgb(0.902, 0.918, 0.949);
/// Secondary text.
pub(crate) const TEXT_MUTED: Color = Color::srgb(0.540, 0.580, 0.660);
/// City labels: quieter than airport labels.
pub(crate) const TEXT_PLACE: Color = Color::srgba(0.700, 0.740, 0.820, 0.75);
/// Panel background.
pub(crate) const PANEL: Color = Color::srgba(0.060, 0.086, 0.149, 0.92);
/// Panel outline.
pub(crate) const PANEL_EDGE: Color = Color::srgba(1.0, 1.0, 1.0, 0.06);
