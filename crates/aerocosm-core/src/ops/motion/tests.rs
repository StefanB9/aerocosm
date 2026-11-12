//! Tests of the scalar kinematics.

use anyhow::{Context, Result, bail};
use proptest::prelude::*;

use super::*;

/// An A320-like profile over `distance` metres between sea-level fields.
fn profile(distance: f64) -> Profile {
    Profile::new(&ProfileInputs {
        cruise_speed: 230.0,
        cruise_altitude: 11_000.0,
        climb_rate: 10.0,
        descent_rate: 10.0,
        approach_speed: 70.0,
        origin_elevation: 100.0,
        destination_elevation: 450.0,
        enroute_length: distance,
        taxi_out_length: 2_000.0,
        taxi_in_length: 1_500.0,
        ..ProfileInputs::default()
    })
}

/// The same profile with a pushback, a landing roll to an exit and an exit
/// speed (plan M4 D71, D72).
fn ground_profile(pushback: f64, landing: f64, exit_speed: f64) -> Profile {
    let mut inputs = profile(400_000.0).inputs;
    inputs.pushback_length = pushback;
    inputs.landing_length = landing;
    inputs.exit_speed = exit_speed;
    Profile::new(&inputs)
}

struct Flown {
    phases: Vec<Phase>,
    events: Vec<Event>,
    seconds: u32,
    peak: f64,
    faf_altitude: Option<f64>,
}

fn fly(profile: &Profile, dt: f64) -> Result<Flown> {
    fly_with(profile, dt, |_| {}, |_, _| true)
}

/// Flies a leg after `setup`, asking `clearance` with the seconds flown.
fn fly_with(
    profile: &Profile,
    dt: f64,
    setup: impl FnOnce(&mut Motion),
    mut clearance: impl FnMut(Request, u32) -> bool,
) -> Result<Flown> {
    let mut motion = Motion::start(profile, 600.0, 300.0);
    setup(&mut motion);
    let mut flown = Flown {
        phases: vec![motion.phase()],
        events: Vec::new(),
        seconds: 0,
        peak: 0.0,
        faf_altitude: None,
    };
    while motion.phase() != Phase::Arrived {
        if flown.seconds > 20 * 3600 {
            bail!("never arrived; stuck in {:?}", motion.phase());
        }
        let before = motion.phase();
        let now = flown.seconds;
        if let Some(event) = motion.advance(profile, dt, &mut |request| clearance(request, now)) {
            flown.events.push(event);
        }
        if before == Phase::Descent && motion.phase() == Phase::Approach {
            flown.faf_altitude = Some(motion.altitude());
        }
        if motion.phase() != before {
            flown.phases.push(motion.phase());
        }
        flown.peak = flown.peak.max(motion.altitude());
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let step = dt as u32;
        flown.seconds += step;
    }
    Ok(flown)
}

#[test]
fn test_motion_pushes_back_then_taxies_from_the_push_point() {
    let pushed = ground_profile(50.0, 0.0, 0.0);
    let mut motion = Motion::start(&pushed, 600.0, 300.0);
    assert_eq!(motion.phase(), Phase::Pushback);
    // Pushed back at walking pace.
    motion.advance_free(&pushed, 10.0);
    assert!((motion.along() - 10.0).abs() < 1e-9);
    assert!(motion.speed() > 0.0 && motion.speed() < 2.0);
    for _ in 0..4 {
        motion.advance_free(&pushed, 10.0);
    }
    assert!((motion.along() - 50.0).abs() < 1e-9);
    // Engines start before it taxis.
    assert_eq!(motion.phase(), Phase::Pushback);
    assert!(motion.speed().abs() < 1e-9);
    // A minute of engine start.
    for _ in 0..6 {
        motion.advance_free(&pushed, 10.0);
    }
    assert_eq!(motion.phase(), Phase::TaxiOut);
    let at = motion.along();
    assert!(
        (at - 50.0).abs() < 1e-9,
        "taxiing starts where the push ended: {at}"
    );
    motion.advance_free(&pushed, 60.0);
    // 1,950 m left in 600 s.
    assert!((motion.along() - (50.0 + 1_950.0 / 10.0)).abs() < 1e-6);
    // Without a pushback the aircraft taxies straight away.
    let plain = Motion::start(&profile(400_000.0), 600.0, 300.0);
    assert_eq!(plain.phase(), Phase::TaxiOut);
}

