# Known issues

Open problems found in the simulation, kept until they are fixed. Each entry says what
happens, how often, why, and the intended fix.

## Ground control (plan M4 PR 4)

### Escapes at about 1 % of taxis, mostly at Frankfurt

**What:** an aircraft that gives way in a waiting cycle for a minute, or waits half an hour
outside a runway queue, is cleared regardless (D75, D83); so is a landing that finds every
exit taken (a go-around in reality, not modelled). Each case counts as a ground conflict and
the flight view marks it (`FlightView::ground_conflict`).

**How often:** a 7-day run (seed 0) has 371 escapes in 32,240 taxis (1.15 %; PR 3: 419 in
32,442), 291 of them at Frankfurt, 62 at Munich, 2 at Düsseldorf. A 24 h run has 61 (PR 3:
56). The morning bank in the ground test (seeds 1, 3, 7, 11) has 0.5–0.9 %.

**Why:** at Frankfurt queues from the aprons, the runway exits and the holding positions
meet on few taxiways; aircraft waiting there for each other form cycles that giving way
breaks only by an escape.

**Intended fix:** published or curated one-way taxi flows at Frankfurt (D84), and holding
arrivals at the exit taxiways rather than on the main taxiways.

### Taxi delays at Frankfurt

**What:** with clearances, Frankfurt's average taxi delay is 15 min over 7 days (23 min in a
24 h run; PR 3: under 2 min), its movements 4 % under target in the 24 h run, and its
average flight delay 118 min over 7 days. Elsewhere taxi delays are 2–4 min.

**Why:** clearances hold aircraft until the way to the next stopping point is free; on
Frankfurt's dense aprons stopping points are far apart (junctions within 45 m almost
everywhere), so clearances are long and block others for minutes.

**Intended fix:** finer stopping points on aprons (holding short of a junction rather than
clear of it), flows (D84).

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
