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
| D68 | **Stands:** size class (wingspan limit) from the OSM tags where present, else from the stand's geometry and apron. A curated `assets/ground/stand-areas.json` maps airlines to terminals or aprons at the hubs (e.g. Lufthansa at FRA T1, MUC T2). Otherwise the nearest suitable free stand to the expected exit is used. Turnarounds stay on their stand | Plausible stand use without a full allocation dataset |
| D69 | **Full stands:** an arrival with no suitable free stand taxis to a waiting spot (a remote apron edge or a curated spot) and waits there. The delay is counted in statistics and shown on boards. Supersedes D60 (gates unlimited) | Stand capacity becomes a real constraint |
| D70 | **Ground control:** routes are planned with time reservations on edges and nodes, first come first served. Each aircraft plans at pushback request or at the runway exit, around existing reservations; there is waiting in place on taxiways but no reversing. Along an edge, aircraft keep a following distance. A delayed aircraft releases and re-plans. Fixed order: time, then tail index | Deadlock-free by construction, deterministic (D63), testable as a pure planner |
| D71 | **Runway use:** landing aircraft vacate at the first exit they can make (by wake category and deceleration, rapid exits up to 45°). Departures line up from an OSM holding position at their runway end and join the M3 queue there. A route crossing an active runway asks the runway controller for a gap between movements | Real geometry replaces the fixed landing roll; crossings are a real constraint at FRA and MUC |
| D72 | **Pushback:** the core models pushback as reverse along the stand's lead-in to the taxilane, then a turn and a stop; it needs a reservation. The viewer animates it | Visible and realistic apron movement; it blocks the taxilane like real pushbacks |
| D73 | **Taxi times in schedules:** schedule generation uses each mapped airport's typical routed taxi-out and taxi-in time (median stand → runway in use, without traffic), computed in the core from the network. Unmapped airports keep the M2 draws | Block times stay consistent with what aircraft actually do |
| D74 | **Performance:** the 15k-aircraft benchmark stays under 1 ms per tick with ground control on. Planning cost is per route request, not per tick; ground aircraft advance along their reserved route | Scale target from M2/M3 |

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

## PR 4 — Viewer

- Aprons, taxiway centrelines and stand markers at airport zoom (meshes and gizmo lines); OSM
  attribution in the footer.
- Labels for taxiway names and stand numbers at close zoom (existing label placement).
- Selected aircraft: planned taxi route and its reservations; flight panel shows the stand
  and taxi state ("Pushback from V155", "Taxiing via N, N7 to 25C", "Waiting for stand").
- Pushback animation (reverse movement, heading from the core).
- Browser build checked.

## Definition of done

- At least FRA, MUC, BER, DUS, HAM, CGN and STR pass the quality gate (if data allows).
- `aerocosm run --duration 7d`: no stuck aircraft, no ground conflicts, movement targets and
  capacities still met, taxi times plausible (FRA taxi-out mostly 10–20 min).
- The viewer shows aircraft at real stands, pushing back, taxiing along taxiways, holding
  short and crossing runways; native and web.

## Risks

| Risk | Mitigation |
|------|------------|
| OSM gaps (unconnected stands, missing exits) | Connection snapping, the quality gate, a straight-line fallback, the importer report |
| Overpass mirror flaky or unreachable | Raw files stay outside the repo; a regenerating developer uses any Overpass instance; the query is documented |
| Space-time planning cost at hubs (FRA ≈ 100 aircraft on the ground at peak) | Plan only per request, prune reservations behind the clock, bound search with a horizon |
| Reservations too rigid (late aircraft block others) | Re-plan with released windows; conservative buffers |
| ODbL obligations | Separate asset, licence file, visible attribution |
