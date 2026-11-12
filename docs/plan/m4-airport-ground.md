# Plan: M4 Airport Ground

Status: **approved** (2026-11-06)

## Goal

Aircraft park at real stands and taxi along real taxiways at the German airports. Stands,
taxiways, holding positions and aprons come from OpenStreetMap (D53). A ground controller
plans every taxi route with time reservations: no two aircraft on the same spot, no
deadlocks. Arrivals vacate at real exits. Crossing an active runway needs a clearance from
the M3 runway controller. Departures push back, taxi to a holding position and join the
take-off queue. Full stands keep arrivals waiting. Airports without usable OSM data keep the
M2 straight-line ground (D47). The viewer draws aprons, taxiways, stands, labels, the selected
aircraft's taxi route and pushbacks.

## Data survey (Overpass, 2026-11-06)

| Airport | Stands | Gates | Taxiways | Holding positions | Stands with ref |
|---------|-------:|------:|---------:|------------------:|----------------:|
| EDDF | 444 | 122 | 609 | 361 | 328 |
| EDDM | 236 | 165 | 606 | 256 | 195 |
| EDDB | 233 | 59 | 452 (+159 taxilanes) | 319 | 222 |
| EDDL | 314 | 66 | 181 | 89 | 119 |
| EDDH | 186 | 29 | 160 | 66 | 103 |
| EDDK | 225 | 31 | 96 | 48 | 180 |
| EDDP | 188 | 0 | 160 | 182 | 127 |
| EDDN | 150 | 12 | 35 | 11 | 147 |
| EDFH | 42 | 12 | 36 | 12 | 20 |
| EDDE | 27 | 3 | 34 | 13 | 20 |
| EDLV | 23 | 9 | 74 | 18 | 18 |
| EDLP | 16 | 7 | 74 + 22 | 14 | 13 |
| EDDG | 16 | 11 | 69 | 19 | 9 |
| EDDR | 0 | 0 | 10 | 4 | 0 |

The mirror timed out on the remaining airports, so the importer report fills them in. At
FRA, the taxiways near the runways form one network of 6,778 of 7,839 nodes. The remaining
small components are cut off by the query box or are disconnected service areas.

## Decisions log (continues the M3 plan)

