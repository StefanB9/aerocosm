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

## The world model (plan M5 PR 2)

### Ground control under the 2019 world traffic

**What:** in the ground test's morning bank (04:00–08:00 UTC, seeds 3, 7, 11), with the
world model's traffic at the German airports, 5–8 % of taxis escape a deadlock and 7–8 %
share a stand, and the longest taxi takes up to 110 minutes; M4 kept each under 2 % and 75
minutes with the curated 2025 traffic. The test's limits are relaxed to 10 % and 2 hours
until PR 3. A departure whose stand has no way out for its wingspan taxies straight (D47):
one aircraft in a morning at most.

**Why:** Frankfurt and Munich get about 1,400 and 1,100 movements a day (2019), with more
wide-bodies and hundreds of airlines instead of 23; the curated stand areas know only the
airlines of 2025 (removing them did not help, so the stands and taxiways themselves run
short).

**Intended fix:** plan M5 PR 3: generic stand choice (D103), world-scale ground, and the M4
limits restored.

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
