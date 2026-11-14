# Known issues

Open problems found in the simulation, kept until they are fixed. Each entry says what
happens, how often, why, and the intended fix.

## Ground control (plan M4 PR 4, PR 6, PR 7)

### Escapes at about 2 % of taxis

**What:** an aircraft that gives way in a waiting cycle for a minute, or waits half an hour
outside a runway queue, is sent another way if there is one (D94) and otherwise cleared
regardless (D75, D83). Each escape counts as a ground conflict and the flight view marks it
(`FlightView::ground_conflict`).

**How often (plan M5 PR 3):** in the ground test's morning bank on region E (260 airports
under ground control), 33, 49 and 27 escapes in about 2,350 taxis for seeds 3, 7 and 11
(1.4 %, 2.0 %, 1.1 %): seed 7 exceeds the test's limit of one in fifty by one escape, and the
test fails there. A 4-hour world run (seed 3, 836 airports) has 314 in 14,610 taxis (2.1 %).
M4's 7-day run on 22 German airports had 2.3 %.

**Why (diagnosed on seed 7):** nearly every escape ends a cycle of two aircraft waiting for
each other, in two patterns:

- *Short stubs* (about 15 of 49): two junctions closer together than an aircraft needs to stand
  clear of both (30–60 m, e.g. Munich's junctions 202/203, twice in the morning). An arrival
  stops at the hold for one with its tail still in the other, while a departure stands in the
  first, needing the second.
- *Order inversion at a runway entry* (about 10): a departure 400 m back is cleared onto the
  runway junction (Frankfurt's junction 98 at 25R, four times) while the one in front, which
  needs it first, waits for it.

Two experiments made no difference and were reverted: keeping the close junctions beyond a
cleared one for it (escapes 40/61/40, worse) and letting aircraft pass a waiting one down to
21 m (32/50/27).

**Intended fix:** clear a junction only to the first aircraft in line for it; one clearance
for both junctions of a short stub, the arrival holding before the first; published or
curated one-way flows (D84).

### Taxi delays at Frankfurt

**What:** Frankfurt's average taxi delay is 3.2 min over 7 days (target about 2 min; PR 4:
15 min); its departure delay 17.7 min, movements on target. Elsewhere taxi delays are 2–3
min.

**Why:** clearances wait for gaps in the through traffic on the main taxiways, and waiting
cycles at the apron exits hold aircraft for a minute before one gives way.

**Intended fix:** flows (D84).

### Landings roll on to later exits

**What:** a landing whose planned exit's way off is taken rolls on to a later exit. A 24 h
run (seed 0, PR 7) has 82 % of landings on their planned exit, 14 % on a later one and 4 %
with none clear, waiting on the runway (the next arrival then goes around, D90). The planned
roll (touchdown at 300 m, braking at 1.8 m/s² to 15 or 8 m/s) is not yet checked against
real exit use at Frankfurt or Munich.

**Intended fix:** compare planned exits with published exit use or ADS-B taxi tracks.

### Stand shares

**What:** an aircraft that finds neither a free stand nor a free waiting spot shares a stand
(D69, D127), counted as a stand share.

**How often (plan M5 PR 3):** 0.4–0.6 % of taxis in the morning bank on region E; 1.4 % in a
4-hour world run. Remote parking (D125) and remote boarding at waiting spots (D127) took
Frankfurt and Munich from 7–8 % to nearly none. Most remaining shares are arrivals whose
planned exit has no way off at touchdown, and airports whose stands the spacing rule rates
smaller than their largest freighters (D119).

**Decision:** reported as a statistic.

### Benchmark above the 1 ms target

**What:** the 15k-aircraft benchmark takes about 3.8 ms per tick with ground control
(PR 7; dev before PR 6: 3.5 ms), above the D74 target of 1 ms.

**Why:** at 23-fold traffic the mapped airports are gridlocked (Frankfurt at about 29,000
movements a day): hundreds of aircraft taxi at each hub, each checked against the others,
and most route searches fail after exploring the whole network.

**Decision (M4 PR 3):** D74 amended; revisit when the world grows beyond Germany.

## The world model (plan M5 PR 2)

### Ground control under the 2019 world traffic

**Resolved in plan M5 PR 3** except for escapes (above): generic stand choice (D120), remote
parking and boarding (D125, D127), a gap in the arrivals for long-waiting crossings and the
runway occupied while a landing rolls out (D128), and pacing at unmapped queues restored M4's
limits for stand shares, separation and the longest taxi in the morning bank on region E.

### Runway capacity by rule is short at some busy airports

**What:** schedule generation drops round trips that find no slot at a coordinated airport.
Newark, Mumbai, LaGuardia, San Francisco, Seattle, Las Vegas, Sydney and Melbourne get
10–30 % fewer movements than their targets; worldwide 1.4 % of round trips are dropped.

**Why:** the capacity rule (D113) gives a lane of one runway 40 movements an hour and counts
only the parallel group that takes the prevailing wind. Busy single runways handle 45–55
(Mumbai, Gatwick), close parallels run dependent approaches, and crossing runways add
departures (Newark's 11/29, Zurich's 16) that the rule leaves out.

**Intended fix:** published declared capacities as premium-tier data, or a rule that
credits crossing runways for departures; until then the figure stays conservative.

### The fallback traffic model underestimates the largest unobserved hubs

**What:** airports outside well-covered countries take their traffic from the fitted model
(D109): Beijing Capital gets about a third of its real traffic, the busiest Chinese,
Indonesian and Nigerian airports similar.

**Why:** catchment, income, runways and size explain hub status poorly (the fit's error is
a factor of about 3 in either direction); OpenSky saw 7 % of Chinese flights end to end.

**Intended fix:** official national statistics as premium or baseline data per country, or
more regressors (airline presence, distance to other hubs).

### Cargo night departures at Leipzig are under-seen

**What:** OpenSky saw 17,495 departures but only 7,346 arrivals at Leipzig in 2019; the
target still lands within the validation tolerance, but cargo at airports with mostly night
traffic is uncertain.

**Why:** receiver coverage at low altitude at night.

**Intended fix:** none planned; the cargo share per airport comes from airline counts, which
are less affected.
