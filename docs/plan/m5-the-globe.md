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
| D122 | **Scale measured.** Layouts are built once at scenario start; start-up time, memory and tick time with ground at every gated airport are measured and recorded (the 8 ms target is PR 6's, plan changed 2026-11-15) |
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

*As built (PR 4):* the projection keeps `f64` Web Mercator positions with an origin that
moves to the camera once it is 10,000 pixels (or a quarter of the world) away; meshes are
built in their own frame (basemap per country and lake, three copies a world apart;
runways per 10° tile; each airport's ground) and one system places them at the copy nearest
the camera. The viewer opens on the whole world; `--view <lat>,<lon>,<metres per pixel>`
opens anywhere. Aircraft and airport symbols shrink to 40 % between country and world zoom.
Region files are read through Bevy's asset server and the simulation is built once all 21
are in; grounds are built within a quarter view of the screen below 30 map units per pixel,
at most two per frame, and dropped beyond one and a half views or above 120.
Two problems surfaced on the way and are fixed here. Labels: the world has about 102,000
(28,204 airports, 7,341 places, 66,252 runway ends, then taxiways and stands), each a hidden
UI text entity, about a gigabyte natively and walked by Bevy's UI every frame; they are now
plain sources and only placed labels get an entity. The schedule reserved ten round trips per
service, and the world has 10 million services (nearly all hardly ever flying): 4.4 GB of
untouched virtual memory natively, but more than wasm32 can address, so the browser aborted
while building the schedule (`capacity overflow`, reproduced with the core's schedule tests
on 32-bit x86); the reservation now follows expected trips.
A third: at 60× a frame owing more ticks than it could run owed more the next frame, up to
2,000, so the viewer now gives the simulation at most 25 ms of a frame and drops the rest.
Measured in this container (4 cores, no GPU): natively the window opens after 0.6 s, the 21
region files are in after 1.1 s and the simulation is ready after 12.9 s, with 1.0 GB of
memory then and 1.17 GB a minute later (2.2 GB before labels were made on demand). In headless
Chromium (WebGL2 build, `.wasm` 42 MB, WebGPU 40 MB) the files are fetched after 4.9 s and the
simulation is ready after 25.5 s. Frame times here are those of software rasterisers (Mesa
llvmpipe natively: about 500 ms at the world view, three quarters of the busy samples in the
rasteriser, the viewer's own systems a few per cent; SwiftShader in headless Chromium: about
7 s), so the frame time on a real GPU is still to be measured on hardware (PR 6, D145).

### PR 5 — Benchmark suite

- A benchmark suite that records how performance develops from commit to commit: the
  simulation at world scale, start-up and memory, micro-benchmarks, the viewer.

*Detail (agreed 2026-11-15, D134–D141).* Before this PR the only benchmarks were one
Criterion file (geometry, an empty-world tick, one world tick with regions E and L);
Criterion keeps its baselines in `target/`, so nothing recorded how performance changed.
Measured with the runner on 2026-11-14 (release build, 4 cores, ground everywhere, seed 3):
10–12 ms per tick in the hours from 06:00, 12:00 and 16:00 UTC (12,200–14,800 airborne),
above D107's 8 ms; a sampled profile of the midday hour put half the tick in ground control
and half in the tails. Whether and how to optimise is decided after this PR, on the suite's
numbers (PR 6).

| # | Decision |
|---|----------|
| D134 | **Scope.** Four groups, each runnable alone: *world* (ticks of fixed world scenarios, a simulated day), *start-up* (asset decoding, traffic derivation, simulation set-up, memory), *micro* (Criterion benchmarks of hot functions in the core and the viewer), *viewer* (CPU work of the viewer's frames, headless). Build times and binary sizes are not measured |
| D135 | **World scenarios.** The shipped world with all 21 ground regions, seed 3, warm start on Wednesday 2026-07-08 at 00:00, 06:00, 12:00 and 16:00 UTC: 60 ticks to settle, then 600 measured ticks. Each reports ms per tick (median, 95th percentile, worst), aircraft airborne and taxiing, and the split by subsystem (D136). A *day* scenario runs 24 hours from 00:00 and reports wall time, flights flown, and memory at the start and end. Each scenario runs in its own process, so peak memory belongs to it |
| D136 | **Tick split.** `Simulation::step_timed(&impl WallClock) -> PhaseTimes` times the phases of a tick (flow control, ground control, taxiing, flying, parked tails) with the existing `WallClock` trait. It gives exactly the results of `step` (tested); `step` reads no clock, and only benchmarks call `step_timed`, so the core stays deterministic |
| D137 | **Determinism fingerprint.** Each world scenario records a hash of the simulation after its run: FNV-1a (written locally, no dependency) over every flight view and the network statistics in a fixed byte order. When a change alters the fingerprint, it changed behaviour as well as speed |
| D138 | **Start-up.** Decoding times of each shipped asset (world, basemap, observed traffic, aircraft catalog, each ground region), traffic derivation, and `Simulation::new` (schedule, operations, ground networks) as one step, with peak memory after set-up. On Linux, memory comes from `/proc/self/status` (`VmHWM`, `VmRSS`); elsewhere it is reported as absent |
| D139 | **Micro-benchmarks** (Criterion, `bench-unwind` profile). Core: the existing geometry, plus taxi route search (A*) and stand choice at Frankfurt, junction tests, schedule generation for Germany. Viewer: label placement, the ground streaming plan, tessellation of a country and of Frankfurt's ground, reprojection of the labels on screen |
| D140 | **Viewer frames, headless.** A bench binary builds the viewer app without a window and without a GPU (Bevy's render plugin with no backend), with the shipped world and ground, and times each frame's CPU work at fixed views (the world, Europe, Frankfurt's stands) over 300 frames: median, 95th percentile, worst. If Bevy 0.20 cannot run the viewer's plugins without a backend, the app runs the systems that need no rendering, and the PR says which are left out. GPU frame time stays a manual measurement (PR 6) |
| D141 | **History.** `cargo xtask bench [group…]` builds and runs the suite and appends one record to the committed `benchmarks/history.jsonl`: format version, date, commit, toolchain, machine (CPU model, cores, memory, OS) and every result with its unit and, where measured, its spread. It records only from a clean working tree, so each record names a commit; a dirty tree prints the results without recording them. `cargo xtask bench report` compares the latest record with the previous one on the same machine, one line per benchmark with the change in per cent. No thresholds or failures yet: those follow the first benchmark step (PR 6). xtask's logic (records, machine info, comparison, Criterion's `estimates.json`) is tested behind its existing traits for processes and files; serde and serde_json (already in the workspace) enter xtask for the records |

Tests: the phase split sums to the tick and leaves the results unchanged; the fingerprint
changes when a flight changes and not otherwise; record round trip, comparison by machine,
and the clean-tree rule in xtask; each bench target runs in a quick mode in the test suite
(a few ticks, one frame), so benches cannot rot. The first record (this PR's commit, this
container) is committed with the PR.

*As built (PR 5):* the world scenarios live in the runner, `aerocosm bench
<startup|tick-HH|day>`, which already reads the assets from disk; `cargo xtask bench` runs
each in its own process. They settle 30 simulated minutes, not 60 ticks: warm start leaves
the taxiways empty for minutes. The tick split times every sixteenth tail, rotating per tick,
less the clock's own cost (the cheapest of 16 back-to-back readings): timing every tail
inflated the parts by a quarter; now they come within about 10 % of the timed ticks'
own mean, recorded beside them. The viewer became a library with a thin binary so that its
benches can use it; its `bench` module prepares inputs and keeps the internals private, and
the headless viewer (Bevy's render plugin without a backend, no winit, a 1440×900 window
entity) runs every viewer plugin. Left out: schedule generation for Germany (the German world
alone has too few observed airports to fit the traffic model; the world's schedule is timed
in `startup/simulation`) and junction tests (internal to the core). The old world tick with
regions E and L retired. Criterion benches compile with `--all-targets`; the runner's
scenarios and the headless viewer run in the test suite; `cargo xtask bench --quick` runs
everything briefly. Each run's record is also kept in `target/bench-runs.jsonl`.

First record (commit 0deba8d, this container: 4 cores of a 2.1 GHz Xeon, no GPU): start-up
10.9 s, of which `Simulation::new` 10.3 s; 408 MiB peak. Ticks (median / 95th percentile /
worst, 12,100–14,400 airborne, 610–780 taxiing): 8.3 / 12.1 / 24 ms at 00:00, 7.3 / 10.4 /
21 ms at 06:00, 8.7 / 12.5 / 29 ms at 12:00, 8.5 / 12.5 / 26 ms at 16:00 UTC. Of a timed
tick of 8.7–10.3 ms, ground control takes 4.2–5.0 ms, parked tails 2.6–2.8, flying 1.2–1.8,
taxiing 0.8–1.1. A simulated day: 843 s (102 ticks a second), 97,672 flights, memory 390 to
429 MiB. The viewer's CPU work per frame (median): 15 ms at the world view and over Europe,
21 ms at Frankfurt's stands, with about 259,000 entities. Noise: the same ticks vary by about
a quarter from run to run on this shared machine.

### PR 6 — Scale and acceptance

- Benchmark step, then optimisation as needed, 7-day world run, `docs/known-issues.md` and
  the definition of done.

*Agreed 2026-11-14, moved here 2026-11-15:*

*Benchmark step (2026-11-16):* the first record (PR 5) has tick medians of 7.3–8.7 ms and
95th percentiles of 10.4–12.5 ms. A sampled profile of the midday hour: ground control 44 %
of the tick (its survey alone 25 %, which walks the whole fleet of about 30,000 tails to find
the 780 taxiing and clears every junction list of every mapped airport), parked tails 16 %
(checked every tick with nothing due), flying 14 %, taxiing 10 %. Agreed: the optimisation of
D142 as described (taxiing tails in a list, only airports with traffic surveyed, parked tails
passed by until something is due), with every world scenario's fingerprint unchanged; D107's
8 ms counts as the 95th percentile of every tick scenario on this machine.

| # | Decision |
|---|----------|
| D142 | **Benchmark step first.** The suite runs on the merged PR 5; the numbers and the profile are shown to the user, and they decide whether optimisation is needed and which. Earlier work (tails that taxi kept in a list instead of searching the fleet each tick, parked tails passed by until something is due) is kept aside for this; any optimisation must keep the determinism fingerprint (D137) unchanged unless agreed otherwise |
| D143 | **Escapes accepted for M5** at the world's rate (2.1 %, M4: 2.3 %). The fix (a junction cleared only to the first in line, one clearance for a short stub) follows M5; until then the ground test's seed 7 stays over its limit by one escape, as recorded in `docs/known-issues.md` |
| D144 | **Longest taxi reported.** Statistics keep the longest taxi per airport and network, including taxis still under way; the runner reports it. "No stuck aircraft" is M4's limit: no taxi over 75 minutes in the 7-day run |
| D145 | **Frame-time readout.** A key toggles a small readout in the viewer, native and web: frame time (average and worst of the last seconds), ticks run per frame and the simulation's share of the frame. Its text is a pure, tested function. The user measures on real hardware; the numbers are recorded here |
| D146 | **Acceptance run.** The world for 7 days, seed 3, ground everywhere: flights a day, German validation (D114), escapes, stand shares, longest taxi, tick time and memory, each against the definition of done, recorded here and in `docs/known-issues.md` |

*As built (PR 6):* the optimisation of D142 keeps results identical: every world scenario's
fingerprint, the simulated day's included, matches the PR 5 record. A first version left
pushback approvals off the list of airports whose per-tick lists are emptied, and a lone
aircraft's lists went stale; the fingerprints showed it at tick 37 and a test now checks the
invariant. The simulated day takes 519 s instead of 843 s. The longest taxi (D144) is kept
per airport and network and reported by the runner, with a warning above 75 minutes; the
viewer's frame-time readout (D145) is on F3.

Acceptance run (D146), the world for 7 days from Monday 2026-07-06, seed 3, ground at every
gated airport, release build on this container's 4 cores: 704,209 flights (100,600 a day),
peak 15,868 airborne, 6.6 ms per tick on average (151 ticks a second, 67 minutes in all),
peak memory 447 MiB. Escapes 24,857 of 750,724 taxis (3.3 %), stand shares 14,117 (1.9 %),
go-arounds 9,956; 70 % of departures and 60 % of arrivals on time. The first run found an
arrival at Melbourne waiting at a spot for the whole week (a taxi route with no edges had no
end node; fixed). In the second, 23 airports still have a taxi over 75 minutes (Phoenix 632)
and escapes concentrate at a few airports (Ankara, Boston, O'Hare, Oakland, Madrid); agreed
(2026-11-16): recorded in `docs/known-issues.md`, fixed in a milestone after M5.

Benchmark record after PR 6 (`cargo xtask bench`, against the PR 5 record on the same
container): tick medians 3.3–4.7 ms and 95th percentiles 5.0–7.3 ms at 00, 06, 12 and 16 UTC
(PR 5: 7.3–8.7 and 10.4–12.5), so D107's 8 ms holds for the 95th percentile; ground control
1.5–2.0 ms of a tick (was 4.2–5.0), parked tails 0.5–0.8 ms (was 2.6–2.8). The simulated day
takes 469 s (was 843). The fingerprints differ from PR 5's because the statistics now hold
the longest taxi and the Melbourne fix changes behaviour (97,667 flights a day instead of
97,672). The same run shows untouched code (geometry, route search, basemap decoding) 15–90 %
slower than in PR 5's record: the shared host was slower that day, which also explains the
slower start-up and viewer frames; records from the same CPU model can differ by a third.

## Definition of done

- Every airport with scheduled service is in the network; real airlines fly it; the world
  runs about 100,000 flights a day (2019 level within 10 %).
- No hand-curated per-airport or per-airline input remains (D95).
- German airports reproduce their ADV movements within the PR 2 tolerance (D105).
- Ground control at every scheduled airport that passes the quality gate; a 7-day world
  run with no stuck aircraft; escapes no worse than M4's rate. *Not met (PR 6): 23 airports
  have a taxi over 75 minutes and escapes are 3.3 %; recorded in `docs/known-issues.md` and
  moved to a milestone after M5.*
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
