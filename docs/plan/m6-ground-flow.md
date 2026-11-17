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
| D151 | **Departure runway by the time until take-off.** Among the runway ends the mode allows for departures and the aircraft's length needs, a flight takes the one it is off soonest from: the taxi from its stand or the wait for the runway's turn (M4 D91), whichever is longer, then the turn onto its course (2 s per degree, and per kilometre the course lies off the centreline). Chosen when the leg is planned, without queues, and again when it asks to push back, with them. The cap on the learnt take-off interval rises from 5 to 15 minutes, so that the gate hold sees long queues |
| D152 | **Arrival runways stay by direction** (*changed in PR 2, 2026-11-17*; first: also by where the arrival will park). Two rules were built and measured: the landing slot each runway would give the flight at pushback, with and without the taxi to its stands. Both raise escapes at O'Hare, Atlanta and Phoenix, where they move landings onto runways whose taxi-in crosses busy traffic; the figures are under PR 2. The slot rule alone cuts airborne holding by 29 %; it may be tried again once the ground takes it |
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

*As built (PR 1):*

- **The stall (D150)** is one rule in `control.rs`: an aircraft asks for a taxiway lock when
  the place it would stop for it is within the look-ahead. Statistics count a *stall* when a
  taxi has stood five minutes with nobody to wait for; the runner warns of any and the region
  E test requires none. In the world run no stall was counted.
- **Parked aircraft (D148).** `ground/footprint.rs` holds the rule between two parked
  aircraft: their footprints keep the wingtip clearance of the larger one's code along an
  axis of either, with the mapping tolerance. The layout knows each stand's neighbours
  (within 165 m), the stand book the wingspan parked on each stand; a stand has room when it
  is free and its occupied neighbours keep clear. Variants by name stay as they were, an
  exclusion of their own, rather than folding into the rule: same-named alternatives can lie
  farther apart than their footprints reach.
- **A stand's own limit (D148, decided in the PR from the assets' spacing).** Gates lie 39–45 m
  apart at most airports; 2,040 stands lie 36–39 m from their neighbour and 3,500 nearer than
  17 m (alternative centrelines under names of their own). The limit is the largest code that
  blocks at most the gate on either side and four stands in all, stands within 17 m of each
  other counting as one gate and those within 17 m of the stand itself not at all. A gate in
  a row 40 m apart is thus rated code E. Nothing in the assets bounds a stand's depth.
- **What a stand was built for.** With such limits the tightest fit says nothing, and a first
  version let narrow-bodies drift onto wide-body stands (region E, seed 3: 52 escapes instead
  of 33). The layout therefore also works out each stand's natural size at load: the largest
  code that fits with every stand beside it in use. Stand choice takes the tightest stand
  built for the aircraft, as in M5; only an aircraft larger than that goes where it blocks
  the fewest stands beside it.
- **Taxiway access.** At load a stand's limit is lowered to what the taxiways from the runway
  exits admit (OSM wingspan limits on edges). Without it wide-bodies were given stands they
  could not taxi from and left Frankfurt uncontrolled, two queueing on the same spot.
- An aircraft at a waiting spot that finds a stand but no route to it no longer holds that
  stand while it waits.
- **Re-rating (D149):** `aerocosm-data rerate`; 34,933 of 38,554 limits changed, stands that
  take a narrow-body rose from 24,159 to 36,703. The format is unchanged.
- The viewer does not show stand sizes; nothing changed there.
- **The region E ground test passes on seeds 3, 7 and 11**, seed 7 for the first time since
  M5 PR 3.

Measured (36 h, seed 3 from 2026-07-06, against the diagnosis run):

