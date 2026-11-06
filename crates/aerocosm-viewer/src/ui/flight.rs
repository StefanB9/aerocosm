//! The panel describing the selected flight, and following it.

use std::fmt::Write as _;

use aerocosm_core::{Id, LatLon, Scenario, ops::FlightView, schedule::Tail, world::Airport};
use bevy::{
    app::{App, Plugin, Startup, Update},
    color::{Alpha, Color},
    ecs::{
        component::Component,
        entity::Entity,
        hierarchy::ChildOf,
        observer::On,
        query::With,
        resource::Resource,
        schedule::IntoScheduleConfigs,
        system::{Commands, Local, Query, Res, ResMut, Single},
    },
    input::{ButtonInput, keyboard::KeyCode},
    picking::{Pickable, events::PointerClick},
    time::{Real, Time},
    ui::{
        AlignItems, AlignSelf, BackgroundColor, BorderColor, BorderRadius, Display, FlexDirection,
        Node, PositionType, UiRect, Val, px,
    },
    utils::default,
};

use super::{card, format, strong, text};
use crate::{
    aircraft::{AircraftSprite, TrafficView},
    camera::{CameraRig, CameraSet},
    data::Fonts,
    driver::SimDriver,
    selection::Selection,
    theme,
};

/// The panel rebuilds at most this often while a flight is selected, in
/// seconds of real time.
const REFRESH_SECONDS: f64 = 0.2;

/// Which flight the camera follows.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub(crate) struct Follow(pub(crate) Option<Id<Tail>>);

/// Shows details of the selected flight and follows it on request.
pub(super) struct FlightPanelPlugin;

impl Plugin for FlightPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Follow>()
            .add_systems(Startup, spawn_panel.after(crate::data::setup_fonts))
            .add_systems(
                Update,
                (
                    refresh_panel,
                    toggle_follow,
                    follow_flight
                        .after(CameraSet)
                        .after(crate::aircraft::DrawSet),
                ),
            );
    }
}

#[derive(Component)]
struct FlightPanel;

#[derive(Component)]
struct FollowButton;

/// The text of the flight panel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FlightLines {
    pub(crate) callsign: String,
    pub(crate) operator: String,
    pub(crate) airline_rgb: [u8; 3],
    pub(crate) route: String,
    pub(crate) route_names: String,
    pub(crate) state: String,
    pub(crate) progress: f32,
    pub(crate) departure: String,
    pub(crate) arrival: String,
}

/// IATA code if there is one, else the identifier.
fn code(airport: &Airport) -> String {
    airport
        .iata()
        .map_or_else(|| airport.ident().to_string(), |iata| iata.to_string())
}

/// Fraction of the great-circle way from `origin` to `destination`.
pub(crate) fn progress(origin: LatLon, destination: LatLon, position: LatLon) -> f32 {
    let flown = origin.distance_to(position).value();
    let left = position.distance_to(destination).value();
    let total = flown + left;
    if total <= f64::EPSILON {
        return 1.0;
    }
    // A fraction in 0..=1.
    #[allow(clippy::cast_possible_truncation)]
    let fraction = (flown / total) as f32;
    fraction.clamp(0.0, 1.0)
}

/// The panel text for `view`.
pub(crate) fn flight_lines(scenario: &Scenario, view: &FlightView) -> Option<FlightLines> {
    let traffic = scenario.traffic()?;
    let world = scenario.world();
    let airline = traffic.airlines().get(view.airline())?;
    let aircraft = traffic.aircraft().get(view.aircraft())?;
    let origin = world.airport(view.origin())?;
    let destination = world.airport(view.destination())?;
    let origin_zone = traffic.targets().zone(origin.ident());
    let destination_zone = traffic.targets().zone(destination.ident());
    let late = view.off_block().duration_since(view.scheduled_departure());
    let left = !matches!(view.phase(), aerocosm_core::ops::Phase::Boarding);
    let mut departure = format!(
        "Departure {}",
        format::local_clock(origin_zone, view.scheduled_departure())
    );
    let mut arrival = format!(
        "Arrival {}",
        format::local_clock(destination_zone, view.scheduled_arrival())
    );
    if let Some(delay) = format::delay(late) {
        let off = format::local_clock(origin_zone, view.off_block());
        let verb = if left { "left" } else { "expected" };
        let _ = write!(departure, ", {verb} {off} ({delay})");
        if let Ok(expected) = view.scheduled_arrival().checked_add(late) {
            let _ = write!(
                arrival,
                ", expected {}",
                format::local_clock(destination_zone, expected)
            );
        }
    }
    Some(FlightLines {
        callsign: view.callsign().to_string(),
        operator: format!("{} · {}", airline.name(), aircraft.name()),
        airline_rgb: airline.colour().rgb(),
        route: format!("{} → {}", code(origin), code(destination)),
        route_names: format!("{} → {}", origin.name(), destination.name()),
        state: format!(
            "{} · {} · {:.0} kt",
            format::phase_name(view.phase()),
            format::altitude(view.altitude(), view.phase()),
            view.ground_speed().value()
        ),
        progress: progress(origin.location(), destination.location(), view.position()),
        departure,
        arrival,
    })
}