| # | Decision | Rationale |
|---|----------|-----------|
| D64 | **Source:** one Overpass JSON file per airport, fetched with a documented query (aerodrome area: aeroway taxiway, taxilane, runway, parking_position, holding_position, apron, gate, terminal). Pinned by download date and SHA-256 in `docs/data-sources.md`. Read with serde_json; no new dependency | Small files (FRA ≈ 0.9 MB), reproducible, same workflow as the other sources. overpass-api.de and Geofabrik are unreachable from cloud sessions; a public mirror works |
| D65 | **Asset:** generated `assets/ground/germany.ground.json` under ODbL with `LICENSE-ODbL` alongside. Attribution "© OpenStreetMap contributors" goes in the viewer footer and in data-sources. The world file stays public domain | Keeps the ODbL obligations to one derived database (D53) |
| D66 | **Coverage:** every German target airport is imported. A quality gate requires at least one stand per 10 daily movements, a connected network linking stands to every runway end in use with the prevailing wind (OSM one-way tags often reflect one flow direction), and holding positions at the departure runways. Airports that fail keep straight-line ground, and the importer reports why | Uses all usable data and degrades gracefully (EDDR has no stands) |
| D67 | **Network:** ways are split at shared nodes into edges with length and polyline; edges are tagged as taxiway, taxilane or runway. Stands connect to the nearest taxilane or taxiway when not joined. Runway exits are taxiway nodes on a runway's centreline, with exit angle and distance from each threshold. Positions are rounded to ~0.1 m | Graph search and reservations need explicit topology |
| D68 | **Stands:** size class (wingspan limit) from the OSM tags where present, else from the stand's geometry and apron. Where the spacing of the mapped stands misjudges an apron, a curated `assets/ground/stand-sizes.json` bounds the ICAO code by stand-name prefix (Köln/Bonn). A curated `assets/ground/stand-areas.json` maps airlines to terminals or aprons at the hubs (e.g. Lufthansa at FRA T1, MUC T2). Otherwise the nearest suitable free stand to the expected exit is used. Turnarounds stay on their stand | Plausible stand use without a full allocation dataset |
| D69 | **Full stands:** an arrival with no suitable free stand taxis to a waiting spot (a remote apron edge or a curated spot) and waits there. The delay is counted in statistics and shown on boards. Supersedes D60 (gates unlimited) An arrival that has waited 30 minutes takes a stand shared with another aircraft (counted as a stand share); one too large for every stand takes the largest. Stands are chosen among those a route reaches from where the aircraft is | Stand capacity becomes a real constraint |
| D70 | **Ground control (PR 3; superseded by D79 in PR 4):** routes are planned with time reservations on edges and nodes, first come first served. Each aircraft plans at pushback request or at the runway exit, around existing reservations; there is waiting in place on taxiways but no reversing. Along an edge, aircraft keep a following distance. A delayed aircraft releases and re-plans. Nodes closer than 45 m (an aircraft length) are held together, so aircraft waiting at neighbouring nodes do not overlap. Plans trust a waiting aircraft's estimated stay, but no aircraft enters a node until whoever waits there has left. Plans may arrive up to 30 minutes past the time a winding route (3× the straight line) takes. Held-up aircraft retry after a quarter of the time held (15–60 s; 30–120 s at the gate) and skip the search while someone waits at the node ahead. An aircraft on an edge stops while someone else is at the node ahead and goes on, its plan shifted, once they have left. No aircraft enters a node from which one waiting at the next node wants to come towards it (head-on). A departure held up two minutes takes its turn at the holding position before those booked more than two minutes later. Fixed order: time, then tail index | Deadlock-free by construction, deterministic (D63), testable as a pure planner |
| D71 | **Runway use:** landing aircraft vacate at the first exit they can make (by wake category and deceleration, rapid exits up to 45°). Departures line up from an OSM holding position at their runway end and join the M3 queue there. A route crossing an active runway asks the runway controller for a gap between movements | Real geometry replaces the fixed landing roll; crossings are a real constraint at FRA and MUC |
| D72 | **Pushback:** the core models pushback as reverse along the stand's lead-in to the taxilane, then a turn and a stop; it needs a reservation. The viewer animates it | Visible and realistic apron movement; it blocks the taxilane like real pushbacks |
| D73 | **Taxi times in schedules:** schedule generation uses each mapped airport's typical routed taxi-out and taxi-in time (median stand → runway in use, without traffic), computed in the core from the network. Unmapped airports keep the M2 draws | Block times stay consistent with what aircraft actually do |
| D74 | **Performance:** the 15k-aircraft benchmark stays under 1 ms per tick with ground control on. Planning cost is per route request, not per tick; ground aircraft advance along their reserved route | Scale target from M2/M3. Amended with PR 3: about 1.5 ms per tick with 15,271 airborne (dev without ground control: 0.90 ms); at 23-fold traffic the mapped airports are gridlocked and most route searches fail. Revisit when the world grows beyond Germany (`docs/known-issues.md`) |
| D75 | **Deadlock escape (PR 3):** an aircraft that cannot get a conflict-free plan for 10 minutes taxis ignoring other aircraft; each case counts as a ground conflict in the statistics and shows on the flight view; the 10 minutes run from when it last passed a node, across re-plans (or from when it stopped on an edge); an escaped route is not checked against others, and the acceptance run requires (near) zero | The simulation never stalls; failures stay visible |
| D76 | **Waiting spots (PR 3):** an arrival without a free stand waits at the free spot with the shortest taxi from its exit, a dead-end taxiway or taxilane node (not a stand, not on a runway), derived from the layout and at least 45 m from every other edge so traffic passes, until a stand frees | Works at every mapped airport without curation |
| D77 | **Runway crossings (PR 3):** a route crossing an active runway reserves the crossing in a gap of its landing timeline; the crossing aircraft holds short until its window; take-offs wait while a crossing is under way, and give way to an aircraft that has held short for a minute while it keeps asking (a request lapses 20 s after its last renewal). Leaving the runway an aircraft is on is no crossing. Taxi plans never run along an active runway unless nothing else leads on (backtracking at small airports): such edges cost 100 times their duration | Crossings are a real constraint (e.g. FRA Terminal 3 traffic across 25L) |
| D78 | **Line-up (PR 3):** departures queue at the runway holding position; on take-off clearance a line-up phase taxies them onto the runway (counted as runway occupancy) before the roll | Matches real hold-short and line-up procedure |
| D79 | **Ground controller (PR 4):** ground control works like a real ground controller issuing progressive taxi clearances. An aircraft is cleared along its route only up to its next clearance limit, and only when the way there is free or used in the same direction ("follow"); otherwise it holds at its limit. Replaces the timed node and edge reservations, the timed planner and the PR 3 recovery rules (stopping on edges, head-on checks, going first, re-plan backoff). Route choice (A*), departure management at the gate, stand choice, waiting spots and typical taxi times stay | PR 3's first-come-first-served reservations deadlocked on about 1 % of taxis; clearances mirror what controllers do and keep aircraft apart by construction |
| D80 | **Clearance points (PR 4, as built):** derived from each layout. A clearance point is a node, or a point every ≤100 m along long edges so aircraft can follow each other. One aircraft holds a point at a time, and every point also conflicts with the points within 45 m (an aircraft length; not grouped transitively, which made clusters of 30–67 junctions). An aircraft may stop only at a point on an edge or a node with at most two edges and no junction within 45 m; a clearance runs on to the next such point. While an aircraft is on an edge, or intends to take it within 300 m beyond its clearance, nobody is cleared onto it the other way. *As planned:* junction areas and stretches; the points model is the same idea at finer grain and needs no grouping | Works at all mapped airports without curation; direction rules out head-on meetings |
| D81 | **Priority (PR 4):** contested points go to aircraft on or vacating a runway, then crossing aircraft, then departures, then arrivals; pushbacks are cleared at off-block, after the tick's requests. Ties by how long they have waited, then tail index. A refused aircraft's points and the edges it intends to take are *wanted*: nobody less urgent is cleared onto them (or onto those edges the other way) unless the refused aircraft waits on them, so merging aircraft are sequenced in rather than starved | The runway is cleared first, as controllers do; deterministic (D63) |
| D82 | **Runways and pushback (PR 4):** crossings and line-up stay with the runway controller (D77, D78): an aircraft holds short until it may cross, then is cleared across in one go. A pushback needs a clearance for its lead-in and on to where it may stop, and is refused (asked again after 30 s) otherwise. A landing chooses its exit at touchdown: the planned one or a later one whose way off the runway is free, cleared at once; a vacating aircraft stops 90 m clear of the runway at the first point that is no junction, holding short of the taxiway there if need be. With every exit taken, the tower would send it around; go-arounds are not modelled, so it taxies regardless and counts as a ground conflict | One set of rules for everything that moves on the ground; arrivals never wait on the runway |
| D83 | **Gridlock (PR 4, as built):** the controller follows who waits for whom; in a cycle the least urgent member gives way and, after 60 s, is cleared regardless (counted as a ground conflict, D75). Re-routing round the cycle was tried and is mostly infeasible on real layouts. Routes avoid edges in use the other way (10× their length) and, as published taxi flows would, edges against the direction traffic has recently taken them (3×, learnt per edge). The 30-minute last resort (D75) remains, except in a runway queue | Mirrors controllers noticing a jam; learnt flows remove most head-on cycles before they form |
| D84 | **No published taxi flows yet:** standard one-way flows per runway direction are not curated in PR 4; the learnt edge directions of D83 stand in for them. Published flows are a later improvement (curated, or Navigraph in the premium tier) | Needs per-airport sources |
| D85 | **Taxi state (PR 5):** `FlightView` gains a `TaxiState` while an aircraft moves on the ground of a mapped airport: *pushback from* a stand; *taxiing via* the next (up to three) taxiway names *to* a holding point (runway), a stand or a waiting spot; *holding short of* a runway to cross it; *waiting for* another flight (its callsign) or behind the runway queue; *waiting at a spot for a stand*; *cleared regardless* (an escape, D75). Fixed-size, so `FlightView` stays `Copy` | The flight panel says what ground control is doing with the aircraft; the waits name who is in the way |
| D86 | **Taxi route query (PR 5):** `Simulation::taxi_route(tail)` returns the selected aircraft's taxi polyline, how far along it is, how far it is cleared and where the route ends; computed on demand for one aircraft, not per tick | The viewer draws the route and the cleared part without the core keeping anything for it |
| D87 | **Ground drawing (PR 5):** below an airport zoom threshold, apron polygons as meshes tessellated once at start-up; taxiway and taxilane centrelines, stand lead-ins and stand markers as gizmo lines redrawn each frame, only for mapped airports in view. A selected taxiing aircraft shows its route ahead, the cleared part brighter and its clearance limit marked. Taxiway names (one label per named stretch) and stand names at closer zoom go through the existing label placement, after airports | Gizmos keep the code small; culling by airport keeps the per-frame cost to the airports on screen |
| D88 | **Clearance waits (PR 5):** the statistics add the time taxiing aircraft wait for a clearance, per airport (total and average per taxi), shown in the airport panel's flow status next to the taxi delay and in the headless run summary | Shows where ground control holds traffic, e.g. the Frankfurt regression |
| D89 | **Follow clearances (PR 6):** an aircraft whose way is held by a leader going the same way along the same route is cleared to follow it, as controllers say "follow the A320": it may take the points the leader leaves as soon as they are free and taxies up to a following distance (60 m) behind the leader, stopping when it stops. Its clearance limit is the leader's, so convoys move at taxi speed instead of stop-and-go. Crossing or merging traffic and head-on rules are unchanged | A 24 h run shows FRA's waits (25,000 min, 95 % arrivals taxiing in) are convoys on taxiway P behind moving leaders, chains of median 9 aircraft; following removes the stop-and-go |
| D90 | **Go-arounds (PR 6):** on short final the tower checks the landing can vacate: the runway is clear and an exit is free or will be by touchdown. If not, the aircraft goes around: it climbs out on the runway heading to the missed-approach altitude, flies back to the final fix and is re-sequenced for the next free landing slot. Counted per airport as a statistic, not a ground conflict, and shown in the flight panel | Replaces counting a landing with every exit taken as a conflict (PR 4); what towers do |
| D91 | **Departure management per runway (PR 6):** the expected take-off timeline and learnt interval are kept per departure runway, shared by all its holding positions; a pushback waits at the gate while the runway's queue would exceed two minutes | Berlin queued up to an hour because each holding position kept its own estimate |
| D92 | **Acceptance (PR 6):** the 7-day run (seed 0) has zero ground conflicts (escapes stay as a counted safety net; go-arounds are not conflicts), FRA's average taxi delay is back near PR 3's (~2 min), movement targets and capacities hold, and the benchmark is re-measured | The M4 definition of done |


