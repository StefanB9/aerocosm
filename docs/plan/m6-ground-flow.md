# M6 — Ground flow

**Goal:** the world's ground traffic meets M4's limits. M5 ended with escapes at 3.3 % of
taxis over a world week, 23 airports with a taxi over 75 minutes, and the region E ground
test failing on seed 7 (plan M5 D143, D146). M6 removes the causes, by general behaviour
only (M5 D95): nothing per airport.

**Status:** approved 2026-11-16. Decisions continue M5's numbering.

## Diagnosis (2026-11-16)

Two instrumented world runs (seed 3 from Monday 2026-07-06, 36 h and 24 h, ground at every
gated airport) dumped every taxi over 40 minutes and, at twelve hubs, stand use and runway
queues every half hour. In 36 hours 211 airports had an escape; 25 airports had 72 % of
them. Four causes, in order of effect:

1. **Stands rated too small.** The importer gives each stand the largest ICAO code letter
   whose footprint keeps the wingtip clearance to every neighbour at once (M5 D119). Gates
   38–40 m apart, built for the 737, fail code C (36 m + 4.5 m) by a metre and are rated
   24 m. Stands that fit an A320 or 737: Ankara 6 of 62, Charlotte 28 of 177, Seattle 58 of
   180 (one for a wide-body), Denver 160 of 246 (7 for a wide-body). Ankara never used more
   than 6 stands; its arrivals waited at spots on the taxiways (464 stand waits in 36 h,
   escapes at 49 % of movements). Stand waits: O'Hare 457, Seattle 450, Charlotte 441,
   Denver 433, Phoenix 304.
2. **Departures pile onto one runway.** A leg's runways are fixed when it is planned, by
   direction of flight alone. Boston had 15–16 aircraft taxiing to 33L and none to 32;
   Frankfurt 10–20 for 25R at a take-off every 200 s; Madrid 19–21 for 32R on routes of
   8 km. Queues of 40–65 minutes form at the holding positions; the gate hold (M4 D91)
   underestimates them because the learnt interval is capped at 5 minutes. Arrivals land
   far from their stands the same way (O'Hare's taxi-ins are 4–8.5 km).
3. **A controller stall.** An aircraft stops short of the junction before a taxiway it has
   not locked, but asks for the lock only within 420 m of that taxiway. Where the taxiway
   starts farther beyond the junction it stands with nothing to wait for, so neither the
   deadlock rule nor the 30-minute rule applies: Phoenix, 632 minutes.
4. **Slow, long taxis.** Taxi speed is a flat 5 m/s. An unimpeded taxi of 8 km takes 27
   minutes, and O'Hare's arrivals pass 40–70 junctions at the lowest priority. Aircraft
   stay on the taxiways two to three times as long as real ones.

The patterns diagnosed in M4 (short stubs, order inversion at a runway entry;
`docs/known-issues.md`) remain as the background of 1–2 %.

## Decisions

