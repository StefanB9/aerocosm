//! Inputs and entry points for the viewer's benchmarks (plan M5 D139,
//! D140): each prepares its data from the shipped assets once and then runs
//! one of the viewer's own functions, so that `benches/` can time them
//! without the viewer's internals becoming public.

use std::time::{Duration, Instant};

use aerocosm_basemap::Basemap;
use aerocosm_core::{
    LatLon,
    ground::{GroundDefinition, GroundNetwork, ground_file},
    world::World,
};
use anyhow::{Context, Result, bail};
use bevy::{
    DefaultPlugins,
    app::{App, PluginGroup},
    asset::{AssetMetaCheck, AssetPlugin},
    camera::visibility::Visibility,
    ecs::{entity::Entity, query::With},
    log::LogPlugin,
    math::{Rect, Vec2},
    render::{
        RenderPlugin,
        settings::{RenderCreation, WgpuSettings},
    },
    utils::default,
    window::{ExitCondition, Window, WindowPlugin, WindowResolution},
    winit::WinitPlugin,
};

use crate::{
    driver::SimDriver,
    ground::stream,
    labels::{LabelCandidate, place_labels},
    loading::REGIONS,
    projection::MapProjection,
    start::StartView,
    tessellate::{fill_outlines, fill_polygons},
    ui::labels::estimated_size,
};

/// The views the frames are timed at (plan M5 D140).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    /// The whole world, as the viewer opens.
    World,
    /// Europe at country zoom.
    Europe,
    /// Frankfurt's stands, its ground built.
    Frankfurt,
}

impl View {
    /// Every view, in the order they are measured.
    pub const ALL: [Self; 3] = [Self::World, Self::Europe, Self::Frankfurt];

    /// The view's name in results.
    pub const fn name(self) -> &'static str {
        match self {
            Self::World => "world",
            Self::Europe => "europe",
            Self::Frankfurt => "frankfurt",
        }
    }

    fn start(self) -> Result<StartView> {
        let (lat, lon, scale) = match self {
            Self::World => return StartView::home(),
            Self::Europe => (50.0, 10.0, 2_500.0),
            Self::Frankfurt => (50.035, 8.56, 1.5),
        };
        let args = ["--view".to_owned(), format!("{lat},{lon},{scale}")];
        StartView::from_args(args.into_iter())
    }
}

/// Median, 95th percentile, worst and mean of `samples` (sorted in place),
/// by nearest rank; `None` if there are none or any is not a number.
#[allow(clippy::cast_precision_loss)]
pub fn summary(samples: &mut [f64]) -> Option<[f64; 4]> {
    if samples.is_empty() || samples.iter().any(|s| s.is_nan()) {
        return None;
    }
    samples.sort_by(f64::total_cmp);
    let len = samples.len();
    let rank = |percent: usize| samples[(percent * len).div_ceil(100).clamp(1, len) - 1];
    let mean = samples.iter().sum::<f64>() / len as f64;
    Some([rank(50), rank(95), samples[len - 1], mean])
}

/// Give up waiting for the ground layouts and the simulation after this.
const READY_WITHIN: Duration = Duration::from_secs(300);

/// The viewer without a window or a GPU: Bevy's plugins with no render
/// backend and no windowing, a window entity of the desktop size, the
/// viewer's own plugins and data. Its frames are the viewer's CPU work.
pub struct Headless {
    app: App,
}

