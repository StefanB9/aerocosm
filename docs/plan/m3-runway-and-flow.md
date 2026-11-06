# Plan: M3 Runway & Flow

Status: **approved** (2026-11-06)

## Goal

Runways become shared, separated resources. Coordinated schedules respect declared airport
capacities; arrivals are sequenced, slowed down or sent to holding stacks; congested
destinations hold departures on the ground with slots; aircraft fly simple departure and
arrival procedures instead of turning on the spot. Delays emerge from randomness and knock-on
effects at realistic levels and are reported. Ground movement stays straight-line until M4.

## Decisions log (continues the M2 plan)

| # | Decision | Rationale |
|---|----------|-----------|
| D53 | The former M3 scope is split: **M3 "Runway & flow"** (this plan) and **M4 "Airport ground"**: OpenStreetMap stands and taxiways for the German commercial airports in a separate ODbL-licensed asset, stand assignment, pushback, taxi routing with full ground control (spacing, hold-short, intersection priority, deadlock-free reservations). Supersedes D47's "stands and taxiways arrive in M3" | Each milestone stays reviewable; M4 needs a new data pipeline |
| D54 | Slot coordination in schedule generation: coordinated airports carry declared capacities (movements per rolling hour and per 15 minutes, arrivals and departures where published); trip placement shifts departures until both ends fit | Real German hubs are slot-coordinated; unconstrained banks (FRA: 47 movements in 15 min vs ~27 declared) would run 30–60 min late daily |
| D55 | Curated runway operating modes (arrival and departure runways per flow direction) and capacities for the coordinated German airports and the busiest edge hubs, with sources, in the traffic file; other airports use the best-aligned runway in mixed mode. The flow is chosen from the wind, fixed at 250° until weather arrives | FRA-style modes (18 departures only) cannot be derived from geometry |
| D56 | ICAO wake turbulence separation with the existing categories: on final by distance (e.g. Medium behind Heavy 5 NM, minimum 3 NM), converted to time at approach speeds; departures by time (1–3 min by category); a runway is occupied from threshold to vacating, and mixed-mode departures need a gap before the next arrival | No catalog changes; standard values |
| D57 | An arrival manager per sequenced airport plans landing times first-come-first-served by estimated time over a ~30 min horizon, per arrival runway; delay up to a speed-control budget is absorbed by slowing down, the rest in holding stacks: 2–4 synthesised fixes per airport about 35 NM out on the main inbound directions, racetracks stacked 1,000 ft apart | Realistic delay absorption; real fix names wait for the SID/STAR milestone |
| D58 | ATFM ground delay: at off-block, a departure bound for a sequenced airport books a provisional landing time; if the expected airborne delay exceeds 10 minutes, it is held at the gate with a calculated take-off time (CTOT) | Mirrors the European network manager; keeps holding stacks plausible |
| D59 | Simple procedures: departures climb on runway heading to ~2,000 ft above the field (at least 4 NM) and turn onto course on a standard-rate arc; arrivals join final through an intercept point so the turn onto final is at most 90°, with a downwind leg when arriving from ahead. Paths become chains of great-circle segments and arcs | No more U-turns after take-off or sharp turns at the final fix |
| D60 | Gates stay unlimited until M4 (stands from OSM) | Capacity belongs with real stands |
| D61 | Sequencing and capacities apply at the German airports with movement targets; edge airports fly their curated runway modes but are not capacity-limited, because most of their real traffic is not simulated | Constraining a partial picture would be arbitrary |
| D62 | Delay statistics: on-time performance (≤ 15 min), average departure/arrival delay, holding minutes and CTOTs per airport; reported by the CLI and shown in the viewer | Makes the flow model checkable |
| D63 | Shared resources are resolved in a fixed order (time, then tail index); the 15k-aircraft benchmark stays under 1 ms per tick with runway control on | Determinism and the scale target from M2 |

## PR 1 — Data and slot coordination

