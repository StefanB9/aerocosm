# Plan: M5 The Globe

Status: **approved** (2026-11-13)

## Goal

The simulation covers the world. Every airport with scheduled service is in the network,
real airlines fly it at roughly the real volume (about 100,000 flights a day, about 12,000
airborne at once), and every scheduled airport whose OpenStreetMap layout passes the quality
gate has full ground control. The viewer zooms from the whole globe down to a single stand.

One rule governs the milestone (D95): **general behaviour, as little local interference as
possible.** Every value for an airport, airline or runway comes from global data through
general rules. The hand-curated lists that made Germany work (edge airports, runway modes,
capacities, curfews, hub banks, stand areas and sizes, the airline catalog) stop being
inputs. The German statistics stay, as validation that the general model is realistic.

## What is Germany-specific today

| Area | Today | In M5 |
|------|-------|-------|
| Airports | OurAirports, country DE plus 23 curated edge airports | All aerodromes; the network is every airport with scheduled service (D96) |
| Traffic volume | Movement targets for 24 airports (ADV), DFS overflights | Observed movements per airport from OpenSky 2019, gravity fallback (D97, D98) |
| Airlines | 23 curated airlines (bases, network, fleet, colour) | Real airlines derived by rules from observed flights (D99) |
| Runway use | Curated modes, capacities, curfews, banks | Climatological wind and rules from runway geometry (D102) |
| Time zones | Curated per airport | Open dataset per airport (D101) |
| Declination | One value, 4° E | Learnt per airport from surveyed runways (D101) |
| Ground | 22 OSM layouts, curated stand areas and sizes | OSM layouts for every scheduled airport, generic stand choice, compact binary (D103, D104) |
| Viewer | Projection centred on Germany, all data embedded | World basemap, floating origin, ground streamed per airport (D106) |

## Decisions log

Continues from M4 (D53–D94).

| # | Decision | Rationale |
|---|----------|-----------|
| D95 | **General behaviour over local curation.** Per-airport and per-airline values come from global datasets through general rules, never from hand-written lists. Hand-curated data remains only where it is general knowledge rather than local (the aircraft type catalog, D100). Premium data (Navigraph) is not part of M5 | User rule for M5; a world cannot be curated airport by airport |
| D96 | **Scope.** The map shows all aerodromes with runways worldwide (by zoom level, as D9). The network is every airport OurAirports marks with scheduled service. Edge airports and overflights retire: the world is closed | Germany needed edges and overflights only because it was a cut-out |
| D97 | **Traffic: model calibrated to observation.** The gravity model and proportional fitting (D28) stay. Each airport's target is its observed movements from the OpenSky flight lists (origin and destination counts), scaled for receiver coverage. Where coverage is too thin to trust, the target comes from the model: catchment population (Natural Earth places) weighted by country income (Natural Earth admin-0 GDP), scaled so that the world total matches. Routes still emerge from gravity, not from observed pairs | Observed volume per airport is data, not curation; routes as general behaviour keep the model generative |
| D98 | **Reference year 2019**, the last full pre-Covid year of OpenSky's CC BY flight lists (Zenodo, monthly CSV, 2019 – January 2022) | The CC BY release is frozen; 2020–21 traffic is distorted by Covid |
| D99 | **Airlines: real, derived by rules.** Airlines are the ICAO callsign prefixes seen in the 2019 flight lists above a minimum count; name, callsign and country from OpenFlights (ODbL). Bases are the airports with the largest shares of an airline's departures; network type (hub, point-to-point, regional) by rule from how concentrated those are; fleet mix by rule from airline size, route lengths and the aircraft catalog (or from observed type codes, if the CC BY lists keep them); colours generated from the code. Cargo airlines by rule from the same evidence | Keeps D8 (real airlines) without a curated catalog |
| D100 | The aircraft type catalog stays hand-curated and grows to the world's common types, freighters included | Type performance is general data, not local interference |
| D101 | **Time zones and declination.** IANA zone per airport from an open dataset (`airportsdata`, MIT; its data licence is checked in PR 1, else a polygon lookup in timezone-boundary-builder, ODbL). For synthesised runway headings (D18), the relation between designator and true heading is learnt from runways with surveyed headings: the mean of true heading minus ten times the designator over the 50 nearest surveyed runway ends (*as built*: about 2° mean error against published declinations, under 5° at worst; it includes the historical declination a designator was painted with, which a magnetic model would not) | Replaces two Germany-wide constants; general, from the data itself, no extra dataset (the World Magnetic Model was the first choice) |
| D102 | **Runway use by rule.** Prevailing wind per airport from the NASA POWER climatology (CC BY 4.0), fetched by the importer; the flow uses the runway ends facing it (until the weather milestone). Runway geometry decides the mode: parallel runways far enough apart run independently or segregated (arrivals on one, departures on the other), single and crossing runways mixed. Capacity per hour and per window by rule from the mode. Hub banks by rule for hub airlines. No curfews | Replaces D54/D55 curation; wind climatology is general data |
| D103 | **Ground everywhere.** OSM layouts for every scheduled airport, imported in regional batches; the quality gate (D66) decides which are used, the others taxi straight-line (D47). Stand choice is generic: by stand size (OSM tags, else geometry), then distance, no curated airline areas or size prefixes | User choice; generic stand choice follows D95 |
| D104 | **Compact binary assets, committed.** World, traffic and ground assets move to a versioned binary format (serde with postcard, zstd-compressed, pure-Rust decoder for wasm), ground split by region; estimated 20–40 MB in total. Guard tests load them as now. New dependencies are justified in PR 1 | Today's JSON would be about 250 MB for the world's ground |
| D105 | **German statistics as validation.** ADV movements, the FRA/MUC capacities and the M4 acceptance numbers are no longer inputs; tests check that the world model reproduces German movements within a tolerance (set in PR 2) and keeps under the published capacities | Proof that general behaviour stays realistic |
| D106 | **Viewer.** World basemap from Natural Earth with level of detail; the projection's origin follows the camera (floating origin) so that f32 precision holds anywhere; airports and the basemap are embedded, ground layouts load per airport when zoomed in (from disk natively, over HTTP on the web) | Web Mercator around a fixed origin (D25) breaks far from Germany; embedding the world's ground would bloat the web build |
| D107 | **Scale.** About 100,000 flights a day, about 12,000 airborne, ground control at every gated airport. The benchmark measures the real world instead of 23-fold Germany; a tick must stay under 8 ms (60× real time leaves about 16 ms of wall time per tick) | Replaces D74's synthetic benchmark |
| D108 | **Pipeline.** The importer gains reproducible steps for global airports (with declination, time zones and climatological wind), OpenSky aggregation (raw monthly files stay outside the repository) and regional OSM ground; each is documented in `docs/data-sources.md` with its licence and attribution (ODbL share-alike for ground and airline names, CC BY for OpenSky and NASA POWER) | D53 and the data-tier rules, at world scale |

