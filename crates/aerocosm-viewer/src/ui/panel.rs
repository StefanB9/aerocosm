//! The panel describing the selected airport.

use aerocosm_core::{
    Id,
    ops::BoardKind,
    world::{Airport, AirportKind, GeometrySource, Surface},
};
use bevy::prelude::*;

use super::{boards::spawn_board, card, strong, text};
use crate::{airports::WorldView, data::Fonts, selection::Selection, theme};

/// Shows details of the selected airport.
pub(super) struct PanelPlugin;

impl Plugin for PanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_panel.after(crate::data::setup_fonts))
            .add_systems(Update, (refresh_panel, close_with_escape));
    }
}

#[derive(Component)]
struct Panel;

fn spawn_panel(mut commands: Commands) {
    commands.spawn((
        Panel,
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

fn kind_name(kind: AirportKind) -> &'static str {
    match kind {
        AirportKind::Major => "Major airport",
        AirportKind::Regional => "Regional airport",
        AirportKind::Airfield => "Airfield",
        AirportKind::Edge => "Gateway",
    }
}

fn surface_name(surface: Surface) -> &'static str {
    match surface {
        Surface::Paved => "paved",
        Surface::Grass => "grass",
        Surface::Unpaved => "unpaved",
        Surface::Water => "water",
        Surface::Unknown => "unknown surface",
    }
}

/// The panel's subtitle: codes and kind, e.g. `EDDF · FRA · Major airport`.
fn subtitle(icao_or_ident: &str, iata: Option<&str>, kind: AirportKind) -> String {
    match iata {
        Some(iata) => format!("{icao_or_ident} · {iata} · {}", kind_name(kind)),
        None => format!("{icao_or_ident} · {}", kind_name(kind)),
    }
}

fn refresh_panel(
    mut commands: Commands,
    selection: Res<Selection>,
    view: Res<WorldView>,
    fonts: Res<Fonts>,
    panel: Single<(Entity, &mut Node), With<Panel>>,
    mut shown: Local<Option<Id<Airport>>>,
) {
    // Hover changes also touch `Selection`; rebuild only when the selected
    // airport changes. The panel starts hidden with nothing selected, which
    // matches the initial `None`.
    if *shown == selection.selected_airport() {
        return;
    }
    *shown = selection.selected_airport();
    let (panel, mut node) = panel.into_inner();
    commands.entity(panel).despawn_children();
    let Some(airport) = selection
        .selected_airport()
        .and_then(|id| view.world.airport(id))
    else {
        node.display = Display::None;
        return;
    };
    let Some(id) = selection.selected_airport() else {
        return;
    };
    node.display = Display::Flex;
    let iata = airport.iata().map(|code| code.to_string());
    commands.spawn((
        strong(&fonts, airport.name(), 17.0, theme::TEXT),
        ChildOf(panel),
    ));
    commands.spawn((
        strong(
            &fonts,
            subtitle(airport.ident().as_str(), iata.as_deref(), airport.kind()),
            12.0,
            theme::ACCENT,
        ),
        ChildOf(panel),
    ));
    commands.spawn((
        text(
            &fonts,
            format!(
                "{}  ·  elevation {:.0} ft",
                airport.location(),
                airport.elevation().value()
            ),
            12.0,
            theme::TEXT_MUTED,
        ),
        ChildOf(panel),
    ));
    for kind in [BoardKind::Departures, BoardKind::Arrivals] {
        spawn_board(&mut commands, &fonts, panel, id, kind);
    }
    let runways = view.world.runways_of(id);
    commands.spawn((
        Node {
            margin: UiRect::top(px(8)),
            ..default()
        },
        strong(
            &fonts,
            format!("Runways ({})", runways.len()),
            13.0,
            theme::TEXT,
        ),
        ChildOf(panel),
    ));
    for runway in runways {
        let approximate = if runway.geometry() == GeometrySource::Approximate {
            "  · approx. position"
        } else {
            ""
        };
        commands.spawn((
            text(
                &fonts,
                format!(
                    "{}   {:.0} × {:.0} m, {}{approximate}",
                    runway.name(),
                    runway.length().value(),
                    runway.width().value(),
                    surface_name(runway.surface()),
                ),
                12.0,
                theme::TEXT,
            ),
            ChildOf(panel),
        ));
    }
}

fn close_with_escape(keys: Res<ButtonInput<KeyCode>>, mut selection: ResMut<Selection>) {
    if keys.just_pressed(KeyCode::Escape) && selection.selected.is_some() {
        selection.selected = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subtitle_with_and_without_iata() {
        assert_eq!(
            subtitle("EDDF", Some("FRA"), AirportKind::Major),
            "EDDF · FRA · Major airport"
        );
        assert_eq!(
            subtitle("DE-0062", None, AirportKind::Airfield),
            "DE-0062 · Airfield"
        );
    }

    #[test]
    fn test_surface_names() {
        assert_eq!(surface_name(Surface::Paved), "paved");
        assert_eq!(surface_name(Surface::Unknown), "unknown surface");
    }
}
