| `assets/ground/stand-sizes.json` | Hand-curated: stand size limits (ICAO codes) where the OSM spacing misjudges them, at EDDK; see its `source` field | — | — |
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
| Stands, taxiways | OpenStreetMap + `stand-sizes.json` / `stand-areas.json` curation | Navigraph gates and airport data, where available |
| Departure and arrival procedures | Simplified procedures (plan M3 D59) | Navigraph SIDs, STARs, approaches |
| Airways, waypoints, airspace | — (direct routes) | Navigraph |

| Asset | Source | Licence | Attribution |
|-------|--------|---------|-------------|
| `assets/world/germany.world.json` | [OurAirports](https://ourairports.com/data/) `airports.csv`, `runways.csv` | Public domain | Not required; credited here |
| `assets/basemap/germany.basemap.json` | [Natural Earth](https://www.naturalearthdata.com/) 1:10m vector layers (`GeoJSON` from [nvkelso/natural-earth-vector](https://github.com/nvkelso/natural-earth-vector)) | Public domain | "Made with Natural Earth" (requested, not required) |
| `assets/aircraft/types.json` | Hand-curated; see each entry's `source` field (wingspans: manufacturers' airport-planning documents) | — | — |
| `assets/airlines/airlines.json` | Hand-curated from carriers' published bases and fleets (2025); see each entry's `source` field | — | — |
| `assets/traffic/germany.json` | [ADV-Monatsstatistik 12/2025](https://www.adv.aero/wp-content/uploads/2015/11/12.2025-ADV-Monatsstatistik.pdf) (movements), [DFS 2025 traffic](https://www.dfs.de/homepage/en/media/press/2026/05-01-2026-air-traffic-in-germany-2025-more-flights-good-punctuality-level/), [EUROCONTROL European Aviation Overview 2025](https://www.eurocontrol.int/sites/default/files/2026-01/eurocontrol-european-aviation-overview-20260123-2025-review.pdf) (overflights); capacities and runway modes: [Frankfurt noise commission 2025](https://www.flk-frankfurt.de/eigene_dateien/aktuell/2025-aktuelles/juni/top_3_-_flk-beschluss_zum_entwurf_eines_geaenderten_betriebskonzepts_fuer_den_flughafen_frankfurt__26.6.2025.pdf), [Lärmaktionsplan Flughafen München 2021](https://www.regierung.oberbayern.bayern.de/mam/dokumente/bereich5/pfb/laermaktionsplanung/lap_fh-muc/anlage_11.pdf), [airliners.de on hourly coordination parameters](https://www.airliners.de/stundeneckwert-motor-hub-flughaefen-apropos-10/38723), [Düsseldorf operating rules](https://www.dus.com/de-de/konzern/nachbarn/transparenz/flugbetrieb/betriebszeiten), [City of Frankfurt noise monitoring](https://frankfurt.de/fluglaermschutz/-/media/frankfurtde/microsites/microsite-fluglaermschutz/pdf/laerm-monitoring-2024/fluglaerm-q3_2024.pdf); estimates labelled in the file's `source` field | Statistics cited, figures transcribed | Sources cited here |
| `assets/ground/germany.ground.json` | [OpenStreetMap](https://www.openstreetmap.org/) via the [Overpass API](https://wiki.openstreetmap.org/wiki/Overpass_API): aeroway taxiways, taxilanes, runways, parking positions, holding positions, aprons, terminals and aerodrome outlines around each German target airport | [ODbL 1.0](https://opendatacommons.org/licenses/odbl/1-0/) (`assets/ground/LICENSE.txt`) | "© OpenStreetMap contributors", required wherever the layouts are shown |
| `assets/ground/stand-areas.json` | Hand-curated: terminal use by airline at FRA and MUC; see its `source` field | — | — |
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

Downloaded 2026-11-06 from the Overpass mirror at `maps.mail.ru` (OSM database state of
2026-11-06), with the queries written by `aerocosm-data queries`.

Import result: 22 of 24 airports pass the quality gate (1,857 stands). EDDR (no stands mapped)
and EDLW (only 5 unnamed stand nodes) keep straight-line ground. Intermediate holding positions
(`holding_position:type=intermediate`, on nodes or as lines across a taxiway) are kept since
plan M4 D89: e.g. FRA 168, MUC 206, BER 177, LEJ 119 (same snapshot, re-imported 2026-11-12).

| File | SHA-256 |
|------|---------|
| `EDDB.json` | `554d396c5a3e5232406f6361b4ec1ec339bda3e92619065dff62611d46dd5138` |
| `EDDC.json` | `3ae87dfc3f08d4e9c3a31052460b5573463136b344a6ec1e4fcba40ead41be4f` |
| `EDDE.json` | `9c39c65ad5a6939df6692c8cc63cbd5d784b436c5035a3f0c49a8bfe6d02c320` |
| `EDDF.json` | `7a4263ddc67b908af9f63dba0e1b22720356531b71f874d8b30e14d82a2f4877` |
| `EDDG.json` | `9adb8229e7063502f60702e74218d4bfaea49eb9cf2686ae2a7602c30b9a9c75` |
| `EDDH.json` | `b519f3abc055807c116c85a6b260ce10b6959764fe6c1bd23a884f60d42b7e8d` |
| `EDDK.json` | `7a7758638c1ea92ce3ff0a4476aaf2da394d6aaccb79ddd14c4c05f29b6d80ed` |
| `EDDL.json` | `c439c1cf134ba9658eaa43487ad9914d14ef9c511c86f9ba3527c991bc40d85d` |
| `EDDM.json` | `b28f2e3414b01af4d05ad80bb9a3b199d14b0207dce01465ab50ea879a810846` |
| `EDDN.json` | `46624466140aebd4b0d184cd76365b12cd0553af50c6df589ee2238430419d70` |
| `EDDP.json` | `ec037ceee543ef9e47d0eee5691945fbc85f498e05744d9aa7f321fe0ca620a3` |
| `EDDR.json` | `90cec782a9b5c8078e3463d68ce4ae6980c706c918b446e8b2a068ea7b41f0fc` |
| `EDDS.json` | `318ae9b3279a6aea5ca901cf8a36aaef919acb365f9ad4d19d38c48a7ec9c892` |
| `EDDV.json` | `7e100ad967130ec13a5699eb2b74526aeb773233823f72815d7b7d0fecc67b8e` |
| `EDDW.json` | `804583b0a6a2525096a7f1ea3d32bfbb0dbf101a3ad0d299f883142c054b81e7` |
| `EDFH.json` | `c87c399074d1675d720a0fb7b73d799bcf4d0117ea539eaa0630eba4824a46bf` |
| `EDJA.json` | `1a308aeb5a1007c00ae41f786b5c9f1d0418ffda57b6d4d1ee25610b06c77837` |
| `EDLP.json` | `6ded50e319734e0da49fdbcbe800c24056d797dedf467afbd45c946b3d0b8e95` |
| `EDLV.json` | `f746f4c83348fc55cd95bb48e1f2c3cda94f916f3f05e297d47109851aec8d2e` |
| `EDLW.json` | `6778a7c2b5cfdda1ec4aa4b3e89df05d7b9a01768f36d590cc46fae8dc09f3c7` |
| `EDNY.json` | `1eb9483d02c08bbdcfe6065b3734dc68bb679f9051557e1ab4637a04fb01954f` |
| `EDSB.json` | `c5316bf8280c185d203086cb9d0730a9d82c658c495d93a3af7bb4d0735529db` |
| `EDVE.json` | `ea5a6f21d7fc0f697102a4fe173e31aeb54ccf3b22b19932919127bc19bc7de6` |
| `EDXW.json` | `6a54d403446d467eeb3b953f59d67ab359347e3b9342c758c296b1923903482a` |

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

The ground file is regenerated separately from the world and basemap, from one Overpass JSON
download per airport. Write the queries (one `IDENT<TAB>query` line per German airport with
movement targets) and fetch them from any Overpass instance:

```bash
mkdir -p /tmp/aerocosm-data/osm
```

```bash
cargo run -p aerocosm-data -- queries --out /tmp/aerocosm-data/osm-queries.tsv
```

```bash
while IFS=$'\t' read -r icao query; do curl -sSf --data-urlencode "data=$query" -o /tmp/aerocosm-data/osm/$icao.json https://overpass-api.de/api/interpreter; sleep 10; done < /tmp/aerocosm-data/osm-queries.tsv
```

A response with a `remark` (the server gave up) is rejected by the importer; fetch that airport
again. Then import, optionally writing layouts that fail the quality gate for inspection:

```bash
cargo run --release -p aerocosm-data -- ground --osm /tmp/aerocosm-data/osm --rejected /tmp/aerocosm-data/rejected
```

Update the ground snapshot section above.

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
| NASA POWER responses, 3,846 files `<IDENT>.json` | SHA-256 of their contents concatenated in name order `61bf794d7f0a3bcbc74d5449d937f4132fb988f3afebd1cdb1c65b08d85910d5` |
| `flightlist_2019*.csv.gz`, twelve months | MD5 as published on Zenodo (the record lists them) |

Import result: 26,109 aerodromes with a usable runway, 3,846 with scheduled service; time
zones for 5,591 airports (5,333 by code, 258 from the nearest listed airport in the
country, 1 without); prevailing winds for all 3,846 scheduled airports. `world.bin`
2.8 MB, `basemap/world.bin` 3.8 MB. World format 4 adds 234 countries with population and
GDP (the countries with a valid ISO code and both figures).

Observed traffic: of 30,989,481 flights in the 2019 lists, 11,599,572 are counted with both
ends at known airports (5,183 airports); dropped: 3,582,212 without a known airport,
1,329,637 to the same airport, 323 faster than any airliner. 489 airlines with at least 365
departures and an OpenFlights name; 1,673 smaller callsign prefixes left out. Coverage (the
share of departures seen with both ends) varies: Germany 0.57, United States 0.46,
Australia 0.50, India 0.31, Brazil 0.26, Japan 0.27, China 0.07. `traffic/observed.bin`
219 KB.

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
cargo run --release -p aerocosm-data -- observed --airlines /tmp/aerocosm-data/globe/airlines.dat /tmp/aerocosm-data/globe/flightlist_2019*.csv.gz
```

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
    - Where that spacing misjudges an apron, `assets/ground/stand-sizes.json` bounds the code:
      the rule with the longest stand-name prefix raises it to `at_least` and caps it at
      `at_most`. EDDK follows the VATSIM Germany knowledge base (unofficial; cargo aprons E/F
      code E, its A380 positions code F, feeder apron W code C–D, V at most D) and an estimate of
      code C for the terminal piers A–D, whose OSM stands sit too close for the A320s using them.
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