## PRs

### PR 1 — Global inputs (data)

- Importer: worldwide airports and runways (scheduled-service flag, declination by WMM,
  time zones, NASA POWER prevailing wind per scheduled airport); world basemap.
- OpenSky aggregation: movements per airport and departures per airline and airport for
  2019, coverage estimate per region; airline metadata from OpenFlights.
- Binary asset format (D104) with versioning and guard tests; Germany assets regenerated
  as a subset for fast tests.
- **Tests:** importer units on fixtures (property tests for the aggregation), guard tests on
  the shipped binaries, licence and attribution checks in `docs/data-sources.md`.

### PR 2 — The world model (core)

- Targets from observation with the catchment fallback (D97); airlines by rule (D99);
  runway modes, capacities and banks by rule (D102); the curated Germany inputs removed;
  edges and overflights retired.
- **Tests:** rules on fixtures; the schedule on the shipped world (volume, every scheduled
  airport served, determinism); German validation (D105).

#### PR 2 design (agreed 2026-11-13)

Findings on the shipped data: 1,351 of 3,846 scheduled airports are observed (China 12 of
253); country coverage scales FRA +10 %, HAM +24 %, DUS −7 % against 2019; the catalog's 23
types cover 70 % of typed flights; gravity over all scheduled pairs is 7.4 M routes.

