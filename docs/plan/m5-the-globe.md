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

### PR 3 — Ground at world scale (data + core)

- Regional Overpass imports, resumable; binary ground per region; generic stand choice
  (D103); quality-gate report for the world.
- Memory and time per tick with ground control everywhere measured; layouts loaded once.
- **Tests:** stand choice rules; guard tests per region; the ground integration test on a
  sample of airports from every continent.

### PR 4 — Viewer for the globe

- World basemap with LOD, floating origin (D106), camera from globe to stand, ground
  streamed per airport, web build with fetched layouts (xtask serve).
- **Tests:** projection and origin maths (property tests), LOD and streaming logic behind
  traits; screenshots on three continents.

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
