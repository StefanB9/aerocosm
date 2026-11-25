# M9 — Runtime performance

**Goal:** the world runs well within real time on the reference machine, and the viewer keeps
up with a 120 Hz display. Runtime only: build times are out of scope. Weather, planned before
this milestone was chosen, moves to M10 with its decisions so far (2019 METARs replayed; wind
and runway changes, low visibility, thunderstorms and winter operations; the drawn delay
calibrated by cause as its last PR).

**Status:** approved 2026-11-20. Decisions continue M8's numbering.

## Baseline (2026-11-20)

The reference machine (D175) ran the full suite on `a80e567` (M8 plus Windows machine
detection) and recorded sampled profiles (samply) of start-up, the midday world and the viewer.

| Measure | Reference machine | 4-core cloud Xeon (`ce646b8`) |
|---|---|---|
| Midday tick: median / p95 / worst | 4.2 / 9.9 / 40.6 ms | 3.2 / 5.5 / 46.4 ms |
| Simulated day | 363 s (238 ticks/s) | 319 s |
| Start-up | 10.0 s | 21.0 s |
| Viewer CPU per frame, headless: world / Frankfurt, median (p95) | 12.3 (24.8) / 21.7 (26.2) ms | 22.2 (26.7) / 24.5 (30.1) ms |

The viewer's frame readout (F3, vsync at 120 Hz, simulated morning of 2026-11-05):

| Scene | Frame avg / worst | Ticks per frame | Simulation share |
|---|---|---|---|
| World, 60× | 21 / 33 ms | 1.3 | 19 % |
| World, paused | 16.7 / 18.8 ms | 0 | 0 |
| World, as fast as possible | 43.5 / 48 ms | 14.4 | 59 % |
| Frankfurt's stands, 60× | 26 / 40 ms | 1.9 | 25 % |
| Frankfurt's stands, paused | 17 / 18 ms | 0 | 0 |

Findings:

1. **The simulation uses one of 24 threads.** The fast CPU ticks slower than the cloud
   machine: the Windows build ignores the project's `target-cpu=native` (the Windows section of
   `.cargo/config.toml` replaces the shared flags instead of adding to them), the Windows C
   runtime's `sin`, `cos`, `hypot` and heap calls take about a tenth of a tick, and the
   scheduler may run the one busy thread on an efficiency core (the runs vary from 1.5 to
   3.4 ms at 06:00).
2. **Where a midday tick goes:** the ground survey 22 % (every tick it empties one list per
   taxiway node and chain at every airport with traffic, though few were written), flying 17 %
   (the route segment found by binary search and its trigonometry recomputed each tick),
   driving taxis 10 %, planning legs 8 % (choosing a departure runway computes an A* taxi route
   to every candidate runway end), parked tails 5 %.
3. **Start-up:** placing every tail at the warm start 72 % (the same A* per runway end 29 %,
   stand choice 15 %), generating the week's schedule 23 %.
4. **The viewer's own work, not the simulation, misses the frame.** Paused, it still takes
   8.3–16.7 ms (vsync rounds a frame to whole refresh intervals), about the same at the world
   view as at Frankfurt's stands: the work is per airframe, not per thing shown. Of its busy
   CPU, rebuilding the view of all 95,328 airframes after each tick (`sample_traffic`, on one
   thread) takes 20 %, the simulation 5 %, trails 5 %, aircraft sprites 4 %, and Bevy's
   batching, visibility and transforms for 262,000 entities (two per airframe, parked ones
   included) about 13 %. A tick inside a frame adds 4–9 ms; slow ticks make the 33–40 ms worst
   frames.

## Decisions

