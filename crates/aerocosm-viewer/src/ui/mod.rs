//! Screen-space UI: top bar with clock and speed controls, the airport panel,
//! map labels.

mod labels;
mod panel;

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
        app.add_plugins((LabelsPlugin, panel::PanelPlugin))
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

#[derive(Component)]
struct ClockText;

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
        ClockText,
        strong(fonts, "", 15.0, theme::TEXT),
        ChildOf(bar),
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
        Node {
            position_type: PositionType::Absolute,
            left: px(16),
            bottom: px(12),
            ..default()
        },
        text(
            fonts,
            "Drag to pan · scroll or pinch to zoom · click an airport · Space pause · , . speed · \
             Home reset",
            12.0,
            theme::TEXT_MUTED,
        ),
        Pickable::IGNORE,
    ));
    commands.spawn((
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
                text(
                    fonts,
                    "Data: OurAirports · Made with Natural Earth",
                    11.0,
                    theme::TEXT_MUTED.with_alpha(0.7),
                ),
                Pickable::IGNORE
            ),
        ],
    ));
}

fn update_clock(driver: Res<SimDriver>, mut clock: Single<&mut Text, With<ClockText>>) {
    let formatted = format_clock(driver.simulation().time());
    if clock.0 != formatted {
        clock.0 = formatted;
    }
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