#[test]
fn test_motion_lines_up_after_the_take_off_clearance() {
    let mut inputs = profile(400_000.0).inputs;
    inputs.lineup_length = 100.0;
    let lined = Profile::new(&inputs);
    let mut motion = Motion::start(&lined, 60.0, 300.0);
    while motion.phase() != Phase::Queue {
        motion.advance_free(&lined, 1.0);
    }
    // Held short until cleared.
    motion.advance(&lined, 1.0, &mut |_| false);
    assert_eq!(motion.phase(), Phase::Queue);
    motion.advance(&lined, 1.0, &mut |request| request == Request::Takeoff);
    assert_eq!(motion.phase(), Phase::LineUp);
    for _ in 0..19 {
        motion.advance_free(&lined, 1.0);
    }
    assert_eq!(motion.phase(), Phase::LineUp);
    assert!((motion.along() - 95.0).abs() < 1e-6, "{}", motion.along());
    motion.advance_free(&lined, 1.0);
    assert_eq!(motion.phase(), Phase::TakeoffRoll);
    // Without a line-up the roll starts at the clearance.
    let plain = profile(400_000.0);
    let mut direct = Motion::start(&plain, 60.0, 300.0);
    while direct.phase() == Phase::TaxiOut {
        direct.advance_free(&plain, 1.0);
    }
    direct.advance_free(&plain, 1.0);
    assert_eq!(direct.phase(), Phase::TakeoffRoll);
}

#[test]
fn test_motion_rolls_out_to_the_planned_exit() -> Result<()> {
    // An exit 2,500 m down the runway, left at 15 m/s.
    let profile = ground_profile(0.0, 2_500.0, 15.0);
    assert!((profile.landing_length() - 2_500.0).abs() < 1e-9);
    let flown = fly(&profile, 1.0)?;
    assert!(flown.phases.contains(&Phase::Landing));
    let mut motion = Motion::start(&profile, 600.0, 300.0);
    while motion.phase() != Phase::Landing {
        motion.advance_free(&profile, 1.0);
    }
    let mut slowest: f64 = f64::INFINITY;
    while motion.phase() == Phase::Landing {
        motion.advance_free(&profile, 1.0);
        slowest = slowest.min(motion.speed());
    }
    assert_eq!(motion.phase(), Phase::TaxiIn);
    assert!(
        slowest >= 15.0 - 1e-9,
        "never slower than the exit speed: {slowest}"
    );
    // The default roll ends where braking reaches taxi speed.
    let plain = profile_default_landing();
    assert!(plain > 1_000.0 && plain < 1_500.0, "{plain}");
    Ok(())
}

fn profile_default_landing() -> f64 {
    profile(400_000.0).landing_length()
}

#[test]
fn test_motion_taxi_in_time_can_be_set_at_touchdown() {
    let profile = profile(400_000.0);
    let mut motion = Motion::start(&profile, 600.0, 300.0);
    motion.set_taxi_in(900.0);
    while motion.phase() != Phase::TaxiIn {
        motion.advance_free(&profile, 1.0);
    }
    motion.advance_free(&profile, 90.0);
    // 1,500 m in 900 s.
    assert!((motion.along() - 150.0).abs() < 1e-6, "{}", motion.along());
}

#[test]
fn test_motion_flies_a_full_leg() -> Result<()> {
    let profile = profile(600_000.0);
    let flown = fly(&profile, 1.0)?;
    assert_eq!(
        flown.phases,
        [
            Phase::TaxiOut,
            Phase::Queue,
            Phase::TakeoffRoll,
            Phase::Climb,
            Phase::Cruise,
            Phase::Descent,
            Phase::Approach,
            Phase::Landing,
            Phase::TaxiIn,
            Phase::Arrived,
        ]
    );
    assert_eq!(
        flown.events,
        [Event::Takeoff, Event::Landing, Event::Arrived]
    );
    // 600 km at ~230 m/s plus taxi, climb and approach: about 75 minutes.
    assert!((3_600..6_000).contains(&flown.seconds), "{}", flown.seconds);
    let faf = flown.faf_altitude.unwrap_or_default();
    assert!((faf - profile.faf_altitude()).abs() < 30.0, "FAF at {faf}");
    Ok(())
}

#[test]
fn test_motion_caps_altitude_on_short_legs() -> Result<()> {
    let profile = profile(120_000.0);
    assert!(profile.cruise_altitude() < 11_000.0);
    let flown = fly(&profile, 1.0)?;
    assert!(flown.peak <= profile.cruise_altitude() + 1e-6);
    assert!(!flown.phases.contains(&Phase::Cruise) || flown.peak > 3_000.0);
    Ok(())
}

#[test]
fn test_motion_reports_kinematic_state() {
    let profile = profile(300_000.0);
    let mut motion = Motion::start(&profile, 600.0, 300.0);
    assert_eq!(motion.phase(), Phase::TaxiOut);
    assert!((motion.altitude() - 100.0).abs() < 1e-9);
    motion.advance_free(&profile, 300.0);
    assert!((motion.along() - 1_000.0).abs() < 1e-6, "half the taxi-out");
    assert!((motion.speed() - 2_000.0 / 600.0).abs() < 1e-9);
}

#[test]
fn test_motion_waits_in_the_queue_for_take_off() -> Result<()> {
    let profile = profile(600_000.0);
    let free = fly(&profile, 1.0)?;
    let waited = fly_with(
        &profile,
        1.0,
        |_| {},
        |request, now| request != Request::Takeoff || now >= 900,
    )?;
    assert!(
        waited.seconds >= free.seconds + 250,
        "{} vs {}",
        waited.seconds,
        free.seconds
    );
    assert_eq!(waited.events, free.events);
    Ok(())
}