| # | Decision |
|---|----------|
| D175 | **Reference machine.** The user's Intel Core i9-12900KS (16 cores, 24 threads, 64 GB) on Windows with an RTX 3090 Ti and a 120 Hz display. Acceptance comes only from `cargo xtask bench` records and F3 readings made there; cloud figures are a sanity check |
| D176 | **Targets**, midday world with every ground region: tick p95 under 3 ms and worst tick under 10 ms; a simulated day under 120 s; start-up under 3 s; the viewer's frame p95 under 8.3 ms (120 fps) at the world view and at Frankfurt's stands, both in the headless benchmark and in F3 with vsync off |
| D177 | **Results identical.** Every PR keeps the determinism fingerprints (M5 D137). A change that would alter results is proposed on its own, with its effect on punctuality, escapes and stand shares, and goes in only with the user's agreement |
| D178 | **Parallelism with `rayon`** in the core: work is computed in parallel and applied in a fixed order, so a run is the same whatever the number of threads. A small internal wrapper keeps a sequential path for the web build (wasm has no threads without extra setup) |
| D179 | **Workflow per PR:** red-green TDD; fingerprints compared before and after in the cloud; then the user runs `cargo xtask bench` on the PR's branch and pushes the record to it, and `cargo xtask bench report` against the baseline decides whether the PR is a win before it is opened. Profiles are recorded again on the reference machine where a PR needs them |
| D180 | **Native CPU on every target.** The Windows and Linux sections of `.cargo/config.toml` repeat the shared `target-cpu=native` and `-Z location-detail=none`, so no target loses them. Measured as the first improvement against the `a80e567` baseline |
| D181 | **Vsync switch.** F4 turns the viewer's vsync off and on, and F3's readout says which, so frame times can be read without rounding to refresh intervals |
| D182 | **The viewer measures itself.** `aerocosm-viewer --measure` plays five scenes in its real window with vsync off (the world at 60×, paused and as fast as possible; Frankfurt's stands at 60× and paused), each settled for 5 s and measured for 10 s, and prints frame times, ticks per frame and the simulation's share as benchmark lines. `cargo xtask bench` runs it as the group `window`, by default wherever a display exists, so the reference machine's records carry the F3 readings (added on the user's request, 2026-11-21) |

## PR 1 — Native CPU and the vsync switch (D180, D181)

- `.cargo/config.toml`: the Windows section adds `-C target-cpu=native` and
  `-Z location-detail=none`; the Linux section adds `-Z location-detail=none`.
- The viewer: F4 switches between vsync and no vsync; F3's readout shows the mode.
- The baseline run (`a80e567`, from the reference machine's `target/bench-runs.jsonl`) is added
  to `benchmarks/history.jsonl`.
- Measured: the full suite on the reference machine, fingerprints equal to the baseline's; F3
  readings with vsync off.

*As built:* the flags, the F4 switch (F3's readout ends "vsync on" or "vsync off"; without a
window, as in the headless benchmark, it counts as on) and the baseline record. Fingerprints
are equal to the baseline's in every world scenario. The reference machine's record
(`542eb0f`) against the baseline (`a80e567`):

| Measure | Baseline | PR 1 | D176 target |
|---|---|---|---|
| Midday tick p95 / median / worst | 9.93 / 4.20 / 40.6 ms | 2.58 / 1.61 / 17.0 ms | p95 < 3, worst < 10 ms |
| Simulated day | 363 s | 138 s | < 120 s |
| Start-up | 9.98 s | 9.73 s | < 3 s |
| Viewer CPU per frame, headless, p95: world / Frankfurt | 24.8 / 26.2 ms | 8.3 / 10.0 ms | < 8.3 ms |

Every part of the tick fell by about half to four fifths; start-up hardly moved. F3 in the
viewer did not move (world at 60× 21 ms, paused 16 ms, Frankfurt at 60× 29 ms, paused
16.7 ms), although the viewer's CPU work per frame is now about 5.6 ms: the window's frames are
held by something the headless benchmark does not measure, the render thread's work for
262,000 entities or vsync. PR 2 is judged on F3 as well as on the headless benchmark.

## PR 2 — The viewer (D176)

- Aircraft are sampled after a tick only where they changed and can be seen; callsigns are
  built when shown; the sampling runs in parallel.
- Sprites exist only for aircraft that are shown, taken from a pool, instead of two entities
  for every airframe.
- Trails cost less: drawn only for aircraft on screen.
- The simulation ticks on a background task, so a tick no longer lengthens a frame; the frame
  shows the latest finished tick.
- Measured: the headless frames benchmark and F3 (vsync off) at the world view and Frankfurt.

*As built:* a profile of the viewer on PR 1's build showed the main and render threads mostly
waiting on Bevy's task pool, where the systems run: per frame, rebuilding the traffic view
after a tick took about 10.6 ms on one thread, and Bevy's batching, transforms and visibility
for 262,000 entities most of the rest.

- **Sprite pool:** `pool::Assignment` gives the aircraft shown (visible at the zoom, and on
  screen with a 128-pixel margin, `flights::in_view`) a slot each; an aircraft keeps its slot
  while shown, freed slots are reused before the pool grows. Sprites are spawned as the pool
  grows, never per airframe; trails, halos, picking and the callsign label follow the pool.
  Entities fell from about 262,000 to 99,000 at the world view and 73,000 at Frankfurt.
- **Sampling:** the core gives one aircraft's view at a time (`Simulation::flight`, built by
  the same function as `flights()`); each compute-pool task samples its own 2,048 airframes and
  updates their slots in place. A first version built the views in parallel and then copied
  them into the slots on one thread, which still cost 8–9 ms per tick.