## PR 1 — OSM import and ground network asset

- `aerocosm-data`: `osm/` module (Overpass JSON subset, element conversion) and `import
  --osm <dir>` writing `assets/ground/germany.ground.json`. Includes graph building (D67),
  stand connection and exit detection, the quality gate and a report per airport.
- `aerocosm-basemap`-style shared file types: a ground definition with versioned format,
  aprons as polygons for the viewer.
- Core: `GroundNetwork` per airport (edges, nodes, stands, holding positions, exits) loaded in
  `Scenario`; guard test on the shipped asset.
- `docs/data-sources.md`, ODbL licence file, `queries` command for the downloads.
- *Changed during PR 1:* the curated airline areas move to PR 2, where stand assignment
  uses them. OSM apron and terminal tags proved too inconsistent (e.g. "222;223" at FRA), so the
  areas are defined by stand-name prefixes, which need sourcing per hub.
- **Tests:** conversion units, proptests for graph building (lengths, split ways stay
  connected, no duplicate edges), quality gate cases, determinism (byte-identical output).

## PR 2 — Routing, stands and pushback

- Shortest paths on the network (A* by length, respecting one-way tags and size limits).
- Curated `assets/ground/stand-areas.json` (airlines → stand-name prefixes per hub, with
  sources) and stand assignment (D68) at landing booking or at warm start; turnarounds stay
  put. Variants of one stand (`05`, `05A`, `05B`) are mutually exclusive.