fn spawn_panel(mut commands: Commands) {
    commands.spawn((
        FlightPanel,
        Node {
            position_type: PositionType::Absolute,
            top: px(72),
            right: px(12),
            width: px(330),
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            padding: UiRect::all(px(16)),
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(12)),
            display: Display::None,
            ..default()
        },
        card(),
    ));
}

fn refresh_panel(
    mut commands: Commands,
    selection: Res<Selection>,
    traffic: Res<TrafficView>,
    driver: Res<SimDriver>,
    follow: Res<Follow>,
    fonts: Res<Fonts>,
    time: Res<Time<Real>>,
    panel: Single<(Entity, &mut Node), With<FlightPanel>>,
    mut shown: Local<Option<(Id<Tail>, f64, bool)>>,
) {
    let (panel, mut node) = panel.into_inner();
    let selected = selection.selected_flight();
    let view = selected.and_then(|tail| traffic.flight(tail));
    let (Some(tail), Some(view)) = (selected, view) else {
        if shown.is_some() {
            *shown = None;
            node.display = Display::None;
            commands.entity(panel).despawn_children();
        }
        return;
    };
    let now = time.elapsed_secs_f64();
    let following = follow.0 == Some(tail);
    if shown.is_some_and(|(last, at, was)| {
        last == tail && was == following && now - at < REFRESH_SECONDS
    }) {
        return;
    }
    *shown = Some((tail, now, following));
    let Some(lines) = flight_lines(driver.simulation().scenario(), view) else {
        return;
    };
    node.display = Display::Flex;
    commands.entity(panel).despawn_children();
    spawn_lines(&mut commands, &fonts, panel, lines, following);
}

/// Fills the panel with `lines` and the follow button.
fn spawn_lines(
    commands: &mut Commands,
    fonts: &Fonts,
    panel: Entity,
    lines: FlightLines,
    following: bool,
) {
    let [r, g, b] = lines.airline_rgb;
    let airline = Color::srgb_u8(r, g, b);
    commands.spawn((
        strong(fonts, lines.callsign, 17.0, theme::TEXT),
        ChildOf(panel),
    ));
    commands.spawn((strong(fonts, lines.operator, 12.0, airline), ChildOf(panel)));
    commands.spawn((
        strong(fonts, lines.route, 15.0, theme::TEXT),
        ChildOf(panel),
    ));
    commands.spawn((
        text(fonts, lines.route_names, 12.0, theme::TEXT_MUTED),
        ChildOf(panel),
    ));
    commands.spawn((text(fonts, lines.state, 13.0, theme::TEXT), ChildOf(panel)));
    let bar = commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: px(4),
                border_radius: BorderRadius::all(px(2)),
                ..default()
            },
            BackgroundColor(theme::PANEL_EDGE),
            ChildOf(panel),
        ))
        .id();
    commands.spawn((
        Node {
            width: Val::Percent(lines.progress * 100.0),
            height: px(4),
            border_radius: BorderRadius::all(px(2)),
            ..default()
        },
        BackgroundColor(airline),
        ChildOf(bar),
    ));
    commands.spawn((
        text(fonts, lines.departure, 12.0, theme::TEXT_MUTED),
        ChildOf(panel),
    ));
    commands.spawn((
        text(fonts, lines.arrival, 12.0, theme::TEXT_MUTED),
        ChildOf(panel),
    ));
    let label = if following {
        "Following · F"
    } else {
        "Follow · F"
    };
    let color = if following {
        theme::ACCENT
    } else {
        theme::TEXT
    };
    commands
        .spawn((
            FollowButton,
            Node {
                margin: UiRect::top(px(6)),
                padding: UiRect::axes(px(10), px(5)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                align_items: AlignItems::Center,
                align_self: AlignSelf::FlexStart,
                ..default()
            },
            BorderColor::all(color.with_alpha(0.5)),
            ChildOf(panel),
            bevy::ecs::children![(strong(fonts, label, 12.0, color), Pickable::IGNORE)],
        ))
        .observe(
            |_: On<PointerClick>, selection: Res<Selection>, mut follow: ResMut<Follow>| {
                follow.0 = toggled(follow.0, selection.selected_flight());
            },
        );
}