- **Not built:** ticking on a background task. With the sampling in place, a frame with a
  tick costs about the tick itself more than a paused one; PR 3 and PR 5 shrink the tick, and
  background ticking is reconsidered at acceptance if world view at 60× still misses the
  target (decided with the user, 2026-11-24). Trails needed nothing beyond the pool.
- **Window measurement (D182)** was added here, on the user's request.

The reference machine's records: one taken while the machine was in use (browser, video) ran
untouched code about twice as slow and was dropped; `db421af` measured the pool, `6a12777` the
in-place sampling. Fingerprints are equal to the baseline's throughout.

| Real window, vsync off (median / p95) | PR 1, F3 average | Pool (`db421af`) | PR 2 (`6a12777`) | D176 target p95 |
|---|---|---|---|---|
| World, 60× | 21 ms | 16.0 / 18.2 ms | 7.3 / 9.0 ms | < 8.3 ms |
| World, paused | 16 ms | 5.6 / 5.8 ms | 5.5 / 5.7 ms | < 8.3 ms |
| World, as fast as possible | 43 ms | 40.4 / 46.0 ms (14 ticks) | 33.5 / 38.1 ms (14 ticks) | — |
| Frankfurt's stands, 60× | 29 ms | 13.3 / 14.6 ms | 3.1 / 6.8 ms | < 8.3 ms |
| Frankfurt's stands, paused | 16.7 ms | 3.0 / 3.3 ms | 2.9 / 3.3 ms | < 8.3 ms |
| Headless CPU, world / Frankfurt p95 | 8.3 / 10.0 ms (PR 1) | 3.8 / 2.3 ms | 4.1 / 2.1 ms | < 8.3 ms |

Every scene but the world view at 60× meets the target; there, the ~2.5 ms tick inside the
frame leaves p95 at 9.0 ms, and the slow ticks (worst 15 ms at midday) the 20 ms worst frames.

## PR 3 — Tick hotspots

- The ground survey empties only the lists it wrote to.
- Flying keeps its current route segment instead of searching for it each tick, and keeps its
  segment's trigonometry.
- Parked tails wait in a queue ordered by when they are next due.
- The benchmark reports the slowest ticks split by part; the cause of the 40 ms ticks is found
  and removed.

## PR 4 — Departure runway choice

- The runway end a departure takes no longer needs an A* taxi route to every candidate end:
  routes are reused where the ground's state allows, with results unchanged. If unchanged
  results prove impossible, a cheaper score is proposed under D177.

## PR 5 — Parallel ticks (D178)

- Flying in parallel per aircraft; ground control and taxi driving in parallel per airport;
  results applied in a fixed order.

## PR 6 — Start-up

- Placing tails at the warm start in parallel per airport; the independent parts of schedule
  generation in parallel.
- If start-up still exceeds 3 s: the generated schedule is cached on disk by the binaries
  (keyed by seed and inputs); the core stays free of I/O.

## PR 7 — Acceptance

- The full suite and F3 readings on the reference machine against D176; known issues, README
  and this plan's as-built notes; then `dev` is promoted to `main`.

## Definition of done

- D176's targets are met on the reference machine.
- Every world scenario's fingerprint equals the baseline's, except changes agreed under D177.
- The full test suite passes; the web build still runs (sequential path).

## Risks

| Risk | Mitigation |
|------|------------|
| Shared state (flow-control slot books, stands at both ends of a leg) forces sequential sections, limiting PR 5 | Compute in parallel, apply in order; what cannot be split is measured and left sequential |
| 8.3 ms per frame needs GPU-side instancing beyond PR 2 | Proposed as an extra PR if PR 2's measurements fall short |
| The hybrid CPU moves the busy thread between core types, so runs vary | Each scenario runs three times and is compared by range (M5 D141) |
| Background ticking changes what a frame shows (the latest finished tick, not one in progress) | Interpolation between ticks stays; the shown tick is never older than one tick |
