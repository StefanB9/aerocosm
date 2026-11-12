//! Screen-space UI: top bar with clock and speed controls, the airport panel,
//! map labels.

mod boards;
mod flight;
mod format;
mod labels;
mod panel;
mod status;

use aerocosm_core::SimSpeed;
use bevy::{
    picking::events::{PointerClick, PointerOut, PointerOver},
    prelude::*,
};

pub(crate) use self::labels::LabelsPlugin;
use crate::{
    camera::{CameraRig, PointerState},
    data::Fonts,
    driver::{SimDriver, format_clock},
    projection::MapProjection,
    theme,
};

/// Data credits in the footer; OpenStreetMap's is required by its licence
/// (plan M4 D65).
const ATTRIBUTION: &str =
    "Data: OurAirports · © OpenStreetMap contributors · Made with Natural Earth";

/// Speeds offered in the top bar.
const SPEEDS: [SimSpeed; 7] = [
    SimSpeed::Paused,
    SimSpeed::X1,
    SimSpeed::X10,
    SimSpeed::X60,
    SimSpeed::X300,
    SimSpeed::X1000,
    SimSpeed::Max,
];

/// The top bar, info panel and hints.
pub(crate) struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            LabelsPlugin,
            panel::PanelPlugin,
            flight::FlightPanelPlugin,
            boards::BoardsPlugin,
        ))
        .add_systems(Startup, spawn_chrome.after(crate::data::setup_fonts))
        .add_systems(
            Update,
            (
                update_clock,
                speed_shortcuts,
                style_speed_buttons,
                update_cursor_readout.after(crate::camera::CameraSet),
            ),
        );
    }
}

/// A UI element over the map that map labels keep clear of.
#[derive(Component)]
pub(crate) struct MapCover;

#[derive(Component)]
struct ClockText;

/// Shows how many aircraft are airborne.
#[derive(Component)]
struct AirborneText;

/// Shows the geographic position under the pointer.
#[derive(Component)]
struct CursorReadout;

#[derive(Component)]
struct SpeedButton {
    speed: SimSpeed,
    hovered: bool,
}

/// The speed restored when un-pausing with the space bar.
#[derive(Resource)]
struct ResumeSpeed(SimSpeed);

/// A rounded, translucent card.
pub(crate) fn card() -> impl Bundle {
    (
        BackgroundColor(theme::PANEL),
        BorderColor::all(theme::PANEL_EDGE),
        BoxShadow::new(Color::BLACK.with_alpha(0.45), px(0), px(6), px(0), px(18)),
    )
}

pub(crate) fn text(
    fonts: &Fonts,
    value: impl Into<String>,
    size: f32,
    color: Color,
) -> impl Bundle {
    (
        Text::new(value),
        TextFont::from_font_size(size).with_font(fonts.regular.clone()),
        TextColor(color),
    )
}

pub(crate) fn strong(
    fonts: &Fonts,
    value: impl Into<String>,
    size: f32,
    color: Color,
) -> impl Bundle {
    (
        Text::new(value),
        TextFont::from_font_size(size).with_font(fonts.semibold.clone()),
        TextColor(color),
    )
}

fn spawn_chrome(mut commands: Commands, fonts: Res<Fonts>) {
    commands.insert_resource(ResumeSpeed(SimSpeed::default()));
    spawn_top_bar(&mut commands, &fonts);
    spawn_footer(&mut commands, &fonts);
}

fn spawn_top_bar(commands: &mut Commands, fonts: &Fonts) {
    let bar = commands
        .spawn((
            MapCover,
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                left: px(12),
                right: px(12),
                height: px(48),
                padding: UiRect::horizontal(px(16)),
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(12)),
                ..default()
            },
            card(),
        ))
        .id();

    commands.spawn((
        Node {
            align_items: AlignItems::Baseline,
            column_gap: px(10),
            ..default()
        },
        ChildOf(bar),
        children![
            strong(fonts, "AEROCOSM", 15.0, theme::TEXT),
            text(fonts, "Germany", 13.0, theme::TEXT_MUTED),
        ],
    ));
    commands.spawn((
        Node {
            align_items: AlignItems::Baseline,
            column_gap: px(12),
            ..default()
        },
        ChildOf(bar),
        children![
            (ClockText, strong(fonts, "", 15.0, theme::TEXT)),
            (AirborneText, text(fonts, "", 13.0, theme::TEXT_MUTED)),
        ],
    ));
    spawn_speed_buttons(commands, fonts, bar);
}

