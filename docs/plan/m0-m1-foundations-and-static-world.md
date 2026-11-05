# Plan: M0 Foundations & M1 Static World

Status: **approved** (2026-11-05)

## Vision

*A complete aviation world in miniature.*

1. **Step 1 — fully simulated.** A deterministic simulation of real-world aviation: airports,
   aircraft, airlines, schedules, flight phases, ATC, later demand, economy, weather,
   maintenance and crews. Starts with Germany; grows to Europe and the world. Target scale:
   the ~100k flights/day the real world flies.
2. **Step 2 — influenceable.** Sandbox "god" interventions (close an airport, spawn a storm,
   ground a fleet) injected as timestamped commands. It stays a simulation at heart — not a
   game.

The viewer zooms continuously from country level down to a single airport.

## Decisions log

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | Real-world data, start with Germany | User requirement |
| D2 | Kinematic fidelity: aircraft move along great circles with phase profiles; realistic routes (airways, SID/STAR) later | Gives positions and meaningful ATC without flight-dynamics cost |
| D3 | Rendering: **all-Rust, Bevy 0.20**, native desktop + wasm web | User choice (option B) |
| D4 | No ECS framework in the core: plain structs, typed `Id<T>`, `Vec` storage | 100k flights/day ≈ 10–15k concurrent airborne; trivially within budget. A benchmark guards it from M2 |
| D5 | Deterministic, tick-based core (1 s fixed tick, seeded ChaCha8, per-subsystem RNG streams) | Testability, replay, and clean step-2 command injection |
| D6 | Time: integer ms `SimTime`; real-time pacing with default **60×**, plus as-fast-as-possible batch mode | User requirement: both modes, sensible default |
| D7 | Foreign traffic terminates at a curated set of **edge hubs** | Keeps the Germany scope closed but realistic (list decided in M2 planning) |
| D8 | Schedules (M2): synthetic gravity-model schedules flown by **real airlines** (real ICAO/IATA codes) | Real schedules are proprietary (OAG/Cirium) |
| D9 | M1 map shows **all public German aerodromes with runways** (~400); only commercial airports get scheduled traffic | Map feels alive; minor airfields fade in by zoom level |
| D10 | `serde` (derive) is a regular dependency of `aerocosm-core` | World file loading now, save/load later |
| D11 | Viewer-crate-only allows for `clippy::needless_pass_by_value` and `clippy::too_many_arguments`, with a justification comment | Bevy systems take `Res`/`Query` by value and often have many params |
| D12 | Self-rendered vector basemap from Natural Earth; no web tile services | OSM tile policy forbids it; offline; fully styleable; wasm-friendly |
| D13 | Planning depth: M0–M1 detailed, later milestones planned when reached | User choice |
| D14 | Always feature branch + PR into `dev` | CLAUDE.md branch rules |
| D15 | M1 ships as three PRs: core world model → data pipeline → viewer | Reviewable units; each builds on the merged previous one |
| D16 | Tests and implementation share a commit; red runs are noted in the PR | Every commit stays green, as CLAUDE.md requires |
| D17 | Airports keyed by `AirportIdent` (OurAirports ident: ICAO code or `DE-xxxx`); `icao` is optional | 65 of 436 German aerodromes have no ICAO code. Scheduled traffic only uses ICAO airports |
| D18 | Missing runway thresholds are synthesised (centred on the reference point, heading from data or designator) and flagged `GeometrySource::Approximate` | ~half of small-airfield runways lack coordinates; flag allows OSM replacement later |
| D19 | `AirportKind = Major \| Regional \| Airfield \| Edge` (was `Hub`) | OurAirports `large_airport` includes e.g. Paderborn, which is no hub; "hub" is an airline concept for M2 |
| D20 | Aircraft types live in a hand-curated catalog (`assets/aircraft/types.json`, `AircraftCatalog`) separate from the generated world file | Different sources and lifecycles: the importer regenerates geography; the catalog is edited by hand |
| D21 | Basemap file types live in a small `aerocosm-basemap` crate (written by `aerocosm-data`, read by the viewer) | Keeps the viewer independent of importer dependencies and the core free of map-drawing data |
| D22 | Airport ident = OurAirports `icao_code` when valid, else the OurAirports ident; `icao` is set only when the ident is an ICAO code | Some airfields have local idents (`DE-0004`) but real ICAO codes (`EDUG`); keeps ICAO lookups complete |

## Research findings (2026-11-05)

