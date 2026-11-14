//! What the pointer is over and what is selected: an airport or a flight.

use aerocosm_core::{Id, schedule::Tail, world::Airport};
use bevy::{
    camera::visibility::Visibility,
    ecs::{
        query::With,
        resource::Resource,
        system::{Query, Res, ResMut},
    },
};

use crate::{
    aircraft::AircraftSprite,
    airports::AirportSymbol,
    camera::{CameraRig, PointerState},
    picking,
};

/// Pointer reach for hovering a symbol or aircraft, in pixels.
const HOVER_REACH: f32 = 12.0;

/// Something on the map the pointer can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Pick {
    /// An airport symbol.
    Airport(Id<Airport>),
    /// The flight a tail is flying.
    Flight(Id<Tail>),
}

/// What the pointer is over and what is selected.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub(crate) struct Selection {
    pub(crate) hovered: Option<Pick>,
    pub(crate) selected: Option<Pick>,
}

impl Selection {
    /// The selected airport, if an airport is selected.
    pub(crate) const fn selected_airport(&self) -> Option<Id<Airport>> {
        match self.selected {
            Some(Pick::Airport(id)) => Some(id),
            _ => None,
        }
    }

    /// The hovered airport, if an airport is hovered.
    pub(crate) const fn hovered_airport(&self) -> Option<Id<Airport>> {
        match self.hovered {
            Some(Pick::Airport(id)) => Some(id),
            _ => None,
        }
    }

    /// The selected flight's tail, if a flight is selected.
    pub(crate) const fn selected_flight(&self) -> Option<Id<Tail>> {
        match self.selected {
            Some(Pick::Flight(id)) => Some(id),
            _ => None,
        }
    }

    /// The hovered flight's tail, if a flight is hovered.
    pub(crate) const fn hovered_flight(&self) -> Option<Id<Tail>> {
        match self.hovered {
            Some(Pick::Flight(id)) => Some(id),
            _ => None,
        }
    }
}

/// Hovers the nearest visible airport or aircraft under the pointer; a click
/// selects it, or clears the selection on empty map.
pub(crate) fn hover_and_select(
    rig: Option<Res<CameraRig>>,
    pointer: Res<PointerState>,
    symbols: Query<(&AirportSymbol, &Visibility)>,
    aircraft: Query<(&AircraftSprite, &Visibility), With<AircraftSprite>>,
    mut selection: ResMut<Selection>,
) {
    let Some(rig) = rig else {
        return;
    };
    let hovered = match pointer.cursor {
        Some(cursor) if !pointer.over_ui => {
            let screen = |position| rig.state.world_to_screen(position, pointer.viewport);
            let airports = symbols
                .iter()
                .filter(|(_, visibility)| **visibility != Visibility::Hidden)
                .map(|(symbol, _)| (Pick::Airport(symbol.id), screen(symbol.position)));
            let flights = aircraft
                .iter()
                .filter(|(_, visibility)| **visibility != Visibility::Hidden)
                .map(|(sprite, _)| (Pick::Flight(sprite.tail), screen(sprite.position)));
            picking::nearest(airports.chain(flights), cursor, HOVER_REACH)
        }
        _ => None,
    };
    if selection.hovered != hovered {
        selection.hovered = hovered;
    }
    if pointer.clicked {
        selection.selected = hovered;
    }
}

#[cfg(test)]
mod tests {
    use aerocosm_core::world::AirportKind;
    use bevy::prelude::*;

    use super::*;
    use crate::camera::{
        CameraRig, PointerState,
        controller::{CameraLimits, CameraState},
    };

    fn app(cursor: Option<Vec2>, clicked: bool) -> App {
        let mut app = App::new();
        app.insert_resource(CameraRig::new(
            CameraState::new(Vec2::ZERO, 10.0),
            CameraLimits {
                bounds: Rect::new(-1e6, -1e6, 1e6, 1e6),
                min_scale: 1.0,
                max_scale: 100.0,
                max_width: f32::INFINITY,
            },
        ))
        .insert_resource(PointerState {
            cursor,
            clicked,
            over_ui: false,
            viewport: Vec2::new(800.0, 600.0),
        })
        .init_resource::<Selection>()
        .add_systems(Update, hover_and_select);
        // At scale 10 the screen centre (400, 300) is world (0, 0); this symbol
        // is 50 px right.
        app.world_mut().spawn((
            AirportSymbol {
                id: Id::new(7),
                kind: AirportKind::Major,
                position: Vec2::new(500.0, 0.0),
            },
            Visibility::Inherited,
        ));
        app
    }

    fn spawn_aircraft(app: &mut App, tail: u32, position: Vec2, visibility: Visibility) {
        app.world_mut()
            .spawn((AircraftSprite::new(Id::new(tail), position), visibility));
    }

    #[test]
    fn test_hover_and_click_select_symbol_under_cursor() {
        let mut app = app(Some(Vec2::new(452.0, 301.0)), true);
        app.update();
        let selection = app.world().resource::<Selection>();
        assert_eq!(selection.hovered, Some(Pick::Airport(Id::new(7))));
        assert_eq!(selection.selected_airport(), Some(Id::new(7)));
        assert_eq!(selection.selected_flight(), None);
    }

    #[test]
    fn test_click_on_empty_map_clears_selection() {
        let mut app = app(Some(Vec2::new(100.0, 100.0)), true);
        app.world_mut().resource_mut::<Selection>().selected = Some(Pick::Airport(Id::new(7)));
        app.update();
        let selection = app.world().resource::<Selection>();
        assert_eq!(selection.hovered, None);
        assert_eq!(selection.selected, None);
    }

    #[test]
    fn test_hover_without_click_keeps_selection() {
        let mut app = app(Some(Vec2::new(452.0, 301.0)), false);
        app.update();
        assert_eq!(app.world().resource::<Selection>().selected, None);
    }

    #[test]
    fn test_nearest_of_airports_and_flights_wins() {
        let mut app = app(Some(Vec2::new(452.0, 301.0)), true);
        // An aircraft 1 px from the cursor beats the airport 2 px away.
        spawn_aircraft(&mut app, 3, Vec2::new(520.0, -10.0), Visibility::Inherited);
        app.update();
        let selection = app.world().resource::<Selection>();
        assert_eq!(selection.selected_flight(), Some(Id::new(3)));
        assert_eq!(selection.hovered_flight(), Some(Id::new(3)));
        assert_eq!(selection.selected_airport(), None);
    }

    #[test]
    fn test_hidden_aircraft_are_not_picked() {
        let mut app = app(Some(Vec2::new(452.0, 301.0)), true);
        spawn_aircraft(&mut app, 3, Vec2::new(520.0, -10.0), Visibility::Hidden);
        app.update();
        assert_eq!(
            app.world().resource::<Selection>().selected_airport(),
            Some(Id::new(7))
        );
    }
}
