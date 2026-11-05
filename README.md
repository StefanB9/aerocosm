# aerocosm

A complete aviation world in miniature: a deterministic simulation of real-world aviation,
starting with Germany. Step 1 simulates; step 2 lets you intervene as a sandbox "god".

Current state (milestone M1): the simulation core, the German world (429 aerodromes, 518
runways), a basemap of Germany and its neighbours, and a Bevy viewer for desktop and browser.
Nothing flies yet — aircraft movement arrives in M2.

Plans live in [`docs/plan/`](docs/plan/), data provenance in
[`docs/data-sources.md`](docs/data-sources.md), engineering rules in [`CLAUDE.md`](CLAUDE.md).

## Contents

- [Prerequisites](#prerequisites)
- [Automated checks](#automated-checks)
- [Headless simulation](#headless-simulation)
- [Desktop viewer](#desktop-viewer)
- [Browser viewer](#browser-viewer)
- [Regenerating the data](#regenerating-the-data)
- [Benchmarks](#benchmarks)
- [Screenshots without a display](#screenshots-without-a-display)
- [Workspace](#workspace)

## Prerequisites

- **Rust nightly.** `rust-toolchain.toml` selects it; `rustup` installs it on first use.
- **cargo-nextest**, the test runner:

  ```bash
  curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C ~/.cargo/bin
  ```

- **Linux packages for the viewer** (Debian/Ubuntu names):

  ```bash
  sudo apt-get install libwayland-dev libxkbcommon-dev libxkbcommon-x11-0 libudev-dev libasound2-dev
  ```

  macOS and Windows need nothing extra.

- **For the browser build only:** the wasm target and a `wasm-bindgen` CLI of exactly the
  version in `Cargo.lock` (currently 0.2.130). Prebuilt binaries are on the
  [wasm-bindgen releases](https://github.com/wasm-bindgen/wasm-bindgen/releases); or build it:

  ```bash
  rustup target add wasm32-unknown-unknown
  ```

  ```bash
  cargo install wasm-bindgen-cli --version 0.2.130 --locked
  ```


## Automated checks

The pre-merge checklist; all four must pass with zero warnings.

```bash
cargo fmt --all -- --check
```

```bash
cargo clippy --workspace --all-targets
```

```bash
cargo check --workspace --all-targets
```

```bash
cargo nextest run --workspace
```

The first viewer build compiles Bevy and takes several minutes. Guard tests load the
committed asset files, so a broken regeneration or hand edit fails here.

## Headless simulation

Runs the simulation as fast as possible and logs a summary:

```bash
cargo run -p aerocosm -- run --world assets/world/germany.world.json --duration 24h
```

What to check:

- The log reports `airports=429 runways=518 ticks=86400`.
- `--seed 7`, `--start 2026-11-05T06:00:00Z` and durations like `1h30m` or `7d` are accepted.
- Bad input fails with a clear message, e.g. `--duration soon` or `--world missing.json`.
- `cargo run -p aerocosm -- --help` lists the options.

## Desktop viewer

```bash
cargo run -p aerocosm-viewer
```

For a smoother experience use `cargo run --release -p aerocosm-viewer` (slower to build).

### Controls

| Input | Action |
|-------|--------|
| Drag | Pan (keeps coasting when released) |
| Mouse wheel / touchpad / pinch | Zoom towards the pointer |
| Arrow keys or WASD | Pan |
| `+` / `-` | Zoom in / out |
| Home | Back to the whole of Germany |
| Click an airport | Select it and open its panel |
| Click empty map / Escape | Clear the selection |
| Space | Pause / resume |
| `,` / `.` | Slower / faster |
| Speed buttons (top right) | Pause, 1×, 10×, 60× (default), 300×, 1000×, Max |

### Manual test checklist

1. **Start-up view:** Germany fills the window on a dark map; neighbouring countries, state
   borders, lakes and major rivers are visible; there is no hard map edge anywhere.
2. **Labels:** major airports show IATA codes (FRA, MUC, BER, …) and big cities their names
   (Berlin, Hamburg, Frankfurt, Munich, Amsterdam, Prague, …); no labels overlap.
3. **Clock:** the top bar starts at `Thu 5 Nov 2026 06:00:00 UTC` and advances one simulated
   minute per real second at 60×.
4. **Speed:** click each speed button — the active one is highlighted and the clock rate
   changes; `Pause` stops it; Space toggles pause and resumes the previous speed; `,` and `.`
   step through the speeds.
5. **Zoom:** scroll over Frankfurt — the point under the pointer stays put. Regional airports,
   then airfields and smaller towns, appear progressively; labels never overlap.
6. **Runways:** keep zooming into Frankfurt — four runways appear to scale (07L/25R, 07C/25C,
   07R/25L and 18), each end labelled with its designator.
7. **Pan:** drag the map and release while moving — it coasts and slows down. The view stops
   at the edge of the basemap.
8. **Hover and select:** hovering an airport shows a ring; clicking it shows an accent ring and
   a panel (top right) with name, ICAO · IATA · kind, position, elevation and every runway
   (length × width, surface, "approx. position" where the geometry was synthesised — try a
   small airfield). Escape or a click on empty map closes it.
9. **Cursor readout:** the bottom right shows the latitude and longitude under the pointer.
10. **Keyboard:** arrows/WASD pan, `+`/`-` zoom, Home returns to Germany.
11. **UI vs map:** clicking the speed buttons never pans the map or deselects an airport.

## Browser viewer

Build both variants (WebGPU and WebGL2) into `target/web`, then serve them:

```bash
cargo xtask web build
```

```bash
cargo xtask web serve
```

Open <http://localhost:8080>. The page uses the WebGPU build when the browser can actually
render with WebGPU (it renders one tiny probe frame first) and the WebGL2 build otherwise.
Force one with <http://localhost:8080/?renderer=webgl2> or
<http://localhost:8080/?renderer=webgpu>.

Each variant's `.wasm` is currently about 33 MB uncompressed; the first load takes a moment.

The release build of both variants takes a while; `cargo xtask web build --dev` builds faster
but produces much larger, slower files. `cargo xtask web serve --port 9000` picks another
port; the server only listens on localhost. All project tooling is Rust (`crates/xtask`);
`cargo xtask --help` lists the tasks.

### Manual test checklist

1. Run the [desktop checklist](#manual-test-checklist) in the browser.
2. In the developer tools console, `document.documentElement.dataset.renderer` shows which
   build was loaded; check both `?renderer=webgpu` and `?renderer=webgl2`.
3. Resize the window — the canvas fills it and the map stays sharp.
4. On a touch device (or touch emulation in developer tools): one finger pans, two fingers
   pinch-zoom.
5. With a browser that has no working WebGPU, the default page still loads (WebGL2 build).
   Headless Chromium is such a browser: it offers a WebGPU adapter and device, but rendering
   fails, so automated checks there exercise the WebGL2 build and the fallback.

## Regenerating the data

The committed world and basemap files come from public datasets; the steps, the exact
snapshot and checksums are in [`docs/data-sources.md`](docs/data-sources.md). In short:
download the OurAirports CSVs and the Natural Earth layers listed there into a scratch
directory, then

```bash
cargo run --release -p aerocosm-data -- import --ourairports /tmp/aerocosm-data/ourairports --natural-earth /tmp/aerocosm-data/natural-earth --out assets
```

What to check:

- The log reports the import counts and every skipped record and fallback by reason.
- With the documented snapshot, `git diff assets/` is empty: output is byte-identical.
- `cargo nextest run --workspace` passes (the guard tests load the regenerated files).

## Benchmarks

```bash
cargo bench -p aerocosm-core --bench core --profile bench-unwind
```

The `bench-unwind` profile is required (see `CLAUDE.md`).

## Screenshots without a display

How the viewer was checked in a headless Linux container (Xvfb plus Mesa's software Vulkan
driver; `xdotool` and ImageMagick drive and capture it):

```bash
sudo apt-get install xvfb mesa-vulkan-drivers xdotool imagemagick
```

```bash
Xvfb :99 -screen 0 1440x900x24 &
```

```bash
DISPLAY=:99 cargo run -p aerocosm-viewer &
```

```bash
DISPLAY=:99 import -window root viewer.png
```

`DISPLAY=:99 xdotool mousemove 596 583 click 4` zooms in at a point; `click 1` selects.
Under Xvfb each wheel click arrives as two events, so zoom steps are twice as large as on a
real mouse.

## Workspace

| Crate | Role |
|-------|------|
| `aerocosm-core` | Deterministic simulation: time, units, geodesy, random streams, world model, aircraft catalog |
| `aerocosm-basemap` | Basemap file types shared by the importer and the viewer |
| `aerocosm-data` | Offline import of OurAirports and Natural Earth into `assets/` |
| `aerocosm` | Headless command-line runner |
| `aerocosm-viewer` | Bevy viewer, desktop and browser |
| `xtask` | Project tooling: `cargo xtask web build`, `cargo xtask web serve` |

Data: [OurAirports](https://ourairports.com/) (public domain) · Made with
[Natural Earth](https://www.naturalearthdata.com/) · [Inter](https://github.com/rsms/inter)
typeface (SIL OFL 1.1).
