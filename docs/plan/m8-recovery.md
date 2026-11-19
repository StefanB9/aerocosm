# M8 — Airline recovery

**Goal:** a late aircraft no longer stays late for the rest of the week. Airlines in the
simulation recover the way real ones do: they turn late aircraft faster, swap aircraft, bring
in spares and, as the last resort, cancel. M7 ended with the network's departure delay growing
from 16.9 minutes on day 2 to 30.0 on day 7 (plan M7 PR 6) because nothing ever ends a chain
of late legs (M7 D161 excluded recovery). By general behaviour only (M5 D95): nothing per
airport or airline as input.

**Status:** approved 2026-11-18. Decisions continue M7's numbering.

## Diagnosis (2026-11-18)

The 7-day world run (seed 3 from Monday 2026-07-06, M7's code), instrumented for the run only,
split every departure's delay by cause: the time its aircraft was ready after schedule (late
aircraft), the delay drawn per leg (`ops::tail::draw_delay`), and what remains (flow control,
waits at the stand for a way out). It also counted, every 12 hours, how far behind schedule
each of the 94,010 tails was. The run's punctuality matched M7 PR 6 exactly.

| Day | Departure delay, min | Late aircraft | Drawn | Flow and ground | Departures 3 h+ late on arrival of their aircraft |
|---|---|---|---|---|---|
| 1 | 9.7 | 2.6 | 3.9 | 3.2 | 224 |
| 2 | 16.9 | 6.8 | 4.9 | 5.2 | 745 |
| 3 | 19.4 | 8.9 | 5.1 | 5.4 | 1,053 |
| 4 | 19.0 | 8.6 | 5.2 | 5.1 | 963 |
| 5 | 22.1 | 10.4 | 5.2 | 6.5 | 1,049 |
| 6 | 26.0 | 13.3 | 5.4 | 7.4 | 1,444 |
| 7 | 29.9 | 13.9 | 5.4 | 10.5 | 755 (half a day) |

1. **The share of late-aircraft delay is realistic; its tail is not.** Late aircraft cause
   45–50 % of departure delay, against 43 % in Europe (EUROCONTROL CODA 2019: 5.7 of 13.1
   minutes reactionary) and about 40 % in the US (BTS 2019, late-arriving aircraft). The
   drawn delay stays flat at about 5 minutes.
2. **A small set of tails falls ever further behind.** Tails more than 24 hours behind
   schedule: 0, 77, 138, 201, 241, 353, 401 at the end of each day; more than 3 hours: about
   1,000 (1.1 %). Their next legs leave mostly from Boston (about 390), O'Hare (about 300) and
   Seattle (about 60): airports whose traffic exceeds what they fly, so their backlog grows
   daily, and with it the flow-control share (3.2 → 10.5 minutes).
3. **The flights a cancellation rule would take are about real cancellation rates.** About
   1 % of departures leave with their aircraft 3 hours late; airlines cancelled 1.7 % of
   flights in Europe (EUROCONTROL CODA 2019, operational cancellations) and 1.9 % in the US
   (BTS 2019) in 2019.
4. **The rotations allow recovery without stranding aircraft.** Every tail flies round trips
   from its base (M2 D42), so a trip cancelled at the base leaves the aircraft where its next
   trip starts. Tails share a pool (airline, base, aircraft type): 48,713 pools, 66 % of tails
   in a pool of two or more, 14 % in pools of 11 or more.

## Decisions

