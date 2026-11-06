//! Departure and arrival boards in the airport panel.

use aerocosm_core::{
    Id, Scenario,
    ops::{BoardEntry, BoardKind, FlightStatus},
    world::Airport,
};
use bevy::{
    app::{App, Plugin, Update},
    color::Color,
    ecs::{
        component::Component,
        entity::Entity,
        hierarchy::ChildOf,
        query::Added,
        system::{Commands, Local, Query, Res},
    },
    ui::{FlexDirection, Node, UiRect, px},
    utils::default,
};

use super::{format, strong, text};
use crate::{data::Fonts, driver::SimDriver, theme};

/// Rows per board.
const ROWS: usize = 6;

/// A board's place in the airport panel.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Board {
    pub(crate) airport: Id<Airport>,
    pub(crate) kind: BoardKind,
}

/// How a row's status reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    /// Nothing to note.
    Normal,
    /// Boarding now.
    Active,
    /// Running late.
    Late,
    /// Gone or landed.
    Done,
}

/// One board line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoardRow {
    pub(crate) time: String,
    pub(crate) callsign: String,
    pub(crate) other: String,
    pub(crate) status: String,
    pub(crate) tone: Tone,
}

/// Board lines in the airport's local time.
pub(crate) fn board_rows(
    scenario: &Scenario,
    airport: Id<Airport>,
    entries: &[BoardEntry],
) -> Vec<BoardRow> {
    let world = scenario.world();
    let zone = world
        .airport(airport)
        .zip(scenario.traffic())
        .and_then(|(airport, traffic)| traffic.targets().zone(airport.ident()));
    entries
        .iter()
        .map(|entry| {
            let late = entry.delay();
            let status = format::status(entry.status(), late);
            let tone = match entry.status() {
                FlightStatus::Departed | FlightStatus::Landed => Tone::Done,
                _ if format::delay(late).is_some() => Tone::Late,
                FlightStatus::Boarding => Tone::Active,
                FlightStatus::Scheduled => Tone::Normal,
            };
            BoardRow {
                time: format::local_clock(zone, entry.scheduled()),
                callsign: entry.callsign().to_string(),
                other: world
                    .airport(entry.other())
                    .map_or_else(String::new, |other| {
                        other
                            .iata()
                            .map_or_else(|| other.ident().to_string(), |iata| iata.to_string())
                    }),
                status,
                tone,
            }
        })
        .collect()
}

const fn tone_color(tone: Tone) -> Color {
    match tone {
        Tone::Normal => theme::TEXT,
        Tone::Active => Color::srgb(0.45, 0.85, 0.55),
        Tone::Late => theme::ACCENT,
        Tone::Done => theme::TEXT_MUTED,
    }
}

/// Keeps the boards current.
pub(super) struct BoardsPlugin;

impl Plugin for BoardsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, refresh_boards);
    }
}

fn refresh_boards(
    mut commands: Commands,
    driver: Res<SimDriver>,
    fonts: Res<Fonts>,
    boards: Query<(Entity, &Board)>,
    new: Query<(), Added<Board>>,
    mut last_minute: Local<Option<i64>>,
) {
    let simulation = driver.simulation();
    let minute = simulation.time().as_millis() / 60_000;
    if *last_minute == Some(minute) && new.is_empty() {
        return;
    }
    *last_minute = Some(minute);
    for (entity, board) in &boards {
        commands.entity(entity).despawn_children();
        let entries = simulation.board(board.airport, board.kind, ROWS);
        let rows = board_rows(simulation.scenario(), board.airport, &entries);
        if rows.is_empty() {
            commands.spawn((
                text(&fonts, "No flights", 12.0, theme::TEXT_MUTED),
                ChildOf(entity),
            ));
        }
        for row in rows {
            let line = commands
                .spawn((
                    Node {
                        column_gap: px(8),
                        ..default()
                    },
                    ChildOf(entity),
                ))
                .id();
            for (value, width, strong_text) in [
                (row.time, 40.0, false),
                (row.callsign, 62.0, true),
                (row.other, 44.0, false),
            ] {
                let cell = commands
                    .spawn((
                        Node {
                            width: px(width),
                            ..default()
                        },
                        ChildOf(line),
                    ))
                    .id();
                if strong_text {
                    commands.spawn((strong(&fonts, value, 12.0, theme::TEXT), ChildOf(cell)));
                } else {
                    commands.spawn((text(&fonts, value, 12.0, theme::TEXT), ChildOf(cell)));
                }
            }
            commands.spawn((
                text(&fonts, row.status, 12.0, tone_color(row.tone)),
                ChildOf(line),
            ));
        }
    }
}

/// A titled, empty board for `airport`, filled by [`refresh_boards`].
pub(super) fn spawn_board(
    commands: &mut Commands,
    fonts: &Fonts,
    panel: Entity,
    airport: Id<Airport>,
    kind: BoardKind,
) {
    let title = match kind {
        BoardKind::Departures => "Departures",
        BoardKind::Arrivals => "Arrivals",
    };
    commands.spawn((
        Node {
            margin: UiRect::top(px(8)),
            ..default()
        },
        strong(fonts, title, 13.0, theme::TEXT),
        ChildOf(panel),
    ));
    commands.spawn((
        Board { airport, kind },
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(3),
            ..default()
        },
        ChildOf(panel),
    ));
}

#[cfg(test)]
mod tests {
    use aerocosm_core::{RngSeed, SimConfig, Simulation};
    use anyhow::{Context, Result};

    use super::*;

    #[test]
    fn test_board_rows_read_in_local_time() -> Result<()> {
        let (scenario, _) = crate::data::load()?;
        let simulation = Simulation::new(
            SimConfig::new("2026-11-05T07:00:00Z".parse()?, RngSeed::new(0)),
            scenario,
        )?;
        let frankfurt = simulation
            .world()
            .airport_by_ident("EDDF".parse()?)
            .context("EDDF")?;
        let entries = simulation.board(frankfurt, BoardKind::Departures, ROWS);
        let rows = board_rows(simulation.scenario(), frankfurt, &entries);
        assert_eq!(rows.len(), ROWS);
        for row in &rows {
            // 07:00 UTC is 08:00 in Frankfurt; the board starts 15 min before.
            assert!(
                row.time.as_str() >= "07:45" && row.time.len() == 5,
                "{}",
                row.time
            );
            assert_eq!(row.other.len(), 3, "{}", row.other);
            assert_ne!(row.status.len(), 0);
        }
        Ok(())
    }

    #[test]
    fn test_tone_colours_differ() {
        let tones = [Tone::Normal, Tone::Active, Tone::Late, Tone::Done];
        for (i, a) in tones.iter().enumerate() {
            for b in &tones[i + 1..] {
                assert_ne!(tone_color(*a), tone_color(*b));
            }
        }
    }
}