impl Headless {
    /// The viewer at `view`, once its ground layouts are loaded and its
    /// simulation built; the simulation is paused, so frames time the
    /// viewer alone.
    pub fn new(view: View) -> Result<Self> {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        resolution: WindowResolution::new(1440, 900),
                        ..default()
                    }),
                    exit_condition: ExitCondition::DontExit,
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets").to_owned(),
                    meta_check: AssetMetaCheck::Never,
                    ..default()
                })
                .set(RenderPlugin {
                    render_creation: RenderCreation::Automatic(Box::new(WgpuSettings {
                        backends: None,
                        ..default()
                    })),
                    ..default()
                })
                .disable::<WinitPlugin>()
                // One subscriber per process; several viewers run in turn.
                .disable::<LogPlugin>(),
        );
        crate::assemble(&mut app, view.start()?)?;
        app.finish();
        app.cleanup();
        let started = Instant::now();
        while !app.world().contains_resource::<SimDriver>() {
            if started.elapsed() > READY_WITHIN {
                bail!("the simulation was not ready within {READY_WITHIN:?}");
            }
            app.update();
        }
        app.world_mut()
            .get_resource_mut::<SimDriver>()
            .context("simulation")?
            .set_speed(aerocosm_core::SimSpeed::Paused);
        Ok(Self { app })
    }

    /// Runs one frame and returns its wall time.
    pub fn frame(&mut self) -> Duration {
        let start = Instant::now();
        self.app.update();
        start.elapsed()
    }

    /// How many entities the viewer has: a check that the view is drawn.
    pub fn entities(&mut self) -> usize {
        let world = self.app.world_mut();
        world.query::<Entity>().iter(world).count()
    }

    /// How many aircraft sprites the viewer has, and how many of them are
    /// shown (plan M9 PR 2: a pool for the aircraft on screen).
    pub fn aircraft_sprites(&mut self) -> (usize, usize) {
        let world = self.app.world_mut();
        let mut sprites = 0;
        let mut shown = 0;
        for visibility in world
            .query_filtered::<&Visibility, With<crate::aircraft::AircraftSprite>>()
            .iter(world)
        {
            sprites += 1;
            if *visibility != Visibility::Hidden {
                shown += 1;
            }
        }
        (sprites, shown)
    }
}

/// The screen the micro-benchmarks lay labels out on, pixels.
const VIEWPORT: Vec2 = Vec2::new(1440.0, 900.0);
/// Europe at country zoom, map units per pixel.
const EUROPE_SCALE: f32 = 2_500.0;
/// Frankfurt's ground in view, map units per pixel.
const AIRPORT_SCALE: f32 = 5.0;

/// The inputs of the viewer's micro-benchmarks (plan M5 D139), prepared
/// once from the shipped data: Europe at country zoom for labels and
/// streaming, Frankfurt for its ground.
pub struct Micro {
    basemap: Basemap,
    world: World,
    europe: MapProjection,
    candidates: Vec<LabelCandidate>,
    placed: Vec<(usize, Rect)>,
    areas: Vec<(usize, Rect)>,
    frankfurt_view: Rect,
    aprons: Vec<Vec<Vec2>>,
}

impl Micro {
    /// Reads the shipped world, basemap and every ground region.
    pub fn new() -> Result<Self> {
        let world = World::from_asset(include_bytes!("../../../assets/world/world.bin"))
            .context("world")?;
        let basemap = Basemap::from_asset(crate::data::BASEMAP).context("basemap")?;
        let europe = MapProjection::new(LatLon::new(50.0, 10.0)?);
        let to_screen =
            |position: Vec2| VIEWPORT / 2.0 + Vec2::new(position.x, -position.y) / EUROPE_SCALE;
        let screen = Rect::from_corners(Vec2::ZERO, VIEWPORT);
        let candidates = world
            .airports()
            .map(|(_, airport)| {
                let code = airport.ident().to_string();
                (to_screen(europe.project(airport.location())), code)
            })
            .filter(|(anchor, _)| screen.contains(*anchor))
            .map(|(anchor, code)| LabelCandidate {
                anchor,
                size: estimated_size(&code, 12.0),
                gap: Some(5.0),
            })
            .collect();
        let mut regions = Vec::with_capacity(REGIONS.len());
        for region in REGIONS {
            let path = format!(
                "{}/../../assets/ground/{}",
                env!("CARGO_MANIFEST_DIR"),
                ground_file(Some(region))
            );
            let bytes = std::fs::read(&path).with_context(|| path.clone())?;
            regions.push(GroundDefinition::from_asset(&bytes).with_context(|| path.clone())?);
        }
        let ground = GroundNetwork::from_definition(GroundDefinition::combine(regions)?, &world)?;
        let areas = ground
            .airports()
            .iter()
            .enumerate()
            .filter_map(|(index, layout)| {
                let points = layout.nodes().iter().map(|n| europe.project(n.position()));
                Some((index, crate::ground::geometry::bounds(points)?))
            })
            .collect();
        let frankfurt = world.airport_by_ident("EDDF".parse()?).context("EDDF")?;
        let layout = ground.airport(frankfurt).context("EDDF's ground")?;
        let at = world.airport(frankfurt).context("EDDF")?.location();
        let frankfurt_view = Rect::from_center_size(europe.project(at), VIEWPORT * AIRPORT_SCALE);
        let frame = MapProjection::new(at);
        let aprons = layout
            .aprons()
            .iter()
            .map(|apron| apron.iter().map(|p| frame.project(*p)).collect())
            .collect();
        Ok(Self {
            basemap,
            world,
            europe,
            candidates,
            placed: Vec::new(),
            areas,
            frankfurt_view,
            aprons,
        })
    }