| # | Decision |
|---|----------|
| D167 | **Delay by cause.** Statistics split each departure's delay into late aircraft (the aircraft ready after schedule), airline (the drawn delay beyond that) and flow and ground (the rest), per airport, per day and for the network; the runner reports them. Validation against CODA 2019 (late aircraft 43 % of departure delay) and BTS 2019 (about 40 % of arrival delay minutes) |
| D168 | **Quick turns.** A tail behind schedule may turn in 80 % of its minimum turnaround (M2 timetable: 25, 35, 60 and 90 minutes become 20, 28, 48 and 72), never below 20 minutes. Tried first, before any other action |
| D169 | **Tail swaps at the base.** If a tail is still expected to leave its base past the recovery threshold (D171), and another tail of its pool is parked at the base, ready, with its own next departure later, the two exchange rotations from that point: the ready aircraft flies the late trip, the late aircraft takes the other's. Aircraft (what parks, taxis and flies) and rotations (what is scheduled) become separate, so a flight view names both. Only at the base, where both aircraft start round trips |
| D170 | **Spare aircraft.** Pools of 10 tails or more get one spare per 20 tails (rounded down; about 1 % of the fleet). A spare has no rotation; it parks remote at its base (M5 D125) so it takes no contact stand while idle, and when no swap is possible it takes over a late trip; the late aircraft becomes the spare once it arrives. Spares are part of the generated schedule, from the same seed |
| D171 | **Cancellation at the base, as the last resort.** A round trip whose departure from its base is still expected past the recovery threshold, after quick turns, swaps and spares, is cancelled as a whole; the aircraft stays and takes its next trip. Expected delay counts the aircraft's lateness and the flow-control delay its departure is given (M3 D59). The threshold, one figure for the whole world, is calibrated so the 7-day run cancels within D172's range; about 3 hours from the diagnosis (*calibrated to 90 minutes in PR 2*). Never at the far end: a tail away from its base flies home |
| D172 | **Acceptance.** M7's D163 limits, unchanged: departures on time and average departure delay within the 2019 ranges (about 75–80 %, 12–15 minutes), day 7's average delay no more than 10 % above day 2's, no airport with 100 movements a day or more averaging over 60 minutes, no taxi over 75 minutes, escapes and stand shares under 2 %, no stalls, the region E ground test, the ADV tolerance for German airports, the capacity references, the 95th-percentile tick under 8 ms. Added: cancellations 1–2.5 % of scheduled departures; late aircraft 35–50 % of departure delay. Delay of cancelled flights is not counted (as in CODA and BTS); cancellations are. Then `dev` is promoted to `main` |
| D173 | **The viewer shows recovery.** Airport boards show cancelled flights as "Cancelled"; the flight panel says when its aircraft was swapped or is a spare; the airport's flow status counts the day's cancellations and swaps |
| D174 | **Determinism and benchmarks** as in M7 (D164): recovery decisions are rule-based and drawn, if at all, from the operations stream; fingerprints change in each PR; each PR adds a benchmark record and checks the report before it opens |

No new data: the references are the published 2019 figures already used in M7 (CODA, BTS),
recorded with their sources in PR 1's test.

## PR 1 — Delay by cause (D167)

- Statistics: late aircraft, airline and flow-and-ground delay per departure, per airport,
  per day and for the network; the runner's `punctuality` and `day` lines report them.
- The references (CODA, BTS) in a test with their sources.
- Measured: the 36-hour world run against M7 (statistics only).

*As built (PR 1):* `ops::DelayCauses` splits each departure's delay when it leaves the gate:
late aircraft is how long after schedule its aircraft was ready (never more than the delay),
airline is the delay drawn for the leg beyond that, and flow and ground is the rest. Airport,
day and network statistics keep the totals and report averages per departure; the runner's
`punctuality`, `day` and `movements` lines report them. The references are constants beside
the split (`LATE_AIRCRAFT_REFERENCES`: CODA 2019, 5.7 of 13.1 minutes; BTS 2019, 39.7 % of
delay minutes), with a test that D172's range holds both.

| 36 h, seed 3 | Departure delay, min | Late aircraft | Airline | Flow and ground | Late aircraft share |
|---|---|---|---|---|---|
| Day 1 | 9.7 | 2.7 | 3.9 | 3.2 | 27 % |
| Day 2 (12 h) | 18.6 | 6.8 | 4.8 | 7.0 | 36 % |
| 36 hours | 12.5 | 4.0 | 4.2 | 4.4 | 32 % |

The run is M7's to the minute (155,890 departures, 12.5 minutes, 81.2 % on time). The
first day starts with every aircraft on time, so the share of late aircraft builds over it;
D172's 35–50 % is judged on the 7-day run, where the diagnosis found 45–50 % from day 3.
The benchmark's fingerprint changes because it hashes the network's statistics, which now
hold the split; airborne and taxiing counts are equal at every tick measured. Tick medians
2.5–3.0 ms, 95th percentiles 4.7–5.7 ms (this container ran faster than for M7 PR 5's
record). The full suite passes: 873 tests.

