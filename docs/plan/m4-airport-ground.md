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
| D69 | **Full stands:** an arrival with no suitable free stand taxis to a waiting spot (a remote apron edge or a curated spot) and waits there. The delay is counted in statistics and shown on boards. Supersedes D60 (gates unlimited) | Stand capacity becomes a real constraint |
| D70 | **Ground control:** routes are planned with time reservations on edges and nodes, first come first served. Each aircraft plans at pushback request or at the runway exit, around existing reservations; there is waiting in place on taxiways but no reversing. Along an edge, aircraft keep a following distance. A delayed aircraft releases and re-plans. Fixed order: time, then tail index | Deadlock-free by construction, deterministic (D63), testable as a pure planner |
| D71 | **Runway use:** landing aircraft vacate at the first exit they can make (by wake category and deceleration, rapid exits up to 45°). Departures line up from an OSM holding position at their runway end and join the M3 queue there. A route crossing an active runway asks the runway controller for a gap between movements | Real geometry replaces the fixed landing roll; crossings are a real constraint at FRA and MUC |
| D72 | **Pushback:** the core models pushback as reverse along the stand's lead-in to the taxilane, then a turn and a stop; it needs a reservation. The viewer animates it | Visible and realistic apron movement; it blocks the taxilane like real pushbacks |
| D73 | **Taxi times in schedules:** schedule generation uses each mapped airport's typical routed taxi-out and taxi-in time (median stand → runway in use, without traffic), computed in the core from the network. Unmapped airports keep the M2 draws | Block times stay consistent with what aircraft actually do |
| D74 | **Performance:** the 15k-aircraft benchmark stays under 1 ms per tick with ground control on. Planning cost is per route request, not per tick; ground aircraft advance along their reserved route | Scale target from M2/M3 |
| D75 | **Deadlock escape (PR 3):** an aircraft that cannot get a conflict-free plan for 10 minutes taxis ignoring other aircraft; each case counts as a ground conflict in the statistics, and the acceptance run requires (near) zero | The simulation never stalls; failures stay visible |
| D76 | **Waiting spots (PR 3):** an arrival without a free stand waits at the nearest free dead-end taxiway or taxilane node (not a stand, not on a runway), derived from the layout, until a stand frees | Works at every mapped airport without curation |
| D77 | **Runway crossings (PR 3):** a route crossing an active runway reserves the crossing in a gap of its landing timeline; the crossing aircraft holds short until its window; take-offs wait while a crossing is under way | Crossings are a real constraint (e.g. FRA Terminal 3 traffic across 25L) |
| D78 | **Line-up (PR 3):** departures queue at the runway holding position; on take-off clearance a line-up phase taxies them onto the runway (counted as runway occupancy) before the roll | Matches real hold-short and line-up procedure |