- `bevy` 0.20.1 is current (0.20.0 released 2026-10-08). `bevy_egui` 0.43 supports 0.20.
  `bevy_prototype_lyon` 0.17 and `bevy_slippy_tiles` 0.12 still target 0.19 → **not used**;
  we tessellate with `lyon_tessellation` (Bevy-independent) and build `Mesh`es ourselves.
- **OurAirports** (public domain): `airports.csv`, `runways.csv` — idents, positions,
  elevation, type, scheduled-service flag, runway end coordinates, headings, length, width,
  surface.
- **Natural Earth** (public domain): admin-0 countries, admin-1 states, lakes, rivers,
  populated places (GeoJSON from `nvkelso/natural-earth-vector`).
- **OpenAP** (TU Delft, LGPL-3.0): used only to cross-check aircraft performance figures; no
  data files are copied. **BADA** (Eurocontrol) is licence-restricted → not used.
- **Inter** typeface (SIL OFL 1.1) bundled for the UI.
- `.cargo/config.toml` sets `-C target-cpu=native` under `[build]` and uses `build-std`.
  Compatibility with `wasm32-unknown-unknown` must be verified in M1 (may need per-target
  rustflags).
- `cargo-nextest` and `cargo-edit` are not installed in fresh cloud containers; install
  before running the checklist.

## Target workspace layout (end of M1)

```
crates/
├── aerocosm-core/        library — pure simulation, no I/O, no Bevy
│   └── src/
│       ├── lib.rs
│       ├── error.rs      CoreError
│       ├── id.rs         Id<T>
│       ├── time.rs       SimTime, SimDuration, civil date conversion
│       ├── units.rs      Meters, Feet, Knots, Degrees
│       ├── geo/          LatLon, great-circle math
│       ├── rng.rs        SimRng (ChaCha8, forkable streams)
│       ├── pacing.rs     WallClock trait, SimSpeed, Pacer
│       ├── sim.rs        Simulation, fixed-step loop
│       └── world/        Airport, Runway, AircraftType, codes, World + validation
├── aerocosm-basemap/     library — basemap file types (D21)
├── aerocosm-data/        library + `import` binary — offline data pipeline
├── aerocosm/             binary — headless CLI runner (batch mode)
└── aerocosm-viewer/      binary — Bevy viewer, native + wasm
assets/
├── world/germany.world.json        generated, committed
├── basemap/germany.basemap.json    generated, committed
└── fonts/Inter-*.ttf
docs/data-sources.md                URLs, licences, attribution, regeneration steps
```

Dependency boundary: `core` ← `basemap` ← `data`, `viewer`; `core` ← `aerocosm`. The viewer reads the world file
through core types; it never depends on `aerocosm-data`.

## M0 — Foundations

Everything in `aerocosm-core`. Each row is one (or a few) red-green-refactor commits.

| Module | Contents | Tests |
|--------|----------|-------|
| `id.rs` | `Id<T>`: `u32` index + `PhantomData<fn() -> T>`; `Copy`, `Eq`, `Hash`, `Ord`, `Debug`, `serde` | Unit: equality, ordering, index round-trip |
| `time.rs` | `SimTime(i64 ms UTC since Unix epoch)`, `SimDuration(i64 ms)`; checked `add`/`sub`/`since`; `from_civil(y, m, d, h, min, s)` / `to_civil()` via Hinnant's algorithm; `Display` as ISO-8601 | Proptest: civil round-trip across ±10k years, arithmetic associativity, `since` antisymmetry; unit: known epochs, leap days |
| `units.rs` | `Meters`, `Feet`, `Knots`, `NauticalMiles`, `Degrees` (`f64` newtypes) with conversions | Proptest: conversion round-trips within tolerance |
| `geo/` | `LatLon::new(lat, lon) -> Result<Self>` (rejects out-of-range/non-finite); `distance` (haversine, mean Earth radius), `initial_bearing`, `destination(bearing, distance)`, `interpolate(other, fraction)` | Proptest: symmetry, triangle inequality, `destination∘distance` round-trip, interpolation endpoints & monotonic distance, bearing ∈ [0, 360); unit: published reference distances (e.g. FRA–MUC) |
| `rng.rs` | `SimRng::from_seed(u64)`, `fork(StreamId) -> SimRng`; `StreamId` enum (one variant per subsystem) | Same seed ⇒ same sequence; distinct streams diverge; fork is independent of parent draw count |
| `pacing.rs` | `trait WallClock { fn now(&self) -> WallInstant; }`; `SimSpeed` (`Paused`, `X1`, `X10`, `X60` default, `X300`, `X1000`, `Max`); `Pacer::ticks_due(&mut self, clock) -> u32` with max-ticks-per-frame cap and carried remainder | Mock clock: exact tick counts per speed, pause, cap prevents spiral, remainder carries; proptest: total ticks over many frames ≈ elapsed × speed |
| `sim.rs` | `Simulation { world, time, rng }`, `TICK = 1 s`, `step()`, `run_until(SimTime)`; subsystems are plain functions called in a fixed order (no `dyn`) | Determinism: two sims with the same seed reach identical state after N ticks; time advances exactly N × TICK |
| `error.rs` | New `CoreError` variants (invalid coordinate, time overflow, invalid code, invalid world reference, …) | Display test per variant |