## PR 2 — Quick turns and cancellations (D168, D171)

- Quick turns for tails behind schedule.
- Expected departure delay at the base; cancelled round trips; cancellations in statistics
  and the runner's output (per airport, per day, network).
- The threshold calibrated on 36-hour and 72-hour world runs.
- Measured: delay by day and cause, cancellations, the D172 limits on a 72-hour run.

*As built (PR 2):* `ops::tail::ready_at` turns a tail that would otherwise be late in 4/5 of
its minimum turnaround, never under 20 minutes (D168). `ops::fleet::recovery` decides at two
points, both only where the tail's current leg leaves its base: when the tail reaches the
gate, a round trip it would be ready for more than the threshold after schedule is
cancelled, both legs, and the next trip is looked at the same way; at off-block, a
departure that flow control (M3 D59) or departure management (M4 D91) would hold past the
threshold cancels there, gives back its landing slot and stays parked for its next trip.
Cancellations are counted at the scheduled departure of each leg, per airport, per day and
for the network; the runner reports them with `cancelled_pct`.

| 72 h, seed 3 | Days 1 / 2 / 3, min | Average | Late aircraft | Cancelled | Boston | Seattle | O'Hare |
|---|---|---|---|---|---|---|---|
| M7 (no recovery) | 9.7 / 16.9 / 19.4 | about 15 | 45 % (day 3) | – | 264 min | 144 | 83 |
| 3 h, on arrival only | 8.8 / 15.1 / 13.2 | 12.3 | 25 % | 0.28 % | 246 | 140 | 44 |
| 3 h | 8.0 / 11.4 / 11.0 | 10.1 | 20 % | 0.64 % | 139 | 73 | 29 |
| 2 h | 7.5 / 10.1 / 10.2 | 9.2 | 18 % | 0.95 % | 119 | 53 | 28 |
| **90 min (as built)** | 7.3 / 9.5 / 9.4 | 8.7 | 16 % | 1.26 % | 105 | 44 | 25 |
| 90 min, quick turns 9/10 | 7.4 / 9.7 / 10.5 | 9.1 | 20 % | 1.41 % | 108 | 46 | 48 |
| 90 min, no quick turns | 7.7 / 10.2 / 10.3 | 9.4 | 25 % | 1.34 % | 104 | 46 | 30 |

(Days and airports: average departure delay; M7's average and share are from its 72-hour run
and the diagnosis.) Delay no longer grows: day 3 is below day 2 at every threshold. Holding
falls from M7's 323,107 minutes to 272,845, stalls stay 0, stand shares 441.

What cancelling does to D172's ranges was measured, not assumed: the long chains of late
legs it removes were most of the late-aircraft minutes, so with cancellations in range the
average delay (8.7 minutes) falls below the 12–15 of 2019 and the share of late aircraft
(16 %) below 35–50 %. Quick turns are not the cause: without them the share is 25 %, the
delay 9.4. What remains is the airline's delay drawn per leg (about 5 minutes, flat since
M2) and flow control. **Decision (2026-11-19):** quick turns stay at 4/5 (D168), the
threshold is 90 minutes, and the delay and share are recorded below range, to be judged on
the 7-day run in PR 6 after swaps and spares.

Boston (105 minutes) and Seattle (44) still average most delay; O'Hare's longest taxi is 147
minutes and Frankfurt's 98 (M7's 7-day run: 282 and 124). The full suite passes: 883 tests.