/// Whether following `selected` should be switched on or off.
pub(crate) fn toggled(follow: Option<Id<Tail>>, selected: Option<Id<Tail>>) -> Option<Id<Tail>> {
    match (follow, selected) {
        (Some(current), Some(tail)) if current == tail => None,
        (_, selected) => selected,
    }
}

fn toggle_follow(
    keys: Res<ButtonInput<KeyCode>>,
    selection: Res<Selection>,
    mut follow: ResMut<Follow>,
) {
    if keys.just_pressed(KeyCode::KeyF) {
        let next = toggled(follow.0, selection.selected_flight());
        if follow.0 != next {
            follow.0 = next;
        }
    }
}

/// Keeps the followed aircraft centred; dragging, deselecting or the flight
/// ending stops following.
fn follow_flight(
    rig: Option<ResMut<CameraRig>>,
    selection: Res<Selection>,
    sprites: Query<(&AircraftSprite, &bevy::camera::visibility::Visibility)>,
    mut follow: ResMut<Follow>,
) {
    let (Some(mut rig), Some(tail)) = (rig, follow.0) else {
        return;
    };
    let target = sprites
        .iter()
        .find(|(sprite, visibility)| {
            sprite.tail == tail && **visibility != bevy::camera::visibility::Visibility::Hidden
        })
        .map(|(sprite, _)| sprite.position);
    match target {
        Some(position) if !rig.dragging() && selection.selected_flight() == Some(tail) => {
            let limits = rig.limits;
            rig.state.follow(position, &limits);
        }
        _ => follow.0 = None,
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::{RngSeed, SimConfig, Simulation};
    use anyhow::{Context, Result};

    use super::*;

    #[test]
    fn test_progress_along_the_great_circle() -> Result<()> {
        let frankfurt = LatLon::new(50.03, 8.57)?;
        let munich = LatLon::new(48.35, 11.79)?;
        let middle = frankfurt.interpolate(munich, 0.25)?;
        assert!((progress(frankfurt, munich, middle) - 0.25).abs() < 1e-3);
        assert!((progress(frankfurt, frankfurt, frankfurt) - 1.0).abs() < 1e-6);
        Ok(())
    }

    #[test]
    fn test_toggled_follows_and_unfollows_the_selection() {
        let (a, b) = (Id::new(1), Id::new(2));
        assert_eq!(toggled(None, Some(a)), Some(a));
        assert_eq!(toggled(Some(a), Some(a)), None);
        assert_eq!(toggled(Some(a), Some(b)), Some(b));
        assert_eq!(toggled(Some(a), None), None);
    }

    #[test]
    fn test_flight_lines_describe_a_real_flight() -> Result<()> {
        let (scenario, _) = crate::data::load()?;
        let simulation = Simulation::new(
            SimConfig::new("2026-11-05T07:00:00Z".parse()?, RngSeed::new(0)),
            scenario,
        )?;
        let view = simulation
            .flights()
            .find(|view| view.phase() == aerocosm_core::ops::Phase::Cruise)
            .context("a cruising flight")?;
        let lines = flight_lines(simulation.scenario(), &view).context("lines")?;
        assert_eq!(lines.callsign, view.callsign().to_string());
        assert!(lines.operator.contains(" · "));
        assert!(lines.route.contains(" → "));
        assert!(lines.state.starts_with("Cruising · FL"), "{}", lines.state);
        assert!(lines.state.ends_with(" kt"));
        assert!((0.0..=1.0).contains(&lines.progress));
        assert!(lines.departure.starts_with("Departure "));
        assert!(lines.arrival.starts_with("Arrival "));
        Ok(())
    }
}