| | Before | After |
|---|---|---|
| Stand waits | 8,378 | 1,436 |
| Stand shares | 2,909 | 796 |
| Escapes | 4,743 (3.0 % of taxis) | 4,481 (2.8 %) |
| Stalls | (not counted) | 0 |
| Longest taxi, min | 632 | 247 (O'Hare) |
| Airports with a taxi over 75 min | 17 | 12 |
| Departures on time | 76.5 % | 77.4 % |

Ankara: stand waits 464 → 0, escapes 236 → 14, taxi delay 25.7 → 3.0 min. Charlotte: stand
waits 441 → 0, escapes 194 → 31. Denver and Atlanta: stand waits to 0. Phoenix's longest taxi
fell from 632 to 85 minutes. The airports with runway queues did not improve or got a little
worse as more aircraft reach their stands and leave on time (O'Hare 572 → 607 escapes,
Boston 330 → 426, Madrid 287 → 322, Frankfurt 180 → 231): PR 2's subject. Stand waits remain
at Seattle (364), O'Hare (305), Phoenix (140) and Haneda (99), at peaks when the stands
usable at once are taken; O'Hare's shares rose (222 → 325).

Benchmark record (D156, against M5 PR 6's on the same container): tick medians 3.0–3.8 ms
(were 3.3–4.7), 95th percentiles 4.6–5.7 ms (5.0–7.3); the simulated day 404 s (469), with
fewer aircraft held on the taxiways. Dearer: choosing a stand at Frankfurt 260 µs (75 µs;
217 µs after a later saving that the record does not hold), building the ground networks
384 ms (235 ms, the neighbours and natural sizes), start-up 16.8 s (14.9 s, the warm start's
stand choices). The full suite passes: 846 tests.

## PR 2 — Runway choice (D151, D152)

- Core: departure runway chosen by time when the leg is planned and again at pushback.
- Gate hold on the real queue.
- Measured: escapes, queues and delays at the airports of the diagnosis.

*As built (PR 2):*

- **Leg planning** scores a departure runway in seconds: the taxi there along the route from
  its stand (30 minutes where there is none) or the wait for its turn, whichever is longer,
  plus the course term. Unmapped airports and airports without a mode keep the old choice.
- **At pushback** (`fleet/runways.rs`) the leg is planned again with each runway's turn from
  departure management; the runway already planned counts a minute less, so flights change
  only for a real gain, and a flight held at its gate looks again every five minutes, not at
  every retry. The departure's runway control follows the new runway; a landing slot booked
  at the destination stays.
- **Arrivals (D152) dropped after measurement**, agreed 2026-11-17:

  | 36 h, seed 3 | Escapes | Airborne holding, min | Departure delay, min | Over 75 min |
  |---|---|---|---|---|
  | PR 1 | 4,481 | 409,706 | 19.8 | 12 airports |
  | Departures only (as built) | 4,015 | 395,276 | 17.5 | 7 |
  | + arrivals by landing-slot wait | 4,554 | 289,793 | 18.5 | 6 |
  | + slot wait and taxi-in at 30 % | 4,848 | 351,325 | 18.8 | 6 |
  | + slot wait and taxi-in | 4,632 | 427,414 | 19.8 | 9 |
  | Departures by queue only, arrivals as above | 4,782 | 446,014 | 18.1 | 11 |

  A first rule that shared landings equally between a mode's runways sent as many to the far
  runway as to the near one (Atlanta 77 → 352 escapes) and was replaced by the slot wait
  before these runs.

Measured as built (36 h, seed 3 from 2026-07-06, against PR 1): escapes 4,481 → 4,015 (2.5 %
of taxis), stand shares 796 → 91, stand waits 1,436 → 881, go-arounds 1,780 → 1,528, longest
taxi 247 → 142 minutes, departure delay 19.8 → 17.5 minutes, arrival delay 24.0 → 21.8.
Frankfurt's escapes 231 → 90, Narita's 144 → 33, Madrid's 322 → 252; O'Hare handles 3,498
movements instead of 3,209 with taxi delay 21.1 → 15.4 minutes and no more stand waits (305
→ 0). Worse: Phoenix 79 → 166 escapes while handling 1,888 movements instead of 1,587, and
Atlanta 77 → 105. Ground delays at the gate for landing slots rose (134,467 → 165,742
minutes) as more flights leave on time into the same landing capacity.

Benchmark record (D156, against PR 1's): tick medians 2.8–3.6 ms (were 3.0–3.8), 95th
percentiles 4.6–6.6 ms; the simulated day 378 s (404). Start-up 20.0 s (16.8 s): every leg
with a choice of departure runways looks for a taxi route to each. Keeping the route found
for the chosen runway, and not looking where the mode offers one runway, brings it to 18.9 s
with equal fingerprints (after the record). The full suite passes: 849 tests.

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
| Faster taxiing makes separation at junctions harder | Controller distances derive from braking distance; fallback to 10 / 8 / 5 m/s (PR 4) |
| First-in-line clearances were tried in M4 without effect | M4's experiments ran under stand shortage and runway queues; they are repeated after PR 1 and PR 2, and the user sees the figures before more is built |
| Tick time rises with the neighbour rule or the speed profile | Benchmark record per PR (D156) |