Benchmark record (D174): tick medians 3.1–3.9 ms, 95th percentiles 4.8–6.3 ms, under D107's
8 ms; the simulated day 348 s. Against PR 1's record ticks are 5–48 % slower, but evenly
across every part, flying included, which recovery does not touch, while start-up is 11 %
faster: the container, not the code (PR 1's record was taken on an unusually fast one).

## PR 3 — Tail swaps (D169)

- Aircraft apart from rotations in operations, flight views and statistics.
- Swaps at the base within the pool; swaps in statistics.
- Measured: swaps, cancellations and delay against PR 2 (cancellations should fall).

*As built (PR 3):* operations keep aircraft apart from rotations: each aircraft flies the
rotation `rotation_of` gives it, each rotation is flown by the aircraft `aircraft_of` gives
it, and `FlightView` carries both, `airframe()` (an `Id<Airframe>`, the physical aircraft:
sprites, taxi routes) and `tail()` (the rotation). The refactor alone changes nothing: a
36-hour world run logs identical lines before and after it. The pool is the airline, base
and aircraft type (D169); within it zone, turnaround, callsign, wake, wingspan and cargo are
the same, so a swap changes nothing else.

When a trip from the base would cancel (the tail ready more than the threshold after
schedule), it first looks for a swap: another aircraft of the pool, parked at the base, whose
own next trip from the base is due later and which is ready by the late trip's schedule plus
the threshold. Of those, the one whose trip is due latest is taken. The two exchange
rotations; the ready aircraft flies the trip, and the late one takes the later trip, which
is looked at in turn (and may cancel). One swap is tried per trip; if none is found, the trip
cancels as in PR 2. Swaps are counted at the base, per airport and for the network.

| 72 h, seed 3 | Days 1 / 2 / 3, min | Average | Late aircraft | Cancelled | Swaps | Boston | Seattle | O'Hare |
|---|---|---|---|---|---|---|---|---|
| PR 2 | 7.3 / 9.5 / 9.4 | 8.7 | 16 % | 1.26 % (3,940) | – | 105 min | 44 | 25 |
| **PR 3** | 7.3 / 9.9 / 9.6 | 8.9 | 18 % | 1.12 % (3,516) | 307 | 98 | 44 | 23 |

Swaps take 11 % of cancellations away. Most of the rest are departures held at the gate by
flow control, which a swap cannot help (the other aircraft would be held the same), most of
Boston's and Seattle's. The flights that now fly rather than cancel add a little delay
(8.9 against 8.7 minutes, late aircraft 18 %). Holding 269,670 minutes, stand waits 2,194
(PR 2 2,286), stalls 0; O'Hare's longest taxi 120 minutes, Frankfurt's 96. The full suite
passes: 884 tests.

Benchmark record (D174): tick medians 3.0–4.4 ms, 95th percentiles 4.7–7.4 ms, under D107's
8 ms; the simulated day 397 s. The fingerprints change because flight views and statistics
carry new fields. Against PR 2's record tick 00 and 06 are 5–14 % faster and ticks 12 and 16
20–50 % slower, in every part alike, flying included, and start-up is 19 % slower: the
container, not the code.

## PR 4 — Spares (D170)

- Spares in the generated schedule; parked remote at the base; taking over late trips.
- Measured: stand waits and shares, remote parking at O'Hare and Boston, cancellations.

## PR 5 — Viewer (D173)

- Boards, flight panel, flow status; screenshots; web build.

## PR 6 — Acceptance (D172)

- The 7-day run, benchmark record, `docs/known-issues.md`, README; promotion to `main` on the
  user's word.

## Definition of done

- D172's limits hold in the 7-day world run.
- The full test suite passes, with the delay references, the capacity references and the
  region E ground test.
- No per-airport or per-airline input was added.

## Risks

| Risk | Mitigation |
|------|------------|
| Cancellations hide overload instead of showing it (Boston, O'Hare cancel their way under the limits) | Cancellations are reported per airport; an airport cancelling far above the network's rate is recorded in known issues |
| Swaps and spares move aircraft between stands and remote parking at full airports | M6's limits (stand shares, escapes) are part of the acceptance; PR 4 measures O'Hare and Boston |
| Separating aircraft from rotations touches every part of operations | Done once, in PR 3, with the fingerprint unchanged before swaps are switched on |
| Real cancellations include weather, crew and technical causes the simulation lacks | The range is wide (1–2.5 %); causes not modelled are noted |
| Quick turns make tight schedules look easier than they are | Only for tails behind schedule, and never below 20 minutes |
