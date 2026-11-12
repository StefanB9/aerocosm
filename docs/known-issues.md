# Known issues

Open problems found in the simulation, kept until they are fixed. Each entry says what
happens, how often, why, and the intended fix.

## Ground control (plan M4 PR 4, PR 6, PR 7)

### Escapes at about 2 % of taxis, about half at Frankfurt

**What:** an aircraft that gives way in a waiting cycle for a minute, or waits half an hour
outside a runway queue, is sent another way if there is one (D94) and otherwise cleared
regardless (D75, D83). Each escape counts as a ground conflict and the flight view marks it
(`FlightView::ground_conflict`).

**How often:** the 7-day run (seed 0, PR 7) has 733 escapes in 32,372 taxis (2.3 %), 379 at
Frankfurt and 202 at Munich, and 55 reroutes. 24 h runs, seeds 0–7: 80–125 a day. The
morning bank in the ground test (seeds 3, 7, 11) stays under the test's limit of one in
fifty. Zero is the acceptance goal of D92, not yet met.

**Why:** most cycles form at a junction both aircraft must cross, on approaches too short to
hold clear of it (taxiway stubs of 25–35 m between two intersections), where through
traffic would pass a holder closer than 22.5 m. There is no other way round, so rerouting
rarely helps.

**Intended fix:** hold positions placed in such stubs before the previous junction (a
first attempt that tied the two junctions together made Frankfurt worse), published or
curated one-way taxi flows (D84).

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

### Stand shares overnight

**What:** an arrival that waits 30 minutes for a stand shares one (D69), counted as a stand
share. A 7-day run has 491 (PR 7), nearly all at Munich between 22:00 and 24:00 UTC (the last
arrivals parking overnight), at Memmingen and at Leipzig (few code E stands for B777Fs).

**Why:** the stands mapped in OpenStreetMap run out; remote and maintenance aprons are missing.

**Decision (M4 PR 3):** accepted and reported as a statistic.

### Benchmark above the 1 ms target

**What:** the 15k-aircraft benchmark takes about 3.8 ms per tick with ground control
(PR 7; dev before PR 6: 3.5 ms), above the D74 target of 1 ms.

**Why:** at 23-fold traffic the mapped airports are gridlocked (Frankfurt at about 29,000
movements a day): hundreds of aircraft taxi at each hub, each checked against the others,
and most route searches fail after exploring the whole network.

**Decision (M4 PR 3):** D74 amended; revisit when the world grows beyond Germany.

## Viewer

### Aircraft hard to see on the ground

**What:** zoomed in to an airport's ground, aircraft are hard to tell apart from the aprons
and taxiways in the current colour scheme.

**Intended fix:** a small viewer change after plan M4 PR 7: contrast of aircraft against
aprons and taxiways at ground zoom.
