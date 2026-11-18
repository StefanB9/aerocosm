# M7 — Capacity and delays

**Goal:** the world's runways handle what real ones do, schedules stay within it, and delays
settle at real-world levels instead of growing through the week. M6 ended with three airports
over its 75-minute taxi limit, all overloaded (O'Hare's flights averaged 199 minutes of delay),
and the network's departure delay doubling after the first day. M7 removes the causes, by
general behaviour only (M5 D95): nothing per airport as input.

**Status:** approved 2026-11-17. Decisions continue M6's numbering.

## Diagnosis (2026-11-17)

A 72-hour world run (seed 3 from Monday 2026-07-06), instrumented for the run only, logged
every off-block with the time its aircraft was ready, and every take-off and landing.

1. **The capacity rule undercounts.** M5 D113 credits one parallel group, 40 movements an hour
   for a lane of one runway and 60 for a segregated pair; it leaves out every other runway.
   Denver gets 80 an hour from two of its six runways, Newark and LaGuardia 40 from one,
   Seattle 60 from two of three parallels, Heathrow 80 (declared 88), Frankfurt 100 (104).
   Where the schedule follows real demand and the rule allows half of real capacity, queues
   build all day: departures wait 84–103 minutes at LaGuardia and 85–91 at Newark before
   they leave, beyond their aircraft's own lateness.
2. **The simulation flies below the rule's capacity at some airports.** O'Hare flies 120–139
   movements an hour against the rule's 200, Boston 47–60 against 100, while Frankfurt (95
   against 100) and Denver (88 against 80) reach theirs. How the runway controllers use the
   runways at runtime loses capacity at some multi-runway airports; not yet diagnosed.
3. **Schedules peak above capacity where they are not coordinated.** An airport is coordinated
   when 7.5 % of its daily movements exceed 80 % of its hourly capacity (D113); hub banks then
   pile up unchecked. Boston, not coordinated, schedules up to 102 departures in one hour.
4. **Delay carries from leg to leg and nothing absorbs it.** Network departure delay is
   10.5 minutes on day 1 (a fresh start), 22.0 on day 2 and 20.2 on day 3, half of it the
   aircraft's late arrival (11.3 minutes), half waiting at the origin. O'Hare's day-2 average
   is 167 minutes. Turnarounds are scheduled close to the minimum and block times assume an
   unimpeded taxi, which since M6 is shorter than the real one (Frankfurt 6.5 minutes out).
5. **O'Hare runs short of stands** (953 waits, 786 shares in the 7-day run), possibly since
   estimates use 10.5 m/s (M6 PR 4); not yet diagnosed.

## Decisions

