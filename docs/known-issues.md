# Known issues

Open problems found in the simulation, kept until they are fixed. Each entry says what
happens, how often, why, and the intended fix.

## Ground control (plan M4 PR 4, PR 6)

### Escapes at about 2 % of taxis, about half at Frankfurt

**What:** an aircraft that gives way in a waiting cycle for a minute, or waits half an hour
outside a runway queue, is sent another way if there is one (D94) and otherwise cleared
regardless (D75, D83); so is a landing that finds every exit taken (a go-around in reality,
not modelled). Each escape counts as a ground conflict and the flight view marks it
(`FlightView::ground_conflict`).

**How often:** 24 h runs, seeds 0–7: 72–114 escapes a day in about 4,650 taxis (about 2 %),
37–56 of them at Frankfurt and 21–45 at Munich; 5–37 reroutes. The morning bank in the
ground test (seeds 3, 7, 11) stays under the test's limit of one in fifty.

**Why:** most cycles form at a junction both aircraft must cross, on approaches too short to
hold clear of it (taxiway stubs of 25–35 m between two intersections), where through
traffic would pass a holder closer than 22.5 m. There is no other way round, so rerouting
rarely helps.

**Intended fix:** hold positions placed in such stubs before the previous junction (a
first attempt that tied the two junctions together made Frankfurt worse), published or
curated one-way taxi flows (D84), and go-arounds for blocked exits (D90). Zero escapes is
the acceptance goal (D92).

### Taxi delays at Frankfurt

**What:** Frankfurt's average taxi delay is 2.7–4.2 min over 24 h (PR 4: 23 min; target
about 2 min), its movements 0–4 % under target. Elsewhere taxi delays are about 2 min.

**Why:** clearances wait for gaps in the through traffic on the main taxiways, and waiting
cycles at the apron exits hold aircraft for a minute before one gives way.

**Intended fix:** flows (D84), departure management per runway (D91).

### Landings roll on to late exits

**What:** landings often seem to take the last exit. A 24 h run (seed 0) has 2,290 landings:
72 % vacate at their planned exit, 10 % roll on to a later one because the planned exit's
way off is not approved, and 17 % find no exit clear and wait on the runway. The planned
exit is the runway's last for 10 %.

**Why:** the approval of the way off the runway (`clear_way`) asks for the whole route to be
free of opposing locks and the first 200 m clear; the planned roll (touchdown at 300 m,
braking at 1.8 m/s² to 15 or 8 m/s) is not yet checked against real exit use.

**Intended fix:** plan M4 PR 7 (exits, with go-arounds D90).

### No go-arounds

**What:** a landing that cannot vacate because every exit is taken taxies regardless and
counts as a ground conflict, instead of going around.

**Intended fix:** go-arounds in the runway controller, re-sequencing the arrival.

### Long runway queue at Berlin

**What:** departures at Berlin sometimes queue an hour at the holding position.

**Why:** departure management estimates the queue per holding position, not per runway.

**Intended fix:** departure management per runway.

### Stand shares overnight

**What:** an arrival that waits 30 minutes for a stand shares one (D69), counted as a stand
share. A 7-day run has 575, nearly all at Munich between 22:00 and 24:00 UTC (the last
arrivals parking overnight), at Memmingen and at Leipzig (few code E stands for B777Fs).

**Why:** the stands mapped in OpenStreetMap run out; remote and maintenance aprons are missing.

**Decision (M4 PR 3):** accepted and reported as a statistic.

### Benchmark above the 1 ms target

**What:** the 15k-aircraft benchmark takes about 1.5 ms per tick with ground control (dev
without it: 0.9 ms), above the D74 target of 1 ms.

**Why:** at 23-fold traffic the mapped airports are gridlocked (Frankfurt at about 29,000
movements a day), so most route searches fail after exploring the whole network.

**Decision (M4 PR 3):** D74 amended; revisit when the world grows beyond Germany. Not yet
re-measured with the PR 4 controller.

## Viewer

### Aircraft hard to see on the ground

**What:** zoomed in to an airport's ground, aircraft are hard to tell apart from the aprons
and taxiways in the current colour scheme.

**Intended fix:** a small viewer change after plan M4 PR 7: contrast of aircraft against
aprons and taxiways at ground zoom.
