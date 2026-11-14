# Data sources

The generated assets are committed; raw downloads are not. Regenerate them with the steps
below; the importer output is deterministic, so unchanged inputs give byte-identical files.

## Data tiers

Data comes in two tiers (see `CLAUDE.md`, "Data tiers"): the **baseline** listed below is open,
committed and used by all tests; a **premium** tier of subscription data (planned: Navigraph,
through each user's own subscription) is fetched and cached locally, never committed, and
overrides the baseline where it has data. Every feature works on the baseline alone.

| Data | Baseline (committed) | Premium (planned, local only) |
|------|----------------------|-------------------------------|
| Airports, runways | OurAirports | Navigraph (AIRAC) |
| Stands, taxiways | OpenStreetMap, stand sizes from their spacing, generic stand choice (plan M5 D119, D120) | Navigraph gates and airport data, where available |
| Departure and arrival procedures | Simplified procedures (plan M3 D59) | Navigraph SIDs, STARs, approaches |
| Airways, waypoints, airspace | — (direct routes) | Navigraph |

| Asset | Source | Licence | Attribution |
|-------|--------|---------|-------------|
| `assets/world/germany.world.json` | [OurAirports](https://ourairports.com/data/) `airports.csv`, `runways.csv` | Public domain | Not required; credited here |
| `assets/basemap/germany.basemap.json` | [Natural Earth](https://www.naturalearthdata.com/) 1:10m vector layers (`GeoJSON` from [nvkelso/natural-earth-vector](https://github.com/nvkelso/natural-earth-vector)) | Public domain | "Made with Natural Earth" (requested, not required) |
| `assets/aircraft/types.json` | Hand-curated; see each entry's `source` field (wingspans: manufacturers' airport-planning documents). 37 types; aliases map close variants (ICAO designators) to them, so that 97 % of the typed flights in the 2019 OpenSky lists fly as a catalog type (plan M5 D111) | — | — |
| `assets/ground/<letter>.ground.bin` | [OpenStreetMap](https://www.openstreetmap.org/) planet file (or the [Overpass API](https://wiki.openstreetmap.org/wiki/Overpass_API) for single airports): aeroway taxiways, taxilanes, runways, parking positions, holding positions, aprons, terminals and aerodrome outlines around every scheduled airport, by ICAO region (plan M5 D116, D117) | [ODbL 1.0](https://opendatacommons.org/licenses/odbl/1-0/) (`assets/ground/LICENSE.txt`) | "© OpenStreetMap contributors", required wherever the layouts are shown |
| `assets/fonts/Inter-*.ttf` | [Inter 4.1](https://github.com/rsms/inter) (`extras/ttf`), Regular and SemiBold | SIL Open Font License 1.1 (`assets/fonts/Inter-LICENSE.txt`) | Licence file shipped alongside |

## Current snapshot

Downloaded 2026-11-05.

- OurAirports: nightly export of 2026-11-05.
- Natural Earth: `natural-earth-vector` master at `ca96624a56bd078437bca8184e78163e5039ad19`
  (VERSION `5.2.0-pre`).

| File | SHA-256 |
|------|---------|
| `airports.csv` | `55bd82106c8ac1c9559cf5879ae9b69eb9c6e748df01514632dd5fbbb9e8b474` |
| `runways.csv` | `d94a2e562069ce30e0290649164bf608c437696ccf1fad988c5cba7d25fb0c30` |
| `ne_10m_admin_0_boundary_lines_land.geojson` | `74d9c16229c095fde65943a9919e337682f044bcebccb120764f38edf3b70f4a` |
| `ne_10m_admin_0_countries.geojson` | `239eec57ac17f100a11e2536cffc56752c318b50ae765b0918ff7aab4ce8f255` |
| `ne_10m_admin_1_states_provinces_lines.geojson` | `1a1f30ccaaf4cc9c4bde34266f0b8cbb955d3a4cf254b756912255f2ec7c75b6` |
| `ne_10m_lakes.geojson` | `2d036f53dedec578001c5c30c2959ee7d4eebc1306900fa4367c49929ec8f2d9` |
| `ne_10m_populated_places.geojson` | `9b8e3de09048ef00dfc70357dbb9fa324493f214b5e0ae4daf1aa79a8d10116b` |
| `ne_10m_rivers_lake_centerlines.geojson` | `bb854a900ecbd3b408df46d5e16e3e0f974ba55993f9d8b5c26e855273c0905a` |

Import result: 452 aerodromes (429 German, 23 edge), 582 runways (357 surveyed, 225
approximate); catchment populations for every Major, Regional and Edge airport.

### Ground snapshot

The OSM planet file `planet-261109.osm.pbf` (95.8 GB, OSM database state of 2026-11-09),
streamed on 2026-11-13 through `osmium extract` with the boxes of the 3,908 scheduled airports
(plan M5 D116); the aeroway extract `aeroways.opl` (392 MB) has SHA-256
`14ce87f37d6c5dcdf6848a4c3a3b5db54bde7b1ab5b2092e436e51cc9480f86d`.

Import result: 836 of 3,908 scheduled airports pass the quality gate (D118), 15 MB of region
assets. Holding positions are placed by rule where none are mapped for a departure end in use
(D126). Of the 3,072 rejected, 2,925 map fewer stands than their traffic needs (most small
airports map none), 147 have no holding position reachable from a stand for a runway end in
use (e.g. Schiphol's 18R). By region (passed / rejected):

| Region | Passed | Rejected | Region | Passed | Rejected |
|--------|--------|----------|--------|--------|----------|
| A | 0 | 54 | N | 8 | 102 |
| B | 1 | 22 | O | 15 | 125 |
| C | 18 | 213 | P | 11 | 172 |
| D | 28 | 51 | Q | 0 | 1 |
| E | 144 | 116 | R | 25 | 136 |
| F | 33 | 109 | S | 67 | 274 |
| G | 25 | 29 | T | 10 | 36 |
| H | 23 | 74 | U | 30 | 177 |
| I | 0 | 8 | V | 30 | 217 |
| K | 106 | 336 | W | 16 | 139 |
| L | 170 | 143 | Y | 19 | 140 |
| M | 14 | 154 | Z | 43 | 226 |
| other | 0 | 18 | | | |

German hubs: FRA 284 stands and 104 holding positions, MUC 189 and 27, BER 186 and 36, CGN
186 and 29, LEJ 168 and 26.

## Regenerating

Download into a scratch directory outside the repository:

```bash
mkdir -p /tmp/aerocosm-data/ourairports /tmp/aerocosm-data/natural-earth
```

```bash
for f in airports runways; do curl -sSfo /tmp/aerocosm-data/ourairports/$f.csv https://davidmegginson.github.io/ourairports-data/$f.csv; done
```

```bash
for l in admin_0_countries admin_0_boundary_lines_land admin_1_states_provinces_lines lakes rivers_lake_centerlines populated_places; do curl -sSfo /tmp/aerocosm-data/natural-earth/ne_10m_$l.geojson https://raw.githubusercontent.com/nvkelso/natural-earth-vector/master/geojson/ne_10m_$l.geojson; done
```

Run the importer from the repository root:

```bash
cargo run --release -p aerocosm-data -- import --ourairports /tmp/aerocosm-data/ourairports --natural-earth /tmp/aerocosm-data/natural-earth --out assets
```

Then run the full check: the asset guard tests in `aerocosm-core` and `aerocosm-basemap`
load the shipped files. Update the snapshot section above.

### Airport ground layouts (OpenStreetMap)

The ground assets are regenerated separately from the world and basemap (plan M5 D116), from
the OSM planet file, which is streamed once and never stored: `osmium` (the `osmium-tool`
package) keeps everything inside the airports' query boxes, then the aeroways with the nodes
they use, written as OPL text. Write the boxes (and the Overpass queries, for single
airports):

```bash
mkdir -p /tmp/aerocosm-data/planet
```

```bash
cargo run --release -p aerocosm-data -- queries --out /tmp/aerocosm-data/osm-queries.tsv --boxes /tmp/aerocosm-data/planet/boxes.geojson
```

Stream the planet through `osmium extract` (one pass, about 96 GB; a single connection is
slow, so fetching it in parallel byte ranges and feeding them in order helps, as long as the
bytes reach `osmium` in order):

```bash
curl -sSfL https://planet.openstreetmap.org/pbf/planet-latest.osm.pbf | osmium extract -s simple -p /tmp/aerocosm-data/planet/boxes.geojson -F pbf - -o /tmp/aerocosm-data/planet/boxes.osm.pbf
```

```bash
osmium tags-filter /tmp/aerocosm-data/planet/boxes.osm.pbf nw/aeroway -o /tmp/aerocosm-data/planet/aeroways.osm.pbf
```

```bash
osmium cat /tmp/aerocosm-data/planet/aeroways.osm.pbf -f opl -o /tmp/aerocosm-data/planet/aeroways.opl
```

Then import, writing one asset per ICAO region (`assets/ground/<letter>.ground.bin`; local
codes that start with a digit go to `other.ground.bin`), optionally with the layouts that fail
the quality gate for inspection:

```bash
cargo run --release -p aerocosm-data -- ground --opl /tmp/aerocosm-data/planet/aeroways.opl --rejected /tmp/aerocosm-data/rejected
```

A single airport can still come from Overpass: fetch its line of `osm-queries.tsv` from any
Overpass instance into `<dir>/<IDENT>.json` and run `ground --osm <dir>` (a response with a
`remark`, where the server gave up, is rejected). Update the ground snapshot section above.

## The globe (plan M5)

The globe's assets are binary (plan M5 D104): a header naming the kind and format
version, then postcard-encoded data compressed with zstd.

| Asset | Sources | Licence | Attribution |
|-------|---------|---------|-------------|
| `assets/world/world.bin` | OurAirports `airports.csv`, `runways.csv`; Natural Earth populated places (catchments) and admin-0 countries (`POP_EST`, `GDP_MD`: population and GDP per country, plan M5 D109); [`airportsdata`](https://github.com/mborsetti/airportsdata) `airports.csv` (time zones, from `TimeZoneDB`); [NASA POWER](https://power.larc.nasa.gov/) climatology `WD10M` (prevailing wind) | Public domain; public domain; MIT; CC BY 4.0 | "Wind data: NASA Langley Research Center POWER Project"; "Time zones: airportsdata (MIT), TimeZoneDB" |
| `assets/basemap/world.bin` | Natural Earth 1:10m, as the German basemap | Public domain | Not required ("Made with Natural Earth" shown) |
| `assets/traffic/observed.bin` | OpenSky Network flight lists 2019 ([Zenodo record 5764017](https://zenodo.org/records/5764017)); [OpenFlights](https://openflights.org/data.html) `airlines.dat` (names, callsigns) | CC BY 4.0; ODbL | "Flight data: The OpenSky Network, https://opensky-network.org"; "Airline names: OpenFlights (ODbL)" |

Runway headings synthesised from designators use the offset learnt from the 50 nearest
runway ends with surveyed true headings (plan M5 D101), so no magnetic model is needed.

### Globe snapshot

Downloaded 2026-11-13 (Natural Earth: the 2026-11-05 snapshot above).

| File | Checksum |
|------|----------|
| `airports.csv` (OurAirports) | SHA-256 `a3bbdc1d7321c19e7a3ee6ddc7b570850fdfa56f6357c30bae9600cef08637be` |
| `runways.csv` (OurAirports) | SHA-256 `e1026851f59e3714c2074432e469aeb9c3b83908315cfa547f353ae607f74a1d` |
| `airportsdata.csv` (`airportsdata/airports.csv`, main) | SHA-256 `b218baf3b864af711bfc5c2d6de38f4cdfde873e0c60289fd9c5cc22afe57876` |
| `airlines.dat` (OpenFlights, master) | SHA-256 `39be1a432e8b04ebc12860c29281c974a9cb52169c82b2456a835d66ab1548a1` |
| NASA POWER responses, 3,909 files `<IDENT>.json` | SHA-256 of their contents concatenated in name order (`LC_ALL=C`) `71b296a45dccfb2fef1e13a40974b1a5264a879a1a19390367cdbcc49258c304` |
| `flightlist_2019*.csv.gz`, twelve months | MD5 as published on Zenodo (the record lists them) |

Import result: 28,204 aerodromes with a usable runway, 3,909 with scheduled service; time
zones for 5,686 airports (5,422 by code, 263 from the nearest listed airport in the
country, 1 without); prevailing winds for all 3,909 scheduled airports. `world.bin`
3.0 MB, `basemap/world.bin` 3.8 MB. World format 4 adds 234 countries with population and
GDP (the countries with a valid ISO code and both figures). Runway designators written
without a leading zero (`6L`, American style) are read as `06L` since M5 PR 2; before, 2,269
such runways were skipped, Los Angeles among their airports.

Observed traffic (format 2, M5 PR 2): of 30,989,481 flights in the 2019 lists, 12,402,764
are counted with both ends at known airports (5,402 airports), flights at airports closed
since counted at the nearest open one with scheduled service within 50 km; dropped:
3,250,827 without a known airport, 1,349,320 to the same airport, 373 faster than any
airliner. 497 airlines with at least 365 departures and an OpenFlights name; 1,683 smaller
callsign prefixes left out. Coverage (the share of departures seen with both ends) varies:
Germany 0.64, United States 0.50, Australia 0.51, India 0.31, Brazil 0.29, Japan 0.27, China
0.07. `traffic/observed.bin` 236 KB.

### Regenerating the globe

Download into a scratch directory outside the repository (Natural Earth as above):

```bash
mkdir -p /tmp/aerocosm-data/globe/winds
```

```bash
for f in airports runways; do curl -sSfo /tmp/aerocosm-data/globe/$f.csv https://davidmegginson.github.io/ourairports-data/$f.csv; done
```

```bash
curl -sSfo /tmp/aerocosm-data/globe/airportsdata.csv https://raw.githubusercontent.com/mborsetti/airportsdata/main/airportsdata/airports.csv
```

```bash
curl -sSfo /tmp/aerocosm-data/globe/airlines.dat https://raw.githubusercontent.com/jpatokal/openflights/master/data/airlines.dat
```

Import once without winds, list the wind requests, download them (one small JSON per
scheduled airport; be gentle with NASA's servers), then import again with them:

```bash
cargo run --release -p aerocosm-data -- globe --ourairports /tmp/aerocosm-data/globe --natural-earth /tmp/aerocosm-data/natural-earth --zones /tmp/aerocosm-data/globe/airportsdata.csv --out assets
```

```bash
cargo run --release -p aerocosm-data -- wind-requests --out /tmp/aerocosm-data/globe/wind-requests.tsv
```

```bash
cd /tmp/aerocosm-data/globe && xargs -P 2 -L 1 sh -c 'curl -sSf --retry 3 -o "winds/$0.json" "$1"' < wind-requests.tsv
```

```bash
cargo run --release -p aerocosm-data -- globe --ourairports /tmp/aerocosm-data/globe --natural-earth /tmp/aerocosm-data/natural-earth --zones /tmp/aerocosm-data/globe/airportsdata.csv --winds /tmp/aerocosm-data/globe/winds --out assets
```

Then aggregate the twelve 2019 flight lists from Zenodo (2.7 GB compressed; they can be
streamed through named pipes rather than stored):

```bash
cargo run --release -p aerocosm-data -- observed --airlines /tmp/aerocosm-data/globe/airlines.dat --airports /tmp/aerocosm-data/globe/airports.csv /tmp/aerocosm-data/globe/flightlist_2019*.csv.gz
```

## The world model (plan M5 PR 2)

Movement targets, airlines, runway use and banks are no longer files: the core derives them
at start-up from `world.bin`, `observed.bin` and `types.json` by general rules
(`crates/aerocosm-core/src/traffic/model/`, plan M5 D109–D113). The German statistics are
validation only: `ADV-Monatsstatistik 12/2019`
([PDF](https://www.adv.aero/wp-content/uploads/2016/02/12.2019-ADV-Monatsstatistik.pdf)),
scheduled and charter movements per airport, are constants in the tests (plan M5 D114).

- **Closed airports:** the flight lists name airports closed since (Tegel). When aggregating,
  their flights count at the nearest open airport with scheduled service within 50 km; a
  closed airport's old ICAO code comes from its OurAirports keywords.
- **Coverage per airport:** each observed airport also records its flights whose other end was
  not seen (`partial`, observed format 2).

## Processing notes

- **Edge airports (M2, plan D29):** 23 foreign airports inside the basemap, listed in
  `crates/aerocosm-data/src/ourairports/mod.rs`, imported with their runways as `edge`.
- **Catchment (plan D37):** the sum of `POP_MAX` of all Natural Earth populated places within
  100 km of each Major, Regional and Edge airport (unclipped, so airports near the map edge
  count places beyond it).

- **Aerodromes:** large, medium and small airports in Germany with at least one usable runway.
  The ICAO code is the ident where one exists (plan D22).
- **Runways:** surveyed threshold coordinates are kept when consistent with the declared length
  and reciprocal headings. Otherwise geometry is synthesised along the data heading, or the
  designator plus 4° magnetic declination, and flagged `approximate` (plan D18). Helipads and
  glider strips with non-numeric designators are skipped.
- **Basemap:** clipped to 8° W–30° E, 41°–61° N (wide enough that the viewer never shows the map edge), simplified at 0.002° (~200 m), and
  rounded to 5 decimals (~1 m).
- **Ground layouts (plan M4 D64–D68):**
  - Ways tagged `aeroway=taxiway|taxilane|runway|parking_position` with a node inside the
    aerodrome outline are split into edges at shared nodes, way ends, holding positions and stand
    nodes. Interior points are simplified to 0.5 m and coordinates rounded to 7 decimals. `oneway`
    and `maxspan` are kept.
  - Stands:
    - The loose end of a `parking_position` line, connected to the nearest taxiway or taxilane
      within 60 m if neither end is joined.
    - The middle of a line joined at both ends (drive-through).
    - A `parking_position` node, connected the same way.
    - Names come from `ref` or `name`, else from a named stop node on the line. Unnamed stands
      are left out; repeated names get a `-2` suffix.
    - A `parking_position` node ending a lead-in of the same name is that stand, not another.
    - The wingspan limit is the largest ICAO aerodrome code (A–F, Annex 14) whose parked
      footprint keeps the wingtip clearance to every stand that is not a variant of the same
      stand (`05A`, `05B` of `05`), with 5 % allowance for mapping error. The footprint is a
      square as long as the code's wingspan reaching back from the stand point, so stands side
      by side are judged by their spacing and rows parked nose to nose by the gap between them.
    - No curated bounds (plan M5 D119): where the spacing misjudges an apron, the code stands.
      EDDP follows the same knowledge base's per-bay wingspan limits for the DHL hub apron 4
      (bays A–E, stands 401–487) and an estimate of code C for the airline stands of apron 1.
  - Holding positions: `holding_position` nodes on the network without a type or of type
    `runway`, within 300 m of a runway centreline, assigned to the nearer threshold.
  - Exits: network nodes within 25 m of a runway centreline where a taxiway or taxilane leaves,
    measured 60 m along it, at most 120° off the landing direction.
  - The network is pruned to the part connected to a runway.
  - Stands without a taxi route (respecting one-way edges) to the entry of every departure runway
    in use, or from an exit of every arrival runway in use, are left out (`unroutable` in the
    import report); the simulation could not use them.
  - Quality gate (D66): at least one stand per 10 daily movements. Every runway end in use with
    the prevailing wind (fixed at 250° until weather arrives) needs a holding position reachable
    from a stand, and an exit from which a stand is reachable, respecting one-way edges. Airports
    that fail keep straight-line ground.