- **Traffic file v2:** per coordinated airport `capacity` (per hour and per 15 min; arrival
  and departure limits where published) and `runway_modes` (flow name, arrival runways,
  departure runways), sourced from the German slot coordinator's coordination parameters and
  airport publications; edge hubs get runway modes only. `docs/data-sources.md` updated.
- **Core types:** `Capacity`, `RunwayMode`; validation against the world's runway
  designators.
- **Schedule:** capacity bins per airport (15-min and rolling hour, local time) filled during
  trip placement; a departure that would overflow either end is shifted in 5-minute steps
  (prefer nearby times, keep hub banks roughly intact), and dropped only as a last resort.
- **Tests:** capacity never exceeded in the weekly template; movement targets still within
  tolerance; banks still visible; determinism.

## PR 2 — Procedures and runway modes in flight paths

- Paths as a chain of legs (great-circle segments and constant-radius arcs) walked by
  distance, still without trigonometry per tick (D48).
- Departure: runway heading to the turn point, arc onto course. Arrival: course to the
  intercept point (or downwind and base), arc onto the final approach course, final.
- Runway choice from the airport's active mode (arrival vs departure runways); the
  best-aligned rule remains for uncurated airports.
- **Tests (proptests):** continuity across legs, turn radius from speed and bank, final
  intercept ≤ 90°, no reversal after lift-off.

## PR 3 — Runway control and arrival management

- **Runway controller** per sequenced airport: runway occupancy, wake separation on final and
  between departures, mixed-mode gaps, departure queue at the holding point (take-off
  clearance replaces the free-running roll).
- **Arrival manager:** ETA-based sequence per arrival runway, landing time slots, speed
  control within a budget, holding stack assignment (fix, level), exit from the stack in
  sequence.
- Motion gains a `Holding` phase (racetrack walked by distance) and a speed override.
- **Tests:** no separation or occupancy violations (property tests over random arrival
  streams), FCFS fairness, holding entry/exit order, determinism; benchmark stays < 1 ms.
  Measured: 962 µs per tick with 15,369 airborne (traffic and capacities scaled 23-fold, so the
  runways are heavily congested); the 7-day run takes about 6 s in release.

## PR 4 — Ground delay and statistics

- ATFM: provisional landing bookings at off-block, CTOT assignment beyond 10 minutes, gate
  holds; booking updates when the flight enters the AMAN horizon.
- Statistics: per airport and network OTP, average delays, holding minutes, CTOT counts.
- CLI: `run` reports them next to movements; warnings when capacities are exceeded.
- **Acceptance run:** 7 days: movements within 5 % of targets; no capacity or separation
  violations; FRA and MUC average arrival holding a few minutes; network OTP in a plausible
  range (roughly 70–90 %).

## PR 5 — Viewer

- Holding aircraft fly visible racetracks; stack fixes drawn at airport zoom.
- Runway in use: arrival/departure arrows on the active runways at airport zoom.
- Airport panel: active mode, arrival queue and holding counts, departure queue, current
  delays and OTP; boards show estimated times from the sequence and CTOTs.
- Flight panel: "Holding at FRA-NW, FL90, expected out 08:42", "CTOT 09:15", speed control.
- Browser build checked.

## Definition of done

- `aerocosm run --duration 7d` within targets, capacity- and separation-clean, with plausible
  delays and holding; benchmark < 1 ms at 15k aircraft.
- The viewer shows departures turning onto course, arrivals joining final, holding stacks at
  busy times, and delays on boards and panels; native and web.

## Risks

| Risk | Mitigation |
|------|------------|
| Declared capacities and runway modes hard to source precisely | Published coordination parameters where available; documented estimates otherwise |
| Slot shifting distorts calibration | Tests keep movement targets; shift before dropping |
| Holding/sequencing logic complex and stateful | Pure planners (sequence in, decisions out) tested in isolation; property tests for separation |
| Performance with interactions | Controllers act on events (arrivals entering horizon, runway release), not every tick |
| Network access for data research | Web search for sources; OSM downloads only needed in M4 (environment change requested then) |
