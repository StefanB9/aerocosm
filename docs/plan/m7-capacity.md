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

## PR 1 — Capacity rule (D157, D158)

- References collected (sources in the test), real-world delay figures for D163 recorded.
- The rule: every group of runways in use, crossing runways for departures, lane rates
  calibrated; runway modes list every runway; the world model's targets regenerate.
- Measured: the rule against the references; 36-hour world run (delays, holding, escapes).

## PR 2 — Runtime capacity (D159)

- Diagnosis of O'Hare and Boston: flown rate per runway against what the mode allows.
- The fix; measured: busiest hours against capacity at the hubs, 36-hour world run.

## PR 3 — Coordination (D160)

- Coordination from the generated schedule's peak; slots hold it.
- Measured: peak hours against capacity at every coordinated airport, round trips dropped
  (M5: 1.4 %), movements against targets.

## PR 4 — Buffers and stands (D161, D162)

- Buffers on turnarounds and taxi times; O'Hare's stands diagnosed.
- Measured: delay by day of a 72-hour run, scheduled against flown taxi times at Frankfurt
  and Munich.

## PR 5 — Acceptance (D163)

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