| # | Decision |
|---|----------|
| D147 | **Scope.** All four causes and the M4 junction fixes, largest effect first: stands and the stall, runway choice, junctions, taxi speeds, acceptance. Each PR is measured with a 36-hour world run (seed 3 from 2026-07-06) against the one before; the figures go into this plan |
| D148 | **Stand sizes with dynamic neighbours.** A stand's limit is the largest aircraft that fits its own geometry with its neighbours empty. Whether a stand is free for an aircraft depends on the aircraft actually parked on the neighbours: the two footprints (M5 D119: a square as long as the wingspan, back from the stand point) must keep the wingtip clearance of the larger aircraft's code. A wide-body thus takes the gates beside it and two narrow-bodies fit where the mapping allows, as at real multi-use stands. This replaces the fixed code letter and generalises stand variants (M4), which become the case of footprints that always overlap. Stand choice (M5 D120) is otherwise unchanged |
| D149 | **Limits re-rated from the assets.** A new `aerocosm-data` command recomputes every stand's own limit from the positions and headings in the ground assets and writes them back; the importer uses the same rule from now on. The quality gate (M5 D118) is not re-run: the set of airports stays. The asset format version rises if the stored fields change |
| D150 | **The stall fixed.** An aircraft that stops for a taxiway it has not locked asks for that lock from where it stands, however far ahead the taxiway starts. A test reproduces Phoenix's layout; a guard counts aircraft that stand still with nothing to wait for, in the ground tests and in the runner's report |
| D151 | **Departure runway by direction, taxi distance and queue.** Chosen when the pushback is planned, among the runway ends the mode allows for departures and the aircraft's length needs: the score adds the direction of flight (as now), the taxi distance from its stand and the runway's expected queue (M4 D91). The gate hold uses the runway's real queue: the cap on the learnt interval goes |
| D152 | **Arrival runway also by where it will park.** Chosen when the leg is planned, as now, among the ends the mode allows for arrivals: the score adds the direction it comes from (as now) and the taxi distance from the runway's exits to the stands that fit it (to the stands its airline holds, where it holds some). Landing slots and approach sequencing follow the chosen runway as today; the share per runway is checked against the mode's capacity so that no runway is asked for more landings than its lane takes |
| D153 | **Junction fixes (M4's intended fix).** A junction is cleared only to the first aircraft in line for it; two junctions too close to stand between (a short stub) take one clearance, the arrival holding before the first. Measured on the region E test, seeds 3, 7 and 11 |
| D154 | **Taxi speed profile.** 15 m/s on straight taxiways, 8 m/s on apron taxilanes and through gentle bends, 5 m/s in sharp turns and on stand lead-ins; aircraft speed up and brake at 1 m/s² instead of stopping within a tick. The limits come from the edge's kind and the turn at each node of the route. The controller's distances that assume 5 m/s (look-ahead, the gap to through traffic, pushback clearance) are derived from speed and braking distance. Scheduled taxi times (M4) use the same profile |
| D155 | **Acceptance.** The 7-day world run (seed 3 from 2026-07-06): escapes under 2 % of taxis, stand shares under 2 %, no taxi over 75 minutes, no aircraft standing with nothing to wait for; the region E ground test passes on seeds 3, 7 and 11; German airports stay within the ADV tolerance (M5 D105); the benchmark's 95th-percentile tick stays under 8 ms (M5 D107). Then `dev` is promoted to `main` |
| D156 | **Determinism and benchmarks.** Every PR changes behaviour, so fingerprints (M5 D137) change in each; each PR adds a benchmark record and the report is checked for regressions before the PR opens |

## PR 1 — Stands and the stall (D148–D150)

- Core, test first: the stall (D150) with its guard statistic.
- Core: a stand's own limit and the neighbour rule; `StandBook` answers "free for this
  wingspan" from the neighbours' parked aircraft; variants fold into it; stand choice,
  waiting spots, remote parking and tow-back use it. Property tests: two aircraft are never
  parked with overlapping footprints unless counted as a stand share.
- Data: the re-rating command and the importer's rule (D149); assets regenerated.
- Viewer: stands drawn and labelled from the new limit where they show a size.
- Measured: stand waits, shares, escapes and longest taxi at the diagnosis's airports.

## PR 2 — Runway choice (D151, D152)

- Core: departure runway chosen at pushback planning; the leg's departure procedure follows
  it. Arrival score with the stand distance; per-runway landing share against capacity.
- Gate hold on the real queue.
- Measured: queue lengths per runway at Boston, Frankfurt, Madrid, Denver; taxi-in lengths
  at O'Hare; arrival holding (flow control) must not get worse.

## PR 3 — Junctions (D153)

- First in line; one clearance for a short stub.
- The region E test passes on all three seeds.

## PR 4 — Taxi speeds (D154)

- Speed limit along a route from edge kinds and turns; acceleration and braking in the
  drive step; controller distances from speed.
- Scheduled taxi times from the profile; schedules regenerate.
- Measured: unimpeded taxi times at Frankfurt and Munich against published averages; escapes
  and see-and-avoid stops. If it raises escapes, the speeds drop to 10 / 8 / 5 m/s before
  anything else is tried, and the user decides.

## PR 5 — Acceptance (D155)

- The 7-day run, the benchmark record, `docs/known-issues.md`, README; M5's definition of
  done updated; promotion to `main` on the user's word.

## Definition of done

- D155's limits hold in the 7-day world run.
- The full test suite passes, the region E ground test on every seed included.
- No per-airport input was added.

## Risks

| Risk | Mitigation |
|------|------------|
| A stand's own limit is too generous where OSM maps alternative centrelines of one gate under different names | The neighbour rule still keeps parked aircraft apart; shares and stand waits are measured per airport after re-rating |
| Arrival runways by stand distance overload one runway | The share per runway is held to the mode's capacity (D152); arrival holding is measured in PR 2 |
| Faster taxiing makes separation at junctions harder | Controller distances derive from braking distance; fallback to 10 / 8 / 5 m/s (PR 4) |
| First-in-line clearances were tried in M4 without effect | M4's experiments ran under stand shortage and runway queues; they are repeated after PR 1 and PR 2, and the user sees the figures before more is built |
| Tick time rises with the neighbour rule or the speed profile | Benchmark record per PR (D156) |
