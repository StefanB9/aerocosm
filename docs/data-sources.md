# Data sources

The generated assets are committed; raw downloads are not. Regenerate them with the steps
below; the importer output is deterministic, so unchanged inputs give byte-identical files.

| Asset | Source | Licence | Attribution |
|-------|--------|---------|-------------|
| `assets/world/germany.world.json` | [OurAirports](https://ourairports.com/data/) `airports.csv`, `runways.csv` | Public domain | Not required; credited here |
| `assets/basemap/germany.basemap.json` | [Natural Earth](https://www.naturalearthdata.com/) 1:10m vector layers (`GeoJSON` from [nvkelso/natural-earth-vector](https://github.com/nvkelso/natural-earth-vector)) | Public domain | "Made with Natural Earth" (requested, not required) |
| `assets/aircraft/types.json` | Hand-curated; see each entry's `source` field | — | — |
| `assets/airlines/airlines.json` | Hand-curated from carriers' published bases and fleets (2025); see each entry's `source` field | — | — |
| `assets/traffic/germany.json` | [ADV-Monatsstatistik 12/2025](https://www.adv.aero/wp-content/uploads/2015/11/12.2025-ADV-Monatsstatistik.pdf) (movements), [DFS 2025 traffic](https://www.dfs.de/homepage/en/media/press/2026/05-01-2026-air-traffic-in-germany-2025-more-flights-good-punctuality-level/), [EUROCONTROL European Aviation Overview 2025](https://www.eurocontrol.int/sites/default/files/2026-01/eurocontrol-european-aviation-overview-20260123-2025-review.pdf); estimates labelled in the file's `source` field | Statistics cited, figures transcribed | Sources cited here |
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