**`aerocosm` binary:** `aerocosm run --world <path> --seed <u64> --duration <e.g. 24h>`
loads a world, runs in batch mode, logs a summary. CLI via `clap` (derive).
`#[instrument]` and `info!` lifecycle events per CLAUDE.md.

**Benchmark harness:** `criterion` set up in core with geo micro-benchmarks. The
15k-concurrent-aircraft step benchmark is added in M2.

## M1 — Static world & first look

### Core domain (`world/`)

- Codes: `AirportIdent` (1–8 of `A-Z0-9-`), `IcaoCode` (4 letters), `IataCode` (3 letters),
  `AircraftTypeCode` (2–4, letter first), `CountryCode` (2 letters); fixed-size ASCII,
  validated `TryFrom<&str>`, `Display`, `serde` as strings.
- `Airport { ident, icao: Option, iata: Option, name, location, elevation: Feet, kind:
  AirportKind, country }`, `AirportKind = Major | Regional | Airfield | Edge` (D17, D19).
- `Runway { airport, ends: [RunwayEnd; 2], length: Meters, width: Meters, surface: Surface,
  geometry: GeometrySource }`, `RunwayEnd { designator: RunwayDesignator, threshold: LatLon,
  true_heading: Degrees, displaced_threshold: Meters }`, `GeometrySource = Surveyed |
  Approximate` (D18). Designators are `01`–`36` with optional `L`/`C`/`R`.
- In the world file, runways reference their airport by `AirportIdent` (readable diffs);
  `World` resolves them to `Id<Airport>` and stores runways grouped by airport.
- `AircraftType { code, name, wake: WakeCategory, seats, cruise_speed, cruise_level,
  climb_rate, descent_rate, approach_speed, range, min_runway_length }` — ~15 curated types
  (A319/A320/A321 incl. neo, B737-800/MAX 8, A220-300, E190/E195, CRJ900, ATR 72, DHC-8-400,
  A330, A350, B787, B777, A380, B747-8), each with a source citation. Loaded as a separate
  `AircraftCatalog` from `assets/aircraft/types.json` (D20).
- `WorldDefinition` (serde DTO) → `World::from_definition(def) -> Result<World>` validates
  unique codes, resolvable references, runway geometry (ends ≈ length apart, headings ≈
  reciprocal, designators reciprocal) and builds lookup indices by ident and ICAO code.
- `Simulation` owns the `World`; `aerocosm run --world <path>` loads it.

Tests: unit tests per constructor and validation failure; proptests for code parsing and
for validation (randomly corrupted definitions are always rejected).

### Data pipeline (`aerocosm-data`)

- `import` binary: `aerocosm-data import --ourairports <dir> --natural-earth <dir> --out
  assets/`. Reads local downloads only (no network code).
- Filters German aerodromes (`iso_country = DE`, excluding heliports, closed and balloon
  ports) with at least one open runway; classifies `Major` (large) / `Regional` (medium or
  scheduled service) / `Airfield`. Edge hubs are added in M2.
- Normalises free-text surfaces into `Surface`, drops helipad and malformed runway rows,
  synthesises missing thresholds (D18), defaults missing widths by surface.
- Basemap: clips Natural Earth layers to a Germany + margin bounding box, simplifies with
  Douglas–Peucker (own implementation, tested), writes polygons/polylines per layer.
- Outputs are deterministic (stable ordering) so regenerated files diff cleanly.
- `docs/data-sources.md`: download URLs, versions/dates, licences, required attribution,
  regeneration commands.