| # | Decision |
|---|----------|
| D157 | **Capacity references for validation.** Published capacities for about twenty airports, collected by hand into a test with their source per entry: FAA Airport Capacity Profiles (called and modelled rates in visual conditions; public domain) for US hubs, declared capacities from the coordinators' published parameters for European hubs. Validation only, like ADV for German movements (M5 D105); never an input (D95). The rule's capacity must lie within a tolerance of each (set in PR 1 from the spread of the references themselves, about ±20 %) |
| D158 | **Capacity rule from every runway in use.** Beyond the best parallel group, runway groups that can run at the same time add to the airport's capacity and to its runway modes: another parallel group whose runways do not cross the first's (Denver), and crossing or converging runways credited for departures only (Newark, LaGuardia, Boston). Lane rates are calibrated once against D157's references, the same rates for every airport. The mode lists every runway in use for arrivals and departures; runtime uses them all |
| D159 | **Runtime reaches capacity.** Where demand exceeds capacity, the busiest hours fly at least 90 % of it. O'Hare's and Boston's shortfall is diagnosed first (arrival spacing per runway, departure pacing, mixed mode, the share of each runway); the fix is general |
| D160 | **Coordination by the generated schedule.** An airport is coordinated when the schedule generated for it peaks above 80 % of its capacity in any hour, not by its daily average. Slots then hold every coordinated airport within capacity (M3 D54), banks included |
| D161 | **Schedule buffers.** Real schedules carry buffers, so delays settle: scheduled ground time adds a buffer by rule to the minimum turnaround, and scheduled taxi times add the additional time congestion costs on average, rising with the airport's load against its capacity and calibrated to EUROCONTROL's published additional taxi-out times (Frankfurt 3.5, Munich 3.4 minutes per departure in 2025). No airline recovery actions (swaps, cancellations) in M7 |
| D162 | **O'Hare's stands diagnosed** in the PR that changes schedules (PR 4); fixed by a general rule if the cause is general, otherwise recorded |
| D163 | **Acceptance.** The 7-day world run (seed 3 from 2026-07-06): departures on time (15 minutes) and average departure delay within the 2019 ranges of EUROCONTROL and BTS (about 75–80 % on time, 12–15 minutes average; the exact figures and sources are recorded in PR 1); day 7's average delay no more than 10 % above day 2's; no airport with 100 movements a day or more averaging over 60 minutes of delay; no taxi over 75 minutes; M6's limits still hold (escapes and stand shares under 2 %, no stalls); the region E ground test passes; German airports within the ADV tolerance; the capacity references within D157's tolerance; the benchmark's 95th-percentile tick under 8 ms. Then `dev` is promoted to `main` |
| D164 | **Determinism and benchmarks** as in M6 (D156): fingerprints change in each PR; each PR adds a benchmark record and checks the report before it opens |
| D165 | **Three close parallels land on two** (*added 2026-11-18*, after PR 4). A lane may hold three close parallel runways: the outer two take arrivals, the middle one departures, as at Seattle; its rate is calibrated against the references like the others. Before, a third close runway was not used (M5 D113) and Seattle landed on one |
| D166 | **Arrival runway by landing slot and taxi-in** (*added 2026-11-18*). About to push back, a flight weighs each arrival runway the mode allows by the wait for its earliest landing slot and the typical taxi from that runway's exit to the stands, so landings spread over a mode's arrival runways without sending them across the field (Boston: 15R 18–19 landings an hour, 14 nearly idle). The slot alone was tried twice and dropped (M6 PR 2, M7 PR 2); this is kept only if escapes stay under 2 % and O'Hare does not lose what PR 2 gained |

## PR 1 — Capacity rule (D157, D158)

- References collected (sources in the test), real-world delay figures for D163 recorded.
- The rule: every group of runways in use, crossing runways for departures, lane rates
  calibrated; runway modes list every runway; the world model's targets regenerate.
- Measured: the rule against the references; 36-hour world run (delays, holding, escapes).

*As built (PR 1):*

