# Plan: M2 Living Sky

Status: **approved** (2026-11-05)

## Goal

Real-looking air traffic over Germany: calibrated, rotation-based schedules flown by ~20 real
airlines between German airports and European edge airports, plus overflights; every flight
moves through its phases along great circles and real runways; the viewer shows aircraft,
flight details and departure boards. Deterministic per seed; ready for M3 (runway operations,
separation, delay propagation).

## Decisions log (continues the M0–M1 plan)

| # | Decision | Rationale |
|---|----------|-----------|
| D28 | Calibrated gravity: per-airport daily movement targets from official statistics; a gravity model (catchment population × distance) distributes them over routes | Realistic totals and route mix without proprietary timetables |
| D29 | Scope: German airports + ~20 European edge airports + overflights between edge airports crossing Germany | Overflights are ~43 % of German airspace traffic (8,616 flights/day vs 4,888 arrivals+departures) |
| D30 | Rotation-based schedules: each tail flies a daily rotation from its base; hub banks at FRA/MUC; night curfews | Aircraft continuity; M3 delays propagate along rotations |
| D31 | Ground: boarding/taxi are timed phases at the airport; take-off and landing rolls run along the real runway; runway in use from a prevailing-wind rule (250°) | Looks right at airport zoom; real runway ops are M3 |
| D32 | ~20 real airlines in a hand-curated catalog (`assets/airlines/airlines.json`) with bases, fleets (catalog types) and sources | Covers most real traffic; matches D8 |
| D33 | Viewer aircraft: top-down silhouettes in four size classes, rotated to heading, coloured by phase, short fading trail, constant screen size | Readable and characterful without per-airline art |
| D34 | Weekly template: one schedule per weekday pattern, repeated weekly; daily variation (turnaround, taxi, small delays) from the `Operations` stream | Realistic week rhythm, deterministic |
| D35 | M2 ships as four PRs: data → schedules → flight operations → viewer | Reviewable units |
| D36 | Kinematics are incremental: per leg an orthonormal great-circle basis, position = `a·cos θ + b·sin θ`; altitude/speed from piecewise profiles | M0 benchmark: from-scratch interpolation ~200 ns is too slow for 15k aircraft at 1000× |
| D37 | Catchment population per commercial/edge airport is computed by the importer from Natural Earth places (within 100 km) and stored in the world file | Core stays independent of the basemap; reproducible |
| D38 | Airlines are catalogued with bases (weights), network style and a fleet type mix; the number of tails per airline is an output of schedule generation, reported for sanity checks (replaces curated counts in D32) | Germany-related fleet counts are hard to source, and fixed counts could make the official targets unschedulable |
| D39 | Four cargo carriers (DHL/European Air Transport, UPS, FedEx, AeroLogic) fly mostly at night, mainly LEJ/CGN | The official scheduled movements of those hubs are largely cargo |
| D40 | Local time via `jiff` with the bundled IANA database (`tzdb-bundle-always`): every airport has an IANA zone; curfews, daily curves and banks are local | Exact DST everywhere; bundled data keeps results identical on all platforms and in wasm |
| D41 | An estimated `cargo_share` per cargo airport (labelled as estimate) splits its movement target between cargo and passenger carriers | ADV totals mix both; no per-airport cargo movement statistic is published in the same series |
| D42 | Nightstops allowed: flights are generated as round trips from an airline base whose return may be the next morning; rotations run over a cyclic week (a Sunday-night trip returns on the template's Monday) and continuity is validated across the wrap | Realistic first/last waves for low-cost and leisure carriers while keeping tail positions consistent |
| D43 | The weekly template is expressed in each tail's base-local time (minutes since local Monday 00:00); a concrete week is placed in UTC via the base's zone. Curfews at the far end use the zone offset of a reference week | Airlines publish local times; one template serves the whole year; offsets that differ between zones by season (e.g. Turkey has no DST) shift far-end times by at most an hour |
| D44 | Start mode is a per-run option: `Warm` (default) places every tail mid-leg at its nominal position at the start instant; `Cold` activates only legs departing after the start, tails parked at their next origin | A full sky from the first frame for the viewer, a clean ramp-up for analysis |
| D45 | `Simulation::new(config, scenario)`: a validated `Scenario` bundles the world with optional traffic inputs (aircraft, airlines, targets); the schedule is generated inside from the seed, with zone offsets taken at the start instant | Seed + inputs fully determine a run; a world-only scenario keeps the static viewer and tests working |
| D46 | Determinism is per platform: std floating-point maths, identical runs on the same build and target | No extra dependency or cost in the hot loop; revisit for networked or cross-platform replays |
| D47 | Ground positions without stand/taxiway data: boarding at the airport reference point, taxi as a straight line between it and the runway threshold (out) or the end of the landing roll (in) | Visible ground traffic at airport zoom; stands and taxiways arrive in M3 |
| D48 | Operations run per tail (a dense `Vec` of tails, each flying its weekly rotation week after week) rather than per flight; airborne state stores distance flown along precomputed segments, and positions are evaluated only when viewed | No activation churn; the per-tick update has no trigonometry, which keeps 15k aircraft well under 1 ms |

## Calibration facts (to be cited in `docs/data-sources.md`)

- German airspace: ~8,616 flights/day in 2024 (EUROCONTROL European Aviation Overview); 4,888
  daily arrivals+departures at German airports in 2025 → ~3,700 overflights/day.
- Airport movements 2025: ADV-Monatsstatistik 12/2025, cumulative January–December,
  "Linien- und Charterverkehr (an+ab)": FRA 449,896, MUC 319,039, BER 182,007, DUS 149,240,
  HAM 109,086, CGN 103,956, STR 76,338, LEJ 65,684, … (2,161,690 movements in total).
- DFS: ~3.07 million IFR movements in 2025; peak day 18 July 2025 with 10,220 flights.

## PR 1 — Data: edge airports, airlines, traffic targets

- **Importer:** a curated list of ~20 edge airports inside the basemap (e.g. EGLL, LFPG, EHAM,
  EBBR, LEBL, LEMD, LIRF, LIMC, LSZH, LOWW, LKPR, EPWA, EKCH, ESSA, ENGM, EFHK, LHBP, LROP,
  LTFM, EIDW) imported from OurAirports with runways as `AirportKind::Edge`.
- **Catchment population** (D37): importer sums Natural Earth places within 100 km of each
  Major/Regional/Edge airport; new optional `catchment` field in the world file (format
  version 2, version-1 files rejected with a clear message).
- **Airline catalog** (D32, D38, D39): `Airline { icao, iata, name, callsign, kind (passenger
  / cargo), network (hub / point-to-point / foreign hub), bases: [{airport, weight}], fleet
  mix: [{type, share}], colour, source }`; ~20 passenger carriers plus DHL, UPS, FedEx and
  AeroLogic — Lufthansa, Lufthansa CityLine, Eurowings,
  Discover, Condor, TUIfly, SunExpress, Ryanair, easyJet, Wizz Air, Air France, KLM, British
  Airways, Swiss, Austrian, SAS, Turkish Airlines, LOT, Iberia, ITA; sources cited.
- **Traffic targets:** `assets/traffic/germany.json` — per German commercial airport: daily
  scheduled+charter movements (ADV 2025 "Linien- und Charterverkehr" ÷ 365), IANA time zone,
  local curfew window, hub flag and local bank times; IANA zones of the edge airports; weekday
  factors;
  overflight target; a coarse German airspace polygon (~25 vertices) for overflight
  detection.
- **Core types:** `AirlineCatalog`, `TrafficTargets`, `Zone` (IANA zone via `jiff`, D40) with
  local-time helpers; validated; guard tests load the files.

## PR 2 — Core: schedule generation (`aerocosm-core::schedule`)

Deterministic from the `Scheduling` stream; output is a weekly `Schedule` of flights grouped
into tails (D42, D43). Stages live in separate modules (`demand`, `routes`, `timetable`,
`rotation`, `overflight`), each tested on its own; `generate` composes them.

1. **Demand:** gravity weights `P_i·P_j / d_ij^β` between airport pairs (β ≈ 1.2, minimum
   distance ~150 km, no domestic pairs closer than that).
2. **Fitting:** iterative proportional fitting scales the symmetric route matrix so each
   airport's daily movements match its target (±2 %).
3. **Airlines:** routes go to airlines by base/hub presence (Lufthansa group at FRA/MUC hubs,
   Eurowings/LCCs at their bases, foreign carriers from their edge hubs).
4. **Aircraft type:** by stage length and demand from the airline's fleet, respecting range and
   minimum runway length.
5. **Timetables:** departure times follow a daily curve (morning/evening peaks) or hub banks;
   curfews respected; block time from distance, type speeds and taxi allowances.
6. **Rotations:** flights chained into tail rotations per base (the tail count per airline is
   derived and reported, D38) with minimum turnaround by size
   class (regional 25 min, narrow-body 35 min, wide-body 90 min); every tail ends the day where
   the next day starts.
6a. **Cargo:** cargo carriers' flights are placed at night in local time (D39); where a curfew
   closes the night (Frankfurt), they move to the day.
7. **Overflights:** edge-pair routes whose great circle crosses the German airspace polygon,
   gravity-weighted and scaled to the overflight target; foreign carriers, own rotations at
   edge airports.

Callsigns: airline ICAO + flight number (e.g. `DLH400`). Proptests: totals within tolerance of
targets, rotation continuity (next departure where the last arrived, turnaround ≥ minimum),
curfews, range/runway feasibility, determinism (same seed ⇒ identical schedule).

## PR 3 — Core: flight operations and kinematics (`aerocosm-core::ops`)

- **Phases:** Scheduled → Boarding → TaxiOut → TakeoffRoll → Climb → Cruise → Descent →
  Approach → Landing → TaxiIn → Arrived → (turnaround) next leg.
- **Runway in use:** per airport the runway end best aligned with the prevailing wind (250°),
  among runways long enough for the type (D31).
- **Kinematics** (D36): take-off roll along the runway; great circle from the departure end to
  a final approach fix 10 NM out on the arrival runway's extended centreline; straight-in
  final; landing roll. Altitude from climb/descent rates with top of descent computed from
  distance; speeds per phase from the aircraft type.
- **Operations randomness:** turnaround, taxi and small departure delays from the `Operations`
  stream; late arrivals delay the next leg of the rotation.
- **Simulation integration:** `Simulation` owns the schedule and the active flights (dense
  `Vec`, flights activated before STD, retired after arrival); a read-only view API
  (`FlightView`: id, callsign, airline, type, position, altitude, heading, ground speed, phase,
  origin, destination, times).
- **Performance:** criterion benchmark with 15k concurrent airborne flights; target < 1 ms per
  tick in release (~60 ns per aircraft), result recorded in the PR.
- **CLI:** `aerocosm run` reports flights flown, peak airborne and per-airport movements, and
  checks them against the targets.

## PR 4 — Viewer

- **Aircraft** (D33): four silhouette meshes (regional, narrow-body, wide-body, A380), constant
  screen size, rotated to heading, coloured by phase (ground, climb, cruise, descent),
  fading trail of the last ~2 minutes; level of detail hides ground traffic at country zoom.
- **Flight panel:** callsign, airline, type, origin → destination, scheduled/estimated times,
  phase, flight level, ground speed, progress; hover/select like airports, callsign label for
  the selected flight; follow-camera toggle.
- **Departure/arrival boards:** the airport panel lists the next departures and arrivals with
  time, flight, destination/origin and status (scheduled, boarding, departed, delayed +min).
- **Top bar:** airborne count.
- Browser build checked with `cargo xtask web build|serve`.

## Definition of done

- `aerocosm run --world assets/world/germany.world.json --duration 7d` simulates a week;
  per-airport movements within 5 % of targets; ~3,700 overflights/day; no rotation breaks.
- The viewer shows realistic traffic at country and airport zoom; flights selectable; boards
  update live; native and web.
- 15k-aircraft benchmark under target; full pre-merge checklist green on every PR.

## Risks

| Risk | Mitigation |
|------|------------|
| Calibration numbers vary between sources | Use official ADV/EUROCONTROL publications only; cite them |
| Schedule generator complexity | Pure, staged pipeline; each stage property-tested |
| Fleet data accuracy | Cited, rounded figures; flagged for review like the aircraft catalog |
| Rendering 1,500+ aircraft in debug wasm | Measure in PR 4; batch meshes if needed |