- Motion follows a ground route (polyline walked by distance) for taxi-out and taxi-in;
  pushback phase (D72); the landing roll ends at the chosen exit (D71).
- Typical taxi times per airport and their use in schedule generation (D73).
- No conflicts yet: aircraft may overlap on the ground.
- **Tests:** route continuity and validity, stand suitability, exits reachable by type,
  schedule targets still within tolerance, determinism.
- *Implemented in PR 2:*
  - The aircraft catalog gains wingspans (format 2).
  - Stands are taken when a tail's first leg boards (or at the start) and on touchdown,
    nearest the exit. When nothing suitable is free an arrival shares a stand until PR 3's
    waiting spots.
  - Routes cost runways ×4 and other stands' lead-ins ×20.
  - Routed taxiing averages 5 m/s (about 10 kt). Pushback is 1 m/s along the lead-in plus
    60 s of engine start.
  - Rapid exits (≤ 45°) are taken at 15 m/s, others at 8 m/s, with touchdown assumed 300 m
    past the threshold.
  - Departures still queue at the runway entry; holding short before line-up comes with
    ground control in PR 3.

## PR 3 — Ground control

- Reservation table per airport (D70): node and edge time windows; a pure planner
  (space-time search) taking the request and the reservations and returning a timed route;
  following distance; waiting in place.