- **References** (`traffic/model/tests.rs`): the FAA's Airport Capacity Profiles 2014, Table 1,
  visual conditions, for its 30 core airports (the middle of the called and modelled rates;
  for arrival and departure priority the middle of both); Heathrow's busiest hour in ACL's
  Summer 2019 runway scheduling limits (44 arrivals, 46 departures); Frankfurt's 2019
  coordination parameter (104, from press reports of the coordinator's value). No sourced
  2019 figure was found for Munich or other European hubs. Tolerance agreed 2026-11-17: the
  median error under 12 %, the mean within ±10 %, every airport within 45 %.
- **Delay references for D163:** EUROCONTROL's CODA digest for 2019: 13.1 minutes average
  all-causes departure delay per flight; arrival punctuality (15 minutes) 76–81 % by quarter.
  US figures (BTS) are still to be recorded in PR 5.
- **The rule** (`traffic/model/runways.rs`): runways under 1,500 m do not count beside longer
  ones; parallels group and pair into lanes as before (M5 D113). The group handling most
  movements leads, whatever the prevailing wind: the climatological mean wind is light and
  decides only which way the runways run (Newark's main parallels lie 74° off it, and with
  them out of reach Newark had 40 an hour). Another group that crosses none in use adds
  0.4 of its lanes, arrivals and departures; one that crosses adds 15 departures an hour on
  its longest runway. Lanes: 45 an hour for one runway, 85 for a segregated pair. Fitting
  more parameters fitted noise (a lone runway worth more than an independent one).
- **Against the references:** median error 9 %, mean −1 %. Beyond 20 %: Frankfurt +42 % (its
  104 is a declared limit; its 18 West adds as a second group), Baltimore +42 %, Midway
  +35 %, Minneapolis −29 %, LaGuardia −28 %, O'Hare +26 %, Denver −25 %, Charlotte −25 %.
  Old against new: Atlanta 160 → 215 (221), Denver 80 → 211 (273), Newark 40 → 100 (97),
  LaGuardia 40 → 60 (83), Boston 100 → 120 (120.5), Seattle 60 → 85 (106), Heathrow 80 →
  90 (90), Frankfurt 100 → 148 (104).
- With 148 an hour Frankfurt is no longer coordinated by its daily average (D113); PR 3
  coordinates by the generated peaks.
- **Crossing runways are counted, not flown** (a change to D158 within PR 1): taking
  departures on a runway that crosses one in use put aircraft on an active runway where hold
  lines lie at its junctions (region E, seed 11: two aircraft at one spot at Helsinki). Their
  15 an hour stay in the rating the references check (`RunwayUse::rated`); schedules and
  coordination use the capacity the runway controllers fly. A first run that scheduled the
  crossing departures without flying them gave LaGuardia 272 minutes of delay. Flying them is
  PR 2's work (D159).
- **See and avoid**, two gaps the faster taxiing of M6 PR 4 had opened, found as more runways
  came into use: a one-tick step could pass right over another aircraft (the check compared
  only the distances before and after the move, now the nearest point of it); the probe for
  where a move ends ran straight along the heading, so in a turn towards another aircraft it
  judged the end metres too far off (region E, Copenhagen, 18.7 m; now the point along the
  taxi); and pilots judged others where the survey found them at the start of the tick (each
  move now updates the mover's sighting at once).

| 36 h, seed 3 | Escapes | Departure delay, min | Airborne holding, min | Go-arounds | Over 75 min |
|---|---|---|---|---|---|
| M6 (PR 4) | 2,042 (1.3 %) | 14.8 | 311,533 | 690 | 1 airport |
| Rule, crossing runways flown | 2,727 (1.7 %) | 15.8 | 582,919 | 1,072 | 1 |
| Crossing counted, scheduled, not flown | 1,922 (1.2 %) | 19.4 | 521,419 | 949 | 2 |
| As built | 1,785 (1.1 %) | 16.8 | 512,715 | 712 | 3 (Boston 103, O'Hare 87, Frankfurt 81) |

Where the rule added runways that are flown, delays fall: O'Hare 125 → 63 minutes, Denver
34 → 23, Frankfurt 21 → 15, Newark 90 → 86 while flying 1,663 movements instead of 1,297.
Worse: Boston 76 → 255 minutes (its 15L is under 1,500 m; it now flies 15R and 14 as two
independent runways, rated 90 an hour, and handles 1,161 movements against 1,451), LaGuardia
73 → 126 (coordinated at the 45 its one flown runway allows, against 40 before) and airborne
holding (Newark 1,065 → 42,643 minutes). The runway controllers fly less than the capacity at
these airports: PR 2's subject (D159). The full suite passes: 860 tests.

Benchmark record (D164, against M6 PR 4's): **tick time regressed**: medians 3.4–5.2 ms
(were 2.6–3.2), 95th percentiles 6.4–10.6 ms (4.9–5.9), above D107's 8 ms at 00 and 12 UTC.
Code this PR does not touch ran faster than in the previous record (geometry −21 %, viewer
frames −37 %), so the machine is not the cause. At 12 UTC flying takes 2.6 ms (1.2) with
4.5 % more aircraft airborne, ground control 2.4 ms (1.3) and taxiing 1.1 ms (0.5) with 7 %
more taxis: more aircraft in holding stacks and runway queues. Not yet split between the
behaviour change and the see-and-avoid fixes; to be resolved in PR 2, before D163's
acceptance. The simulated day takes 329 s (346).

## PR 2 — Runtime capacity (D159)

- Diagnosis of O'Hare and Boston: flown rate per runway against what the mode allows.
- The fix; measured: busiest hours against capacity at the hubs, 36-hour world run.

*As built (PR 2):*

**The tick regression of PR 1 does not reproduce.** Measured back to back on one machine,
`tick-12` runs at a median of 2.5–2.6 ms and a 95th percentile of 3.8–5.0 ms on PR 1's code,
on PR 1 without its see-and-avoid fixes and on M6's end; PR 1's code was the fastest. PR 1's
record (10.6 ms) was the machine's.

**Diagnosis.** A 36-hour run logged every touchdown and take-off clearance by runway. Every
busy runway flew below the rate the capacity rule gives it, at O'Hare and Boston as
elsewhere:

| Runway use | Flown at best, an hour | The rule's rate |
|---|---|---|
| Arrivals only (Newark 04L, Seattle 16L, Atlanta 08L, Denver 17L, O'Hare 10R) | 27–31 landings | about 42 |
| Departures only (Newark 04R, O'Hare 09R, Atlanta 09R, Denver 16R) | 33–38 take-offs | about 43 |
| Mixed, one runway (LaGuardia 13/31, Boston 15R/33L) | 30–45 together | 45 |

Three causes, each fixed by a general rule:

- **Landing slots went unused while aircraft held.** About 40 % of landings came at the
  wake minimum, the rest after gaps, a fifth longer than 2.5 minutes, with aircraft in the
  stack: a holder left at the slot it was given when booked, and slots freed ahead of it
  (late flights rebooking, go-arounds, stale provisional slots) stayed empty. Every half
  minute a holder now asks for the earliest slot it can still reach and leaves sooner if it
  moved up. Airborne holding fell from 512,715 to 193,745 minutes over 36 hours.
- **3 NM on final.** With aircraft waiting, landings came at the wake minima: 3 NM between
  aircraft of a class, more behind heavies, about 33–36 an hour with the traffic's mix.
  Busy hubs fly the reduced radar minimum of 2.5 NM where no wake minimum applies (ICAO
  PANS-ATM, runway occupancy under 50 s); agreed 2026-11-18. Heavy behind heavy keeps its
  wake minimum of 4 NM. Landing occupancy is 50 s (was 60).
- **Mixed runways starved their departures.** A take-off needed 90 s before the next
  touchdown, and arrivals came every 80–90 s; LaGuardia's departures were refused for 1,000–
  2,900 s an hour. On a runway used for both, landings are now at least 120 s apart and a
  take-off may roll a minute before the next touchdown, about 2 NM out: one departure after
  each landing (agreed 2026-11-18).

**Tried and dropped:**

- **Arrival runway by earliest landing slot** (retried from M6 PR 2 on the user's word):
  holding −40 %, delays slightly lower across the network, but escapes 1,793 → 2,419 and
  O'Hare 44 → 84 minutes of delay; arrival runways stay by direction (M6 D152), agreed
  2026-11-18.
- **Flying crossing runways for departures:** still unsafe. A departure for the crossing
  runway stands at its holding position where landings on the other runway leave it
  (region E, seed 11: Helsinki). Crossing runways stay counted in the rating, not flown.

**Ground safety at exits.** With the faster runways, the region E ground test found a
departure holding short to cross a runway at the very node where a landing then left it
(Helsinki, 0.0 m): ground control knew of a landing only once it taxied. A landing now
reserves the junction at its exit while it rolls out; an aircraft that would come to rest
in it stops short, traffic passing through goes on. Two broader versions were measured and
dropped: claiming the junction like a clearance (held all traffic through it for each
rollout: Newark 45 → 134 minutes) and taking only exits nobody stands near (no effect).

| 36 h, seed 3 | Escapes | Departure delay, min | Arrival delay, min | Airborne holding, min | Go-arounds |
|---|---|---|---|---|---|
| PR 1 | 1,785 (1.1 %) | 16.8 | 20.0 | 512,715 | 712 |
| + holders move up | 2,105 | 13.8 | 15.8 | 193,745 | 808 |
| + 2.5 NM, mixed runways | 1,793 | 12.3 | 14.8 | 243,684 | 995 |
| + arrivals by earliest slot (dropped) | 2,419 | 12.0 | 13.8 | 147,430 | 1,034 |
| As built (+ exit reserved) | 1,958 (1.2 %) | 12.5 | 14.9 | 264,129 | 1,031 |

Against PR 1: O'Hare 63 → 46 minutes of average delay, LaGuardia 126 → 28, Newark 86 → 43,
Denver 23 → 23, Frankfurt 15 → 16. The network's departure delay (12.5 minutes) is now
within EUROCONTROL's 2019 figure (13.1). Still overloaded: Boston (245 minutes; its landings
crowd 15R while 14 stands nearly idle, arrival runways being chosen by direction) and
Seattle (151 minutes, 101,340 minutes of holding: one arrival runway, its third close
parallel unused by the lane rule). D159's 90 % of capacity in the busiest hours was not
measured hour by hour; the runways' rates are now set by the separations above.

Benchmark record (D164, against PR 1's): tick medians 2.0–2.7 ms (were 3.4–5.2), 95th
percentiles 3.4–4.4 ms (6.4–10.6), well under D107's 8 ms; the simulated day 277 s (329),
start-up 16.5 s (21.4). Untouched code ran faster too (viewer frames −23 %), so part of the
gain is the machine; less holding means fewer aircraft airborne at once. The full suite
passes: 863 tests.

## PR 3 — Coordination (D160)

- Coordination from the generated schedule's peak; slots hold it.
- Measured: peak hours against capacity at every coordinated airport, round trips dropped
  (M5: 1.4 %), movements against targets.

*As built (PR 3):*

- **Every airport is held within its capacity** (`traffic/model/mod.rs`). A slot binds only
  in an hour the schedule would exceed, so coordinating by the generated peak (D160) and
  coordinating every airport give the same schedule: airports whose peak lies between 80 %
  and 100 % of capacity are untouched either way. The daily-average rule is gone; Frankfurt
  (148 an hour since PR 1) is held again. Slot tests still hold every capacity; round trips
  dropped stay under 2 % (`test_few_slots_are_dropped`), German movements within the ADV
  tolerance. Cargo still leaves at night where slots allow: at least 90 % in the test, where
  before every flight did between airports without slots.
- **Arrival and departure limits, tried and dropped.** The total does not stop a schedule
  from sending an airport more arrivals than its arrival runways land (Seattle: 85 an hour
  in all, one runway landing). Separate hourly limits from the lanes (a mixed runway 30 and
  30, a segregated pair 42 and 43) cut Seattle's holding from 131,338 to 80,300 minutes and
  Boston's delay from 252 to 220 minutes, but pushed O'Hare's banks into fewer hours: its
  stands ran short (198 shares) and its delay rose 42 → 125 minutes; network departure delay
  12.3 → 13.0. Totals only, agreed 2026-11-18.

| 36 h, seed 3 | Escapes | Departure delay, min | Arrival delay, min | Holding, min | Flights |
|---|---|---|---|---|---|
| PR 2 | 1,958 | 12.5 | 14.9 | 264,129 | 155,886 |
| Every airport within capacity (as built) | 1,889 | 12.3 | 14.8 | 285,191 | 155,971 |
| + arrival and departure limits (dropped) | 1,952 | 13.0 | 14.8 | 231,742 | 155,149 |

The effect is small: the airports still overloaded (Boston 252 minutes, Seattle 137) are
held by runway use, not by schedule peaks (see PR 2). Newark's delay 43 → 73 minutes, O'Hare
46 → 42, Denver 23 → 23. The full suite passes: 863 tests.

Benchmark record (D164, against PR 2's): tick medians 2.7–3.9 ms, 95th percentiles 5.2–6.7
ms, under D107's 8 ms; the simulated day 383 s (277), start-up 23.4 s (16.5). Viewer frames,
untouched, rose 72 %: the machine ran slower. Slot books at every airport may add to
start-up; not separated from the machine.

## PR 4 — Buffers and stands (D161, D162)

- Buffers on turnarounds and taxi times; O'Hare's stands diagnosed.
- Measured: delay by day of a 72-hour run, scheduled against flown taxi times at Frankfurt
  and Munich.

*As built (PR 4):*

- **Delay by day.** The statistics count departures, on-time share and delay per day of the
  run; the runner logs a `day` line for each (D163 compares day 7 with day 2).
- **Taxi buffer** (`schedule/trips/context.rs`). A mapped airport's scheduled taxi times add
  the time congestion costs: five minutes out and two in per unit of its load (movements a
  day against sixteen hours of capacity). Frankfurt gets 2.9 minutes out, Munich 3.8, against
  EUROCONTROL's additional taxi-out times of 3.5 and 3.4 minutes per departure (2025).
  Unmapped airports keep their calibrated default of 12 and 6 minutes.
- **Turnaround buffer, tried and dropped.** Scheduled turnarounds a fifth above the shortest
  (A320 42 minutes for 35): aircraft then stood at their stands until the padded departure,
  and O'Hare's stands overflowed (368 waits, 229 shares; delay 39 → 145 minutes). That is
  D162's finding: O'Hare's mapped stands hold its traffic at the shortest turnarounds, not at
  padded ones; at the shortest it has no shortage (0 shares in the 72-hour run below).
  Agreed 2026-11-18.

| 72 h, seed 3 | Day 1 | Day 2 | Day 3 | On time (dep / arr) | Arrival delay | Holding, min | O'Hare |
|---|---|---|---|---|---|---|---|
| No buffers | 9.4 | 17.6 | 19.0 | 78.5 / 73.2 % | 18.6 | 579,628 | 39 min, 0 shares |
| Both buffers (dropped) | 8.7 | 17.3 | 21.5 | 80.4 / 77.5 % | 17.7 | 480,096 | 145 min, 229 shares |
| Taxi buffer (as built) | 9.4 | 18.6 | 19.2 | 78.9 / 76.0 % | 17.7 | 579,319 | 45 min, 0 shares |

(Days: average departure delay, minutes.) Arrivals come in on time more often; departure
delay is unchanged, and **it still grows after the first day** with or without buffers: Boston
(304 minutes) and Seattle (228) stay overloaded by their runway use (PR 2) and send their
delay on through the network. D163's limit of day 7 within 10 % of day 2 is in doubt. The
full suite passes: 865 tests.

Benchmark record (D164, against PR 3's): tick medians 2.4–3.2 ms, 95th percentiles 4.4–6.3
ms, under D107's 8 ms; the simulated day 330 s (383), start-up 22.4 s (23.4). Within the
machine's swing; schedules alone changed.

## PR 5 — Runway use at Boston and Seattle (D165, D166)

- Lanes of three close parallels; arrival runway by slot and taxi-in.
- Measured: 72-hour run (delay by day, Boston, Seattle, O'Hare, escapes), capacity
  references, region E ground test.

*As built (PR 5):*

- **Three close parallels land on two** (D165, `traffic/model/runways.rs`). A lane holds up to
  three runways closer than 1,035 m to its first; with three, the outer two land and the
  middle one takes off, 105 an hour (Seattle's published 106; the capacity references still
  pass). The rule changes six airports: Seattle, Philadelphia, and four small ones (Phoenix-
  Mesa, Sheppard, Adana, Ulanqab).
- **Arrival runway by landing slot and taxi-in** (D166), tried and dropped. A flight about to
  push back weighed each arrival runway by its earliest landing slot and the typical taxi
  from its exit to the median stand, from any origin. With D165: holding 579,319 → 247,458
  minutes, Boston 304 → 265 minutes of delay, but O'Hare 45 → 161 minutes and its escapes
  525 → 2,094 (network 3,908 → 5,576): landings moved onto runways whose taxi-in O'Hare's
  ground cannot take, as twice before. Reverted by D166's own condition.

| 72 h, seed 3 | Days 1 / 2 / 3 | Holding, min | Escapes | Seattle | Boston | O'Hare |
|---|---|---|---|---|---|---|
| PR 4 | 9.4 / 18.6 / 19.2 | 579,319 | 3,908 | 228 min | 304 min | 45 min |
| D165 and D166 (D166 dropped) | 8.3 / 17.8 / 19.2 | 247,458 | 5,576 | 158 | 265 | 161 |
| D165 (as built) | 9.7 / 16.9 / 19.4 | 323,107 | 4,013 | 144 | 264 | 83 |

(Days: average departure delay, minutes; airports: average departure delay.) Seattle's
holding falls from 292,569 to 16,741 minutes. O'Hare's runways are untouched by D165; its
72-hour delay has swung between 39 and 83 minutes across PR 4's and this PR's runs as the
schedule's draws shift with any capacity, and stays recorded as such (agreed 2026-11-18).
Boston (264 minutes) remains overloaded: its landings still crowd 15R. The full suite passes:
865 tests.

## PR 6 — Acceptance (D163)

- The 7-day run, benchmark record, `docs/known-issues.md`, README; promotion to `main` on the
  user's word.

## Definition of done

- D163's limits hold in the 7-day world run.
- The full test suite passes, with the capacity references and the region E ground test.
- No per-airport input was added.

## Risks

| Risk | Mitigation |
|------|------------|
| More capacity at hubs moves the queues elsewhere (stands, taxiways) | M6's limits are part of the acceptance; each PR runs the 36-hour world run |
| References differ in what they count (called against declared, visual against all-weather) | Visual-conditions rates for the FAA, declared hourly capacity for Europe; the tolerance reflects their spread |
| Coordination by peaks drops more round trips and lowers movements against targets | Dropped trips and movements against targets are measured in PR 3; slots move trips before they drop them |
| Buffers hide runtime problems | Buffers come last (PR 4), after capacity and runtime are measured without them |
| Real-world delay figures include weather and ATC causes the simulation lacks | The acceptance uses ranges, not single values; causes not modelled are noted |