fn spawn_speed_buttons(commands: &mut Commands, fonts: &Fonts, bar: Entity) {
    let speeds = commands
        .spawn((
            Node {
                column_gap: px(4),
                align_items: AlignItems::Center,
                ..default()
            },
            ChildOf(bar),
        ))
        .id();
    for speed in SPEEDS {
        let label = match speed {
            SimSpeed::Paused => "Pause".to_owned(),
            SimSpeed::Max => "Max".to_owned(),
            other => other.to_string(),
        };
        commands
            .spawn((
                SpeedButton {
                    speed,
                    hovered: false,
                },
                Node {
                    padding: UiRect::axes(px(10), px(5)),
                    border_radius: BorderRadius::all(px(7)),
                    ..default()
                },
                BackgroundColor(Color::NONE),
                ChildOf(speeds),
                children![(
                    strong(fonts, label, 12.0, theme::TEXT_MUTED),
                    Pickable::IGNORE
                )],
            ))
            .observe(
                |click: On<PointerClick>,
                 buttons: Query<&SpeedButton>,
                 mut driver: ResMut<SimDriver>| {
                    if let Ok(button) = buttons.get(click.entity) {
                        driver.set_speed(button.speed);
                    }
                },
            )
            .observe(
                |over: On<PointerOver>, mut buttons: Query<&mut SpeedButton>| {
                    if let Ok(mut button) = buttons.get_mut(over.entity) {
                        button.hovered = true;
                    }
                },
            )
            .observe(
                |out: On<PointerOut>, mut buttons: Query<&mut SpeedButton>| {
                    if let Ok(mut button) = buttons.get_mut(out.entity) {
                        button.hovered = false;
                    }
                },
            );
    }
}

fn spawn_footer(commands: &mut Commands, fonts: &Fonts) {
    commands.spawn((
        MapCover,
        Node {
            position_type: PositionType::Absolute,
            left: px(16),
            bottom: px(12),
            ..default()
        },
        text(
            fonts,
            "Drag to pan · scroll or pinch to zoom · click an airport or aircraft · F follow · \
             Space pause · , . speed · Home reset",
            12.0,
            theme::TEXT_MUTED,
        ),
        Pickable::IGNORE,
    ));
    commands.spawn((
        MapCover,
        Node {
            position_type: PositionType::Absolute,
            right: px(16),
            bottom: px(12),
            column_gap: px(16),
            ..default()
        },
        Pickable::IGNORE,
        children![
            (
                CursorReadout,
                text(fonts, "", 11.0, theme::TEXT_MUTED),
                Pickable::IGNORE
            ),
            (
                text(fonts, ATTRIBUTION, 11.0, theme::TEXT_MUTED.with_alpha(0.7),),
                Pickable::IGNORE
            ),
        ],
    ));
}

fn update_clock(
    driver: Res<SimDriver>,
    mut clock: Single<&mut Text, (With<ClockText>, Without<AirborneText>)>,
    mut airborne: Single<&mut Text, (With<AirborneText>, Without<ClockText>)>,
) {
    let formatted = format_clock(driver.simulation().time());
    if clock.0 != formatted {
        clock.0 = formatted;
    }
    let count = airborne_text(driver.simulation().airborne());
    if airborne.0 != count {
        airborne.0 = count;
    }
}

/// The top bar's traffic count, e.g. `823 airborne`.
fn airborne_text(count: usize) -> String {
    format!("{count} airborne")
}

fn update_cursor_readout(
    rig: Option<Res<CameraRig>>,
    pointer: Res<PointerState>,
    projection: Res<MapProjection>,
    mut readout: Single<&mut Text, With<CursorReadout>>,
) {
    let position = rig.zip(pointer.cursor).and_then(|(rig, cursor)| {
        let world = rig.state.screen_to_world(cursor, pointer.viewport);
        projection.unproject(world).ok()
    });
    let formatted = position.map_or_else(String::new, |position| position.to_string());
    if readout.0 != formatted {
        readout.0 = formatted;
    }
}

fn speed_shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    mut driver: ResMut<SimDriver>,
    mut resume: ResMut<ResumeSpeed>,
) {
    let speed = driver.speed();
    if keys.just_pressed(KeyCode::Space) {
        if speed == SimSpeed::Paused {
            driver.set_speed(resume.0);
        } else {
            resume.0 = speed;
            driver.set_speed(SimSpeed::Paused);
        }
    }
    if keys.just_pressed(KeyCode::Period) {
        driver.set_speed(speed.faster());
    }
    if keys.just_pressed(KeyCode::Comma) {
        driver.set_speed(speed.slower());
    }
}

fn style_speed_buttons(
    driver: Res<SimDriver>,
    mut buttons: Query<(&SpeedButton, &mut BackgroundColor, &Children)>,
    mut colors: Query<&mut TextColor>,
) {
    for (button, mut background, children) in &mut buttons {
        let active = button.speed == driver.speed();
        let (fill, ink) = match (active, button.hovered) {
            (true, _) => (theme::ACCENT, theme::SEA),
            (false, true) => (Color::WHITE.with_alpha(0.08), theme::TEXT),
            (false, false) => (Color::NONE, theme::TEXT_MUTED),
        };
        background.set_if_neq(BackgroundColor(fill));
        for child in children {
            if let Ok(mut color) = colors.get_mut(*child) {
                color.set_if_neq(TextColor(ink));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attribution_credits_every_source() {
        // The ground layouts are ODbL: the credit must be visible.
        assert!(ATTRIBUTION.contains("© OpenStreetMap contributors"));
        assert!(ATTRIBUTION.contains("OurAirports") && ATTRIBUTION.contains("Natural Earth"));
    }

    #[test]
    fn test_airborne_text() {
        assert_eq!(airborne_text(823), "823 airborne");
        assert_eq!(airborne_text(0), "0 airborne");
    }
}