- Runway crossings and line-up through the M3 runway controller (D71); waiting spots for
  full stands (D69); re-planning when a pushback, crossing or take-off slips.
- Statistics: taxi delay and stand waits per airport.
- **Tests:** property tests over random requests (no two aircraft on a node or edge closer
  than the spacing, no deadlock, every request completes), FCFS order, crossing gaps never
  violate runway separation, 7-day run without stuck aircraft; benchmark < 1 ms.

## PR 4 — Ground controller (D79–D84)

- Core: clearance points derived from each layout (D80, `ground/clearance.rs`): holders,
  neighbours within 45 m (points on edges included), edge directions and intents, stopping
  points, learnt edge flows.
- The controller (`ops/fleet/ground/grant.rs`): every tick, aircraft near their limit ask to
  be cleared on, most urgent first (D81); a clearance runs to the next stopping point, across
  a runway in one go; wanted points sequence merging aircraft; waiting cycles give way (D83).
- Runway crossings and line-up through the runway controller (D82); pushback cleared for its
  lead-in and on to a stopping point; exits chosen at touchdown, the vacate cleared at once.
- Removed: timed node and edge reservations, the timed planner (`ground/reserve/`), the PR 3
  recovery rules (`fleet/ground/stop.rs`). Kept: A* route choice (with flow and traffic
  costs), departure management at the gate, stand choice, waiting spots, typical taxi times.
- **Tests:** clearance points on grid and parallel-lane fixtures; spacing helpers against a
  full search at FRA; integration on seeds 3, 7 and 11 (and 1 checked) with no two taxiing
  aircraft within 20 m, pushbacks included, escapes at most 1 % of taxis, nobody taxiing over
  an hour; determinism; benchmark measured.
- **Moved to PR 5:** the taxi state in the flight view ("cleared to holding point 25C",
  "holding short", "following DLH123") and a clearance-wait statistic, with the viewer that
  shows them.