| # | Decision |
|---|----------|
| D109 | **Targets.** Airline-callsign flights only. An airport in a country with coverage ≥ 0.3 and ≥ 365 observed airline flights has the target of its observed departures scaled by the flights its own receivers saw over those seen whole ((complete + partial) ÷ complete, counted per airport; *refined in PR 2*: the country's coverage scaled Hamburg 27 % too high). Every other scheduled airport gets the model: a power law in catchment and country GDP per capita (Natural Earth admin-0, added to the world asset by the importer), longest runway, runway count and airport size, fitted on the observed airports. The world total is then scaled to 2019 (ICAO, about 105,000 departures a day). Traffic observed at an airport now closed counts, when the flight lists are aggregated, at the nearest open scheduled airport within 50 km (Tegel to BER) |
| D110 | **Airlines.** An observed prefix is dropped when most of its typed flights are on types outside the airliner catalog (business jets, pistons: NetJets). It is cargo when at least 40 % of its flights are on freighter-only types (A300-600F, 777F, MD-11F; *refined in PR 2*: 757s and 767s fly as both, so they count as passenger types that cargo airlines may fly) or its OpenFlights name says Cargo, Freight, Parcel or Logistics (*refined*: "Express" also names passenger airlines). Its stations are the airports where it departs, weighted by its share of the airport's departures; an unobserved airport inherits the airlines of the nearest observed airports in its country. Hubs: airports with ≥ 8 % of the airline's departures where it flies ≥ 25 % of the airport's; an airline with a hub is a hub carrier. Fleet: observed types mapped to the catalog (exact, else the catalog type of the same family and size). Colour from the code. Cargo share per airport follows from the cargo airlines' departures |
| D111 | **Catalog** grows by about 20 common types (737-700/900, E175, CRJ-200/700, 777-200, A330-200, 747-400, 787-8, E145, E170, 717, MD-11, A318, …) to cover about 95 % of typed flights (D100) |
| D112 | **Demand at world scale.** Gravity pairs pruned to each airport's 150 strongest partners (union), proportional fitting on adjacency lists |
| D113 | **Runways by rule.** Ends within 90° of the prevailing wind are in use; the flow runs along the parallel group that handles most movements within 60° of the wind (*refined in PR 2*: Schiphol's three parallels lie 50° off its prevailing wind; the best-aligned runway alone gave 40 an hour). Parallels ≥ 1,035 m apart run independently; closer ones pair up as one lane, segregated (arrivals on one, departures on the other); of crossing runways the best aligned runs mixed. Capacity per hour: 40 for a lane of one runway, 60 for a lane of two, lanes added up (*refined in PR 2*: Frankfurt's 518 m pair works segregated; it gives FRA 100, MUC 80, ATL 160), 10-minute windows at a sixth. An airport is coordinated (D54) where its generated peak hour exceeds 80 % of its capacity. Banks for hub carriers at their hubs, evenly between 06:00 and 22:00 local, their number from the carrier's daily departures there. Weekday factors stay one global pattern |
| D114 | **Validation (D105)** against ADV 2019: each German airport within ±25 %, Germany's total within ±10 %, FRA and MUC under their published capacities |
| D115 | **Tests** run on the full world (no subset). *As built:* nextest runs each test in its own process, so nothing is shared; instead the dev profile optimises `aerocosm-core` and `ruzstd` (opt-level 2): deriving the model takes a fraction of a second, generating the week's schedule a few seconds, and the core suite about 20 minutes |

### PR 3 — Ground at world scale (data + core)

- Regional Overpass imports, resumable; binary ground per region; generic stand choice
  (D103); quality-gate report for the world.
- Memory and time per tick with ground control everywhere measured; layouts loaded once.
- **Tests:** stand choice rules; guard tests per region; the ground integration test on a
  sample of airports from every continent.

#### PR 3 design (agreed 2026-11-13)

Findings: the main Overpass instance and Geofabrik are unreachable from the build container;
the `maps.mail.ru` mirror answers (Frankfurt: 22 s, 1.4 MB), but not reliably; the planet file
streams at about 18 MB/s. The 22 German layouts are 1.5 MB
of JSON; stand choice uses curated airline areas (FRA, MUC) and curated size limits (CGN,
LEJ). Under the 2019 world traffic, Frankfurt and Munich escape 5–8 % of taxis and share
7–8 % of stands (PR 2, known issues).

| # | Decision |
|---|----------|
| D116 | **Import every scheduled airport** (3,908). The query box is the old 5.5 × 5.7 km box round the reference point, grown to every runway end plus 0.03° (about 3.3 km; Denver spreads over 13 km). *As built:* the Overpass mirror answered 504 to most requests, so the world comes from the OSM planet file instead: `osmium` (a documented system tool, like `curl`) streams it twice, for the aeroway features and then for their nodes, without storing it; the importer reads the result in OSM's line-based OPL text format (no new dependency) and cuts each airport's box from it. The Overpass reader stays for single airports. Raw files stay outside the repository |
| D117 | **Regional binary ground.** One asset per first letter of the airport ident (the ICAO region: E northern Europe, L southern Europe, K the United States, …), `assets/ground/<letter>.ground.bin`, in the M5 binary format (D104). `germany.ground.json` retires; tests take Germany from region E. The core accepts several ground definitions; the runner reads a directory of regions |
| D118 | **Quality gate at world scale.** The M4 gate (D66) unchanged: enough stands for the movement target, and stands linked to every runway end in use with the prevailing wind. The importer reports passed and rejected layouts per region with the reason; the snapshot in `docs/data-sources.md` records the totals |
| D119 | **Stand sizes from geometry only.** The spacing rule of M4 decides the ICAO code; `stand-sizes.json` (CGN, LEJ) retires (D95) |
| D120 | **Generic stand choice.** Among free stands that fit: the tightest fit (15 m steps, as M4), then the nearest to a stand the same airline holds now (250 m steps; airlines gather in their own areas by behaviour, not by list), then the nearest to the runway exit. Cargo airlines prefer and passenger airlines avoid aprons OpenStreetMap marks as cargo (apron tags or names), where any are marked. `stand-areas.json` and the core's stand areas retire |
| D121 | **M4 limits restored** as acceptance: escapes and stand shares each under 2 % of taxis, no taxi over 75 minutes. Where the generic choice does not meet them at Frankfurt and Munich, general behaviour is added (e.g. towing aircraft with long ground times to free stands); each addition is diagnosed and shown to the user first |
| D122 | **Scale measured.** Layouts are built once at scenario start; start-up time, memory and tick time with ground at every gated airport are measured and recorded (the 8 ms target is PR 5's) |
| D123 | **Tests.** Stand choice rules on fixtures; a guard test per region asset; the ground integration test runs the world with every region and checks the mapped airports, reported per region (a sample of the busiest gated airport per region letter if the full world is too slow for the suite) |
| D124 | **Viewer and runner.** Until PR 4 streams ground per airport, the viewer embeds regions E and L; the runner's `--ground` takes the directory of region files; `--stand-areas` retires |

Findings after the world import (agreed 2026-11-13): 641 of 3,908 scheduled airports pass the
gate. In a 4-hour morning with ground everywhere, 11.5 % of taxis share a stand and 4.2 %
escape a deadlock. Overnight fleets exceed the stands OSM maps (no remote or maintenance
parking): at 04:00 UTC Munich holds 429 aircraft on 189 stands, Frankfurt 469 on 284.

| # | Decision |
|---|----------|
| D125 | **Remote parking.** An aircraft whose next off-block is more than 2 hours after it reaches its stand is towed off to remote parking (unmapped, not simulated on the taxiways) once its turnaround allows, freeing the stand; it is towed back to a free stand an hour before off-block. If no stand is free it waits remote, counted as a stand wait; the warm start puts such aircraft remote from the outset |
| D126 | **Holding positions by rule.** Where OSM maps no holding position for a runway end in use, the importer places one on each taxiway joining that runway, at the Annex 14 distance from the centreline for the runway's code. Mapped holding positions always win |
| D127 | **Remote boarding at the peak** (agreed after D125 left 2.3–3.5 % shares in region E, mostly Oslo and Copenhagen). An arrival that finds no stand within 30 minutes at its waiting spot deplanes there and is towed to remote parking; a departure that finds no stand within 30 minutes of its tow-back boards remote and joins the taxiways at a free waiting spot, without pushback. A stand is shared only when neither a stand nor a spot is free; waiting spots are offered only where a stand that fits can be reached from them |
| D128 | **A gap in arrivals for crossings** (agreed after a 97-minute wait to cross Frankfurt's mixed-mode 25R, D113). Once an aircraft has waited 3 minutes to cross a runway and keeps asking, no further approaches to it are cleared, and the crossing no longer waits for booked slots of landings not yet cleared, only for those cleared. Arrivals absorb the gap as holding. A landing occupies the runway until it vacates, not a fixed minute. Departures queueing where OSM maps no holding position are paced by their runway end, and a runway's pace is also learnt from a departure's own wait at the hold |
| D129 | **Flat Web Mercator at every zoom** (agreed for PR 4). The far view is the whole world as a flat map, not a 3D sphere; one projection from the globe to a stand | Smallest change from D25; a sphere would need 3D cameras, every mesh re-projected and a transition between the two |
| D130 | **Floating origin.** Map positions are `f64` Web Mercator metres; the projection's origin moves to the camera centre whenever the camera is more than 10,000 pixels from it. Meshes and lines are built relative to their own anchor, and their transform is the anchor minus the origin narrowed to `f32`, so every drawn offset stays small: under a thousandth of a pixel of error at any zoom, anywhere | D106; replaces the fixed German origin (about 1 m of error at Sydney, against the centimetres a stand needs) |
| D131 | **Seamless wrap at the antimeridian** (agreed). East–west positions wrap by the world's width; everything is drawn at its copy nearest the camera, with a second copy where an item is wider than the rest of the view; the farthest zoom shows at most one world's width, so nothing is seen twice side by side | Trans-Pacific flights cross ±180° without jumping; Pacific views have no seam |
| D132 | **Ground: all at start, drawn on demand** (agreed). The simulation gets every region file at start, as the runner does: read from `assets/ground` on disk natively, fetched over HTTP on the web through Bevy's asset server (15 MB), behind a loading screen. Only the drawing is streamed: an airport's aprons, taxiways and stands are built when it comes into view at airport zoom and dropped when it is far away. The embedded regions E and L retire (D124); the world, basemap, aircraft and observed traffic stay embedded | Ground control behaves the same wherever the camera is (same seed + same inputs ⇒ same world); the web build does not carry the ground in its wasm |
| D133 | **World basemap and level of detail.** The viewer draws the world basemap (`basemap/world.bin`) in place of the German one; the home-country tint retires (D95). Zoom limits and level-of-detail thresholds extend to the world view; it opens on the whole world and Home returns there (agreed) | The German basemap ends a few hundred kilometres outside Germany |

*As built (PR 3):* 836 of 3,908 scheduled airports pass the gate (15 MB of region assets).
The integration test runs region E (northern Europe, 260 airports under ground control)
with M4's limits; a guard test loads every region and checks a hub on each continent (the
world-wide integration run of D123 stays a measurement, below, as it takes minutes per seed).
M4's limits hold for stand shares, separation and the longest taxi on seeds 3, 7 and 11;
escapes are 1.4 %, 2.0 % and 1.1 %, so seed 7 fails the one-in-fifty limit by one escape,
diagnosed in `docs/known-issues.md` and left for a follow-up. Scale (D122), the world for 4
hours from 04:00 UTC with ground everywhere, release build on 4 cores: start-up (model,
schedule, ground networks) about 13 s, 7.4 ms per tick on average, 399 MB peak memory,
escapes 2.1 % and stand shares 1.4 % of 14,610 taxis.

### PR 4 — Viewer for the globe

- World basemap with LOD, floating origin (D106), camera from globe to stand, ground
  streamed per airport, web build with fetched layouts (xtask serve).
- **Tests:** projection and origin maths (property tests), LOD and streaming logic behind
  traits; screenshots on three continents.

*Detail (agreed 2026-11-14, D129–D133):* the projection keeps `f64` positions and a movable
origin with wrap (pure, property-tested: round trip, wrap, rebase keeps every drawn point
within tolerance); static meshes and lines carry an anchor and are placed by one system
when the origin moves; which airports' ground to build and drop is a pure function of the
view, the zoom and what is built, behind a trait for the loader (disk, HTTP); the viewer
builds the simulation once the ground is in. `cargo xtask web build` copies
`assets/ground` next to the wasm. Measured in the PR: frame time at world zoom (about
12,000 aircraft) natively and on the web, start-up, wasm size. Screenshots: the world, a
Pacific crossing, and stands at Frankfurt, Atlanta and Shanghai Pudong.

### PR 5 — Scale and acceptance

- World benchmark (D107), 7-day world run, `docs/known-issues.md` and the definition of done.

## Definition of done

- Every airport with scheduled service is in the network; real airlines fly it; the world
  runs about 100,000 flights a day (2019 level within 10 %).
- No hand-curated per-airport or per-airline input remains (D95).
- German airports reproduce their ADV movements within the PR 2 tolerance (D105).
- Ground control at every scheduled airport that passes the quality gate; a 7-day world
  run with no stuck aircraft; escapes no worse than M4's rate.
- A tick under 8 ms for the full world (D107).
- The viewer, native and web, zooms from the globe to a stand anywhere.

## Risks

| Risk | Mitigation |
|------|------------|
| OpenSky coverage is thin in parts of Africa, Asia and South America | Coverage estimate per region; the catchment model fills thin regions (D97) |
| OpenSky 2019 is old and pre-dates some airports (e.g. new hubs) | Model fallback for airports without observations; documented as a known issue |
| World ground import (Overpass) is slow or rate-limited | Regional batches, resumable, raw files outside the repository |
| Memory and tick time with ground control at thousands of airports | Measured in PR 3; per-airport state only while aircraft are on its ground |
| Binary assets still too large for the repository | Regional split; Germany subset for tests; revisit Git LFS only if needed |
| Licences: ODbL share-alike, CC BY attribution | Attribution in the viewer and `docs/data-sources.md`; derived assets marked |
| Network access from the importer (OpenSky, NASA POWER, Overpass hosts) | Importer steps run where the hosts are reachable; outputs committed |
