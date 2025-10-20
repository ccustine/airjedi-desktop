// Copyright 2025 Chris Custine
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use walkers::sources::{Attribution, TileSource};
use walkers::TileId;

/// Tile source for Carto CDN dark basemap tiles
/// Uses subdomain load balancing across a-d.basemaps.cartocdn.com
pub struct CartoTileSource;

impl TileSource for CartoTileSource {
    fn tile_url(&self, tile_id: TileId) -> String {
        // Subdomain load balancing (a, b, c, d) based on tile coordinates
        let subdomain = ['a', 'b', 'c', 'd'][((tile_id.x + tile_id.y) % 4) as usize];

        format!(
            "https://{}.basemaps.cartocdn.com/dark_all/{}/{}/{}.png",
            subdomain, tile_id.zoom, tile_id.x, tile_id.y
        )
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            text: "© OpenStreetMap contributors, © CARTO",
            url: "https://carto.com/attributions",
            logo_light: None,
            logo_dark: None,
        }
    }

    // Use default implementations for tile_size() and max_zoom()
    // tile_size() defaults to 256px
    // max_zoom() defaults to appropriate level for the source
}