    /// Decodes the shipped basemap; returns its countries.
    pub fn decode_basemap() -> Result<usize> {
        Ok(Basemap::from_asset(crate::data::BASEMAP)?.countries().len())
    }

    /// Lays out the labels of the airports in view over Europe; returns how
    /// many are placed.
    pub fn place_labels(&mut self) -> usize {
        place_labels(&self.candidates, VIEWPORT, &[], &mut self.placed);
        self.placed.len()
    }

    /// Projects every label source again, as after a move of the floating
    /// origin: each airport, place and runway end.
    pub fn reproject(&self) -> f32 {
        let mut sum = 0.0;
        let mut add = |at: LatLon| sum += self.europe.project(at).x;
        for (id, airport) in self.world.airports() {
            add(airport.location());
            for runway in self.world.runways_of(id) {
                for end in runway.ends() {
                    add(end.threshold());
                }
            }
        }
        for place in self.basemap.places() {
            add(place.location());
        }
        sum
    }

    /// Which grounds to build with Frankfurt in view, none built yet;
    /// returns how many.
    pub fn stream_plan(&self) -> usize {
        let plan = stream::plan(
            self.areas.iter().copied(),
            self.frankfurt_view,
            AIRPORT_SCALE,
            |_| false,
        );
        plan.build.len()
    }

    /// Tessellates the country with the most points; returns its triangle
    /// indices.
    pub fn tessellate_country(&self) -> Result<usize> {
        let country = self
            .basemap
            .countries()
            .iter()
            .max_by_key(|c| {
                c.polygons
                    .iter()
                    .map(|p| p.outer().path().points().len())
                    .sum::<usize>()
            })
            .context("no country")?;
        let frame = country
            .polygons
            .first()
            .and_then(|p| p.outer().path().points().first())
            .map(|p| MapProjection::new(*p))
            .context("empty country")?;
        Ok(fill_polygons(&country.polygons, &frame)?.indices.len())
    }

    /// Tessellates Frankfurt's aprons; returns their triangle indices.
    pub fn tessellate_frankfurt(&self) -> Result<usize> {
        Ok(fill_outlines(self.aprons.iter().cloned())?.indices.len())
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_micro_inputs_do_real_work() -> Result<()> {
        let mut micro = Micro::new()?;
        assert!(Micro::decode_basemap()? > 150);
        assert!(micro.place_labels() > 50);
        assert!(micro.reproject().is_finite());
        assert!(micro.stream_plan() > 0);
        assert!(micro.tessellate_country()? > 1_000);
        assert!(micro.tessellate_frankfurt()? > 100);
        Ok(())
    }

    #[test]
    fn test_summary_by_nearest_rank() {
        let mut samples: Vec<f64> = (1..=100).rev().map(f64::from).collect();
        assert_eq!(summary(&mut samples), Some([50.0, 95.0, 100.0, 50.5]));
        assert_eq!(summary(&mut []), None);
        assert_eq!(summary(&mut [f64::NAN]), None);
    }

    #[test]
    fn test_headless_viewer_draws_frankfurt() -> Result<()> {
        let mut viewer = Headless::new(View::Frankfurt)?;
        let before = viewer.entities();
        for _ in 0..5 {
            viewer.frame();
        }
        // Frankfurt's ground is built over the first frames.
        assert!(viewer.entities() > before, "{before}");
        Ok(())
    }

    #[test]
    fn test_headless_viewer_makes_sprites_for_shown_aircraft_only() -> Result<()> {
        let mut viewer = Headless::new(View::World)?;
        for _ in 0..3 {
            viewer.frame();
        }
        let (sprites, shown) = viewer.aircraft_sprites();
        assert!(shown > 1_000, "the world's airborne flights: {shown}");
        assert!(sprites >= shown);
        assert!(
            sprites < 40_000,
            "a pool for those shown, not one per airframe: {sprites}"
        );
        Ok(())
    }

    #[test]
    fn test_views_open_where_asked() -> Result<()> {
        assert_eq!(View::World.start()?, StartView::home()?);
        let frankfurt = View::Frankfurt.start()?;
        assert_eq!(frankfurt.scale, Some(1.5));
        assert_eq!(View::ALL.map(View::name), ["world", "europe", "frankfurt"]);
        Ok(())
    }
}
