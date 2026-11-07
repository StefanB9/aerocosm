# Known issues

Open problems found in the simulation, kept until they are fixed. Each entry says what
happens, how often, why, and the intended fix.

## Ground control (plan M4 PR 3)

### Deadlock escapes at about 1 % of taxis

**What:** an aircraft that cannot get a conflict-free plan for 10 minutes (or stands still on
an edge that long) taxies ignoring other aircraft (D75). Each case counts as a ground conflict
and the flight view marks it (`FlightView::ground_conflict`).

**How often:** a 7-day run (seed 0) has 419 escapes in 32,442 taxis (1.3 %); a 24 h run about
50, mostly at Frankfurt and Düsseldorf. The count varies by ±10 between small code changes.

**Why:** the reservations plan each aircraft around the others first come first served, and
the rules for when one overstays (blocking, re-planning, stopping on an edge, giving way
head-on, a held-up departure taking its turn) are local. Where queues meet (the approaches to
holding positions, junctions near busy aprons) they can still wait on each other in a cycle.

**Intended fix:** an explicit conflict-resolution layer instead of more local rules, e.g. a
ground controller issuing progressive clearances (plan M4 D79–D83). In work as M4 PR 4.

### Pushback is not checked against neighbouring traffic

**What:** a pushback reserves where it ends, but the aircraft does not stop while pushing if
an arrival taxies onto a neighbouring lead-in; with seed 1 the two came within 19 m at
Frankfurt (the spacing test asks for 20 m).

**Why:** pushback motion runs outside ground control's tick-by-tick checks.

**Intended fix:** pushbacks cleared and moving under the ground controller (plan M4 D82, PR 4).

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

**Decision (M4 PR 3):** D74 amended; revisit when the world grows beyond Germany.