## PR 5 — Viewer (D85–D88)

- Core: `TaxiState` in `FlightView` (D85), kept by ground control as it clears, holds and
  escapes; `Simulation::taxi_route` (D86); clearance-wait statistics (D88) and the headless
  run summary.
- Viewer, pure and tested: the ground level of detail (zoom thresholds), culling of airports in
  view, stand marker and lead-in geometry, taxiway name label candidates (one per named
  stretch), route split at the cleared distance, taxi state wording.
- Viewer systems: apron meshes (start-up), centrelines, lead-ins and stand markers (gizmos),
  the selected aircraft's route and limit, taxiway and stand labels, the flight panel's taxi
  line, the airport panel's clearance-wait line.
- Pushback (already reversing in the core): checked in the viewer, and the turn from pushback
  heading to taxi heading blended rather than snapped.
- OSM attribution is already in the footer (PR 1); kept.
- **Tests:** core unit tests for each taxi state and the route query; a ground-test check
  that every taxiing aircraft at a mapped airport has a taxi state; viewer unit tests for the
  pure functions; screenshots at FRA (aprons, taxiways, stands, labels, a selected taxiing
  aircraft with its cleared route) natively; the browser build checked.
- **As built:** the ground shows below scale 30 (map units per pixel), taxiway names below 6
  and stand names below 1.8; taxiway and stand lines are retained gizmos per airport (like the
  rivers), switched by zoom and view. Map labels now keep clear of the panels over the map.
  The pushback turn is in the core: the heading eases from nose-to-stand to the taxi
  direction over the push, so the viewer needs no special blend. Speed control is noted only
  while airborne.

## PR 6 — Close-out (D89–D92)

- Core, ground: follow clearances (D89) in `grant.rs` (who leads whom) and `drive.rs`
  (following distance behind the leader); the taxi state says "following DLH123".
- Core, runway: go-arounds (D90): the short-final check in the runway controller, a
  missed-approach path and phase, re-sequencing, the statistic; flight panel note.
- Core, departures: departure management per runway (D91).
- Verification: the 7-day and 24 h runs, the benchmark, `docs/known-issues.md` updated
  (FRA, go-arounds and Berlin entries closed or re-measured).
- **Tests:** following on a grid fixture (a follower keeps its distance and never overtakes;
  crossing traffic still waits); a go-around when the exit is blocked, re-sequenced and landing
  later; per-runway departure management with two holding positions; the ground integration
  test tightened (no escapes in the morning bank on seeds 1, 3, 7, 11); determinism.

## Definition of done

- At least FRA, MUC, BER, DUS, HAM, CGN and STR pass the quality gate (if data allows).
- `aerocosm run --duration 7d`: no stuck aircraft, no ground conflicts (PR 4), movement targets and
  capacities still met, taxi times plausible (FRA taxi-out mostly 10–20 min).
- The viewer shows aircraft at real stands, pushing back, taxiing along taxiways, holding
  short and crossing runways; native and web.

## Risks

| Risk | Mitigation |
|------|------------|
| OSM gaps (unconnected stands, missing exits) | Connection snapping, the quality gate, a straight-line fallback, the importer report |
| Overpass mirror flaky or unreachable | Raw files stay outside the repo; a regenerating developer uses any Overpass instance; the query is documented |
| Space-time planning cost at hubs (FRA ≈ 100 aircraft on the ground at peak) | Plan only per request, prune reservations behind the clock, bound search with a horizon |
| Reservations too rigid (late aircraft block others) | Re-plan with released windows; conservative buffers. In PR 3 local recovery rules still deadlocked about 1 % of taxis, hence the controller of PR 4 |
| Gridlock in taxiway loops under clearances | Cycle detection with the least urgent giving way, learnt flows, traffic-aware routes, wanted points (D81, D83); the escape safety net is counted. As built: 0.5–0.9 % of taxis still escape in the morning bank (docs/known-issues.md) |
| ODbL obligations | Separate asset, licence file, visible attribution |
