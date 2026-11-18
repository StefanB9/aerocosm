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
| D171 | **Cancellation at the base, as the last resort.** A round trip whose departure from its base is still expected past the recovery threshold, after quick turns, swaps and spares, is cancelled as a whole; the aircraft stays and takes its next trip. Expected delay counts the aircraft's lateness and the flow-control delay its departure is given (M3 D59). The threshold, one figure for the whole world, is calibrated so the 7-day run cancels within D172's range; about 3 hours from the diagnosis. Never at the far end: a tail away from its base flies home |
| D172 | **Acceptance.** M7's D163 limits, unchanged: departures on time and average departure delay within the 2019 ranges (about 75–80 %, 12–15 minutes), day 7's average delay no more than 10 % above day 2's, no airport with 100 movements a day or more averaging over 60 minutes, no taxi over 75 minutes, escapes and stand shares under 2 %, no stalls, the region E ground test, the ADV tolerance for German airports, the capacity references, the 95th-percentile tick under 8 ms. Added: cancellations 1–2.5 % of scheduled departures; late aircraft 35–50 % of departure delay. Delay of cancelled flights is not counted (as in CODA and BTS); cancellations are. Then `dev` is promoted to `main` |
| D173 | **The viewer shows recovery.** Airport boards show cancelled flights as "Cancelled"; the flight panel says when its aircraft was swapped or is a spare; the airport's flow status counts the day's cancellations and swaps |
| D174 | **Determinism and benchmarks** as in M7 (D164): recovery decisions are rule-based and drawn, if at all, from the operations stream; fingerprints change in each PR; each PR adds a benchmark record and checks the report before it opens |

No new data: the references are the published 2019 figures already used in M7 (CODA, BTS),
recorded with their sources in PR 1's test.

## PR 1 — Delay by cause (D167)

- Statistics: late aircraft, airline and flow-and-ground delay per departure, per airport,
  per day and for the network; the runner's `punctuality` and `day` lines report them.
- The references (CODA, BTS) in a test with their sources.
- Measured: the 36-hour world run against M7 (fingerprint unchanged: statistics only).

## PR 2 — Quick turns and cancellations (D168, D171)

- Quick turns for tails behind schedule.
- Expected departure delay at the base; cancelled round trips; cancellations in statistics
  and the runner's output (per airport, per day, network).
- The threshold calibrated on 36-hour and 72-hour world runs.
- Measured: delay by day and cause, cancellations, the D172 limits on a 72-hour run.

## PR 3 — Tail swaps (D169)

- Aircraft apart from rotations in operations, flight views and statistics.
- Swaps at the base within the pool; swaps in statistics.
- Measured: swaps, cancellations and delay against PR 2 (cancellations should fall).

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