#[test]
fn test_motion_holds_where_planned_until_released() -> Result<()> {
    let profile = profile(600_000.0);
    let free = fly(&profile, 1.0)?;
    let mut lowest_in_hold = f64::MAX;
    let held = fly_with(
        &profile,
        1.0,
        |motion| {
            motion.plan_hold(400_000.0, 2_500.0);
            motion.set_speed_factor(1.0);
        },
        |request, now| {
            if request == Request::LeaveHold {
                lowest_in_hold = lowest_in_hold.min(f64::from(now));
            }
            request != Request::LeaveHold || now >= 3_600
        },
    )?;
    assert!(held.phases.contains(&Phase::Holding), "{:?}", held.phases);
    assert!(
        held.seconds > free.seconds + 600,
        "{} vs {}",
        held.seconds,
        free.seconds
    );
    assert_eq!(held.events, free.events);
    let faf = held.faf_altitude.unwrap_or_default();
    assert!((faf - profile.faf_altitude()).abs() < 60.0, "FAF at {faf}");
    Ok(())
}

#[test]
fn test_motion_orbits_at_the_fix_until_cleared_to_approach() -> Result<()> {
    let profile = profile(600_000.0);
    let free = fly(&profile, 1.0)?;
    let mut first_ask = None;
    let gated = fly_with(
        &profile,
        1.0,
        |_| {},
        |request, now| {
            if request != Request::Approach && request != Request::LeaveHold {
                return true;
            }
            let first = *first_ask.get_or_insert(now);
            now >= first + 400
        },
    )?;
    assert!(gated.phases.contains(&Phase::Holding));
    assert!(gated.seconds >= free.seconds + 300);
    let faf = gated.faf_altitude.unwrap_or_default();
    assert!((faf - profile.faf_altitude()).abs() < 60.0, "FAF at {faf}");
    Ok(())
}

#[test]
fn test_speed_factor_slows_cruise_and_descent() -> Result<()> {
    let profile = profile(1_200_000.0);
    let free = fly(&profile, 1.0)?;
    let slow = fly_with(
        &profile,
        1.0,
        |motion| motion.set_speed_factor(0.85),
        |_, _| true,
    )?;
    assert!(f64::from(slow.seconds) > f64::from(free.seconds) * 1.08);
    assert_eq!(slow.events, free.events);
    Ok(())
}

#[test]
fn test_holding_motion_reports_its_state() {
    let profile = profile(600_000.0);
    let mut motion = Motion::start(&profile, 600.0, 300.0);
    assert!(!motion.has_hold());
    motion.plan_hold(10.0, 2_000.0);
    assert!(motion.has_hold());
    assert!((motion.speed_factor() - 1.0).abs() < 1e-12);
    assert!(motion.held_for() < 1e-12);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn test_motion_always_completes_in_order(
        distance in 40_000.0_f64..4_000_000.0,
        dt in prop::sample::select(vec![1.0, 10.0]),
    ) {
        let fail = |e: anyhow::Error| TestCaseError::fail(format!("{e:#}"));
        let profile = profile(distance);
        let flown = fly(&profile, dt).map_err(fail)?;
        prop_assert!(flown.phases.windows(2).all(|pair| pair[0] < pair[1]));
        prop_assert_eq!(flown.events, vec![Event::Takeoff, Event::Landing, Event::Arrived]);
        prop_assert!(flown.peak <= profile.cruise_altitude() + 1e-6);
        let faf = flown.faf_altitude.unwrap_or_default();
        prop_assert!((faf - profile.faf_altitude()).abs() < 60.0, "FAF at {}", faf);
        // Never much slower than cruising the whole way plus an hour.
        prop_assert!(f64::from(flown.seconds) < distance / 230.0 * 1.3 + 3_600.0);
    }
}

#[test]
fn test_a_landing_refused_on_short_final_goes_around_and_comes_back() -> Result<()> {
    let mut inputs = profile(400_000.0).inputs;
    inputs.go_around_length = 40_000.0;
    let profile = Profile::new(&inputs);
    let mut refused = 0;
    let flown = fly_with(
        &profile,
        1.0,
        |_| {},
        |request, _| {
            if request == Request::Land && refused == 0 {
                refused += 1;
                return false;
            }
            true
        },
    )?;
    assert_eq!(refused, 1);
    assert_eq!(
        flown.events,
        [
            Event::Takeoff,
            Event::GoAround,
            Event::Landing,
            Event::Arrived
        ]
    );
    let around = flown
        .phases
        .iter()
        .position(|&p| p == Phase::GoAround)
        .context("a go-around")?;
    // Back at the fix it holds for the next approach, then lands.
    assert_eq!(
        flown.phases[around - 1..around + 4],
        [
            Phase::Approach,
            Phase::GoAround,
            Phase::Holding,
            Phase::Descent,
            Phase::Approach
        ]
    );
    // It climbs to the missed-approach altitude above the field.
    assert!(flown.peak >= 450.0 + MISSED_APPROACH_HEIGHT - 1.0);
    Ok(())
}