- Tests: parsing against small fixture CSV/GeoJSON files in `tests/fixtures/`;
  classification rules; simplification proptests (endpoints kept, error ≤ tolerance,
  output ⊆ input points).

### Viewer (`aerocosm-viewer`, Bevy 0.20)

- **Projection:** Web Mercator in `f64`; floating origin (entities placed relative to camera
  focus) so runway-scale zoom has no `f32` jitter.
- **Camera:** drag-pan with inertia, cursor-anchored wheel zoom, pinch zoom on touch, zoom
  limits from Germany-wide to ~zoom 16.
- **Basemap:** meshes tessellated once at startup (`lyon_tessellation`): land, neighbours,
  lakes fill; borders, state lines, rivers stroke.
- **Airports:** glyph size by `AirportKind`; zoom-dependent level of detail and label
  fading; collision-aware labels (simple greedy placement).
- **Runways:** true-scale quads with designators, visible from ~zoom 11.
- **UI (`bevy_ui`):** top bar with UTC clock and speed controls (time runs; nothing moves
  until M2); hover highlight; selection panel with airport details and runway list.
- **Style:** minimalist dark theme (deep navy land/water contrast, soft borders, single
  accent colour), Inter font, anti-aliasing (MSAA), smooth transitions.
- **Web:** `wasm32-unknown-unknown` via `trunk`; verify `.cargo/config.toml` flags (see
  findings) and adjust per-target if needed.
- **Tests:** proptest for project/unproject round-trip; headless `App` (`MinimalPlugins`)
  tests for camera clamping, LOD thresholds, selection; Playwright screenshot of the wasm
  build in headless Chromium as the visual check.
- `[lints]` workspace + D11 allows at crate root.

### Dependencies

All declared in `[workspace.dependencies]` with `default-features = false`; exact feature
lists are pinned during implementation.

| Crate | Used by | Why | Alternatives considered |
|-------|---------|-----|-------------------------|
| `serde` (derive) | core, data | World file (de)serialisation; save/load later | Hand-written parsing — too much code |
| `serde_json` | core (load), data | Human-diffable world file | `postcard` — smaller but not reviewable; revisit when files grow |
| `rand_core`, `rand_chacha` | core | Deterministic, portable (wasm) seeded RNG | `rand_pcg` — fine too; ChaCha8 has stronger stream separation |
| `clap` (derive) | aerocosm | CLI parsing | Hand-rolled `std::env::args` — error-prone |
| `csv` | data | OurAirports parsing | Manual split — breaks on quoted fields |
| `geojson` | data | Natural Earth parsing | `geozero` — heavier |
| `bevy` (minimal 2D + UI + text + winit + webgl2) | viewer | Rendering, input, UI, wasm | Option A (web stack) rejected by user |
| `lyon_tessellation` | viewer | Polygon/stroke tessellation | `bevy_prototype_lyon` — not on Bevy 0.20 |
| `criterion` (dev) | core | Benchmarks | `divan` — less maintained |

### M1 definition of done

- `aerocosm-data import` regenerates both asset files byte-identically.
- `aerocosm run --world assets/world/germany.world.json --duration 24h` completes and logs.
- Viewer runs natively and in the browser: Germany basemap, ~400 aerodromes, labels by
  zoom, true-scale runways at airport zoom, selection panel, running clock with speed
  controls.
- Full pre-merge checklist green.

## Risks

| Risk | Mitigation |
|------|------------|
| Bevy 0.20 is weeks old; ecosystem lags | Depend only on `bevy` itself + Bevy-independent crates |
| `target-cpu=native` / `build-std` vs wasm | Verify early in M1; per-target rustflags |
| No GPU in cloud containers | Visual checks via wasm build in headless Chromium |
| Pedantic clippy friction with Bevy | Scoped allows (D11) only |
| Natural Earth resolution too coarse at high zoom | Acceptable for M1; OSM-derived detail in the airport-zoom milestone |

## Roadmap after M1 (planned when reached)

- **M2 — Living sky:** airlines, fleets, gravity-model schedules, edge hubs, flight phase
  state machine, great-circle kinematics, aircraft rendering, flight strips, departure
  boards, 15k-aircraft benchmark.
- **M3 — Airside realism v1:** runway occupancy and separation, arrival sequencing,
  holding, gate capacity, delay propagation.
- Later: demand & economy → weather → airport-level zoom (OSM aeroways, taxi routing) →
  realistic routes (airways, SID/STAR) → maintenance & crew → Europe → world → save/load
  & replay → step 2 god-mode commands.
