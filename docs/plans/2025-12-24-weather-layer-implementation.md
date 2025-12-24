# Weather Layer Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add optional OpenWeatherMap weather tile overlays (precipitation, clouds, wind) to the map with configurable opacity and Settings UI.

**Architecture:** Weather tiles render as semi-transparent PNG overlays between the Carto basemap and aviation data. Each layer type (precipitation, clouds, wind) has its own toggle and uses the `walkers` crate's `HttpTiles` for fetching/caching. API key resolution checks environment variable first, then config file.

**Tech Stack:** Rust, egui, walkers crate, OpenWeatherMap Tile API, confy (config persistence)

---

## Task 1: Add Weather Config Fields

**Files:**
- Modify: `src/config.rs:79-132` (AppConfig struct)

**Step 1: Add weather fields to AppConfig**

Add these fields after `test_video_url` in the `AppConfig` struct:

```rust
    /// Show precipitation radar overlay
    #[serde(default)]
    pub show_weather_precipitation: bool,

    /// Show cloud coverage overlay
    #[serde(default)]
    pub show_weather_clouds: bool,

    /// Show wind speed overlay
    #[serde(default)]
    pub show_weather_wind: bool,

    /// Weather layer opacity (0.0 - 1.0)
    #[serde(default = "default_weather_opacity")]
    pub weather_opacity: f32,

    /// OpenWeatherMap API key (optional, env var takes precedence)
    #[serde(default)]
    pub openweathermap_api_key: Option<String>,
```

**Step 2: Add default function**

Add after `default_test_video_url()`:

```rust
fn default_weather_opacity() -> f32 {
    0.6
}
```

**Step 3: Update Default impl**

Add to the `Default` impl for `AppConfig`:

```rust
            show_weather_precipitation: false,
            show_weather_clouds: false,
            show_weather_wind: false,
            weather_opacity: default_weather_opacity(),
            openweathermap_api_key: None,
```

**Step 4: Build to verify**

Run: `cargo build`
Expected: Compiles without errors

**Step 5: Commit**

```bash
git add src/config.rs
git commit -m "feat(config): add weather layer settings fields"
```

---

## Task 2: Create Weather Module Structure

**Files:**
- Create: `src/weather/mod.rs`
- Create: `src/weather/openweathermap.rs`
- Modify: `src/main.rs:21-31` (add mod declaration)

**Step 1: Create weather module file**

Create `src/weather/mod.rs`:

```rust
//! Weather overlay tile management.
//!
//! This module provides weather tile fetching from OpenWeatherMap
//! with support for precipitation, cloud, and wind layers.

pub mod openweathermap;

pub use openweathermap::{WeatherLayer, OpenWeatherMapSource, WeatherTiles};
```

**Step 2: Create OpenWeatherMap tile source**

Create `src/weather/openweathermap.rs`:

```rust
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

//! OpenWeatherMap tile source implementation.

use walkers::sources::{Attribution, TileSource};
use walkers::{HttpOptions, HttpTiles, TileId};
use eframe::egui;

/// Available weather layer types from OpenWeatherMap
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatherLayer {
    Precipitation,
    Clouds,
    Wind,
}

impl WeatherLayer {
    /// Get the OpenWeatherMap layer name for URL construction
    pub fn as_str(&self) -> &'static str {
        match self {
            WeatherLayer::Precipitation => "precipitation_new",
            WeatherLayer::Clouds => "clouds_new",
            WeatherLayer::Wind => "wind_new",
        }
    }

    /// Get human-readable display name
    pub fn display_name(&self) -> &'static str {
        match self {
            WeatherLayer::Precipitation => "Precipitation",
            WeatherLayer::Clouds => "Clouds",
            WeatherLayer::Wind => "Wind",
        }
    }
}

/// Tile source for OpenWeatherMap weather layers
pub struct OpenWeatherMapSource {
    layer: WeatherLayer,
    api_key: String,
}

impl OpenWeatherMapSource {
    /// Create a new OpenWeatherMap tile source for the specified layer
    pub fn new(layer: WeatherLayer, api_key: String) -> Self {
        Self { layer, api_key }
    }
}

impl TileSource for OpenWeatherMapSource {
    fn tile_url(&self, tile_id: TileId) -> String {
        format!(
            "https://tile.openweathermap.org/map/{}/{}/{}/{}.png?appid={}",
            self.layer.as_str(),
            tile_id.zoom,
            tile_id.x,
            tile_id.y,
            self.api_key
        )
    }

    fn attribution(&self) -> Attribution {
        Attribution {
            text: "Weather data © OpenWeatherMap",
            url: "https://openweathermap.org/",
            logo_light: None,
            logo_dark: None,
        }
    }
}

/// Manager for multiple weather tile layers
pub struct WeatherTiles {
    pub precipitation: Option<HttpTiles>,
    pub clouds: Option<HttpTiles>,
    pub wind: Option<HttpTiles>,
    api_key: Option<String>,
}

impl WeatherTiles {
    /// Create a new weather tiles manager
    pub fn new() -> Self {
        Self {
            precipitation: None,
            clouds: None,
            wind: None,
            api_key: None,
        }
    }

    /// Resolve API key from environment variable or config
    pub fn resolve_api_key(config_key: Option<&str>) -> Option<String> {
        // Check environment variable first
        if let Ok(key) = std::env::var("OPENWEATHERMAP_API_KEY") {
            if !key.is_empty() {
                return Some(key);
            }
        }

        // Fall back to config
        config_key.map(|s| s.to_string()).filter(|s| !s.is_empty())
    }

    /// Check if an API key is available
    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }

    /// Get the source of the API key for UI display
    pub fn api_key_source(&self) -> Option<&'static str> {
        if std::env::var("OPENWEATHERMAP_API_KEY").map(|k| !k.is_empty()).unwrap_or(false) {
            Some("environment variable")
        } else if self.api_key.is_some() {
            Some("config file")
        } else {
            None
        }
    }

    /// Initialize or update the API key and create tile sources
    pub fn set_api_key(&mut self, api_key: Option<String>, ctx: &egui::Context) {
        self.api_key = api_key;

        // Clear existing tiles if no API key
        if self.api_key.is_none() {
            self.precipitation = None;
            self.clouds = None;
            self.wind = None;
        }
    }

    /// Get or create HttpTiles for a specific layer
    pub fn get_or_create_layer(
        &mut self,
        layer: WeatherLayer,
        ctx: &egui::Context,
    ) -> Option<&mut HttpTiles> {
        let api_key = self.api_key.as_ref()?;

        let tiles = match layer {
            WeatherLayer::Precipitation => &mut self.precipitation,
            WeatherLayer::Clouds => &mut self.clouds,
            WeatherLayer::Wind => &mut self.wind,
        };

        if tiles.is_none() {
            let cache_dir = dirs::cache_dir()
                .unwrap_or_else(|| std::path::PathBuf::from(".cache"))
                .join("airjedi-desktop")
                .join("weather")
                .join(layer.as_str());

            let http_options = HttpOptions {
                cache: Some(cache_dir),
                ..Default::default()
            };

            let source = OpenWeatherMapSource::new(layer, api_key.clone());
            *tiles = Some(HttpTiles::with_options(source, http_options, ctx.clone()));
        }

        tiles.as_mut()
    }
}

impl Default for WeatherTiles {
    fn default() -> Self {
        Self::new()
    }
}
```

**Step 3: Add module declaration to main.rs**

Add after `mod video;` (around line 30):

```rust
mod weather;
```

**Step 4: Build to verify**

Run: `cargo build`
Expected: Compiles without errors

**Step 5: Commit**

```bash
git add src/weather/mod.rs src/weather/openweathermap.rs src/main.rs
git commit -m "feat(weather): add OpenWeatherMap tile source module"
```

---

## Task 3: Add WeatherTiles to AirjediApp

**Files:**
- Modify: `src/main.rs` (AirjediApp struct and new())

**Step 1: Add import**

Add to imports at top of `src/main.rs` (after other use statements):

```rust
use weather::WeatherTiles;
```

**Step 2: Add field to AirjediApp struct**

Add after `waterfall_window: Option<ui::WaterfallWindow>,` (around line 670):

```rust
    // Weather overlay tiles
    weather_tiles: WeatherTiles,
```

**Step 3: Initialize in AirjediApp::new()**

Add after `waterfall_window: None,` in the Self block (around line 1120):

```rust
            weather_tiles: {
                let mut tiles = WeatherTiles::new();
                let api_key = WeatherTiles::resolve_api_key(
                    config.openweathermap_api_key.as_deref()
                );
                tiles.set_api_key(api_key, egui_ctx);
                tiles
            },
```

**Step 4: Build to verify**

Run: `cargo build`
Expected: Compiles without errors

**Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat(weather): integrate WeatherTiles into AirjediApp"
```

---

## Task 4: Add Weather Settings UI Section

**Files:**
- Modify: `src/main.rs` (Settings window, around line 3150-3577)

**Step 1: Add weather settings section**

Find the Settings window code (search for `egui::Window::new("Settings")`). Add this new section after the Video Streaming Test section (before the closing `});` of the window):

```rust
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(8.0);

                // Weather Layers section
                ui.heading(egui::RichText::new("Weather Layers")
                    .size(12.0)
                    .strong());

                ui.add_space(8.0);

                // API Key status and input
                let api_key_source = self.weather_tiles.api_key_source();
                let has_api_key = self.weather_tiles.has_api_key();

                if let Some(source) = api_key_source {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("✓")
                            .color(egui::Color32::from_rgb(100, 255, 100)));
                        ui.label(egui::RichText::new(format!("API key from {}", source))
                            .color(egui::Color32::from_rgb(150, 200, 150))
                            .size(9.0));
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("⚠")
                            .color(egui::Color32::from_rgb(255, 200, 100)));
                        ui.label(egui::RichText::new("API key required")
                            .color(egui::Color32::from_rgb(255, 200, 100))
                            .size(9.0));
                    });
                }

                ui.add_space(4.0);

                // API Key input field
                ui.horizontal(|ui| {
                    ui.label("API Key:");
                    let mut key_text = self.config.openweathermap_api_key
                        .clone()
                        .unwrap_or_default();
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut key_text)
                            .password(true)
                            .hint_text("Enter OpenWeatherMap API key")
                            .desired_width(200.0)
                    );

                    if response.changed() {
                        let new_key = if key_text.is_empty() { None } else { Some(key_text) };
                        self.config.openweathermap_api_key = new_key.clone();

                        // Update weather tiles with new key
                        let resolved_key = WeatherTiles::resolve_api_key(
                            self.config.openweathermap_api_key.as_deref()
                        );
                        self.weather_tiles.set_api_key(resolved_key, ctx);

                        if let Err(e) = self.config.save() {
                            eprintln!("Failed to save config: {}", e);
                        }
                    }

                    // Help button linking to OpenWeatherMap signup
                    if ui.button("?").on_hover_text("Get a free API key from OpenWeatherMap").clicked() {
                        let _ = webbrowser::open("https://home.openweathermap.org/api_keys");
                    }
                });

                ui.add_space(8.0);

                // Opacity slider
                ui.horizontal(|ui| {
                    ui.label("Opacity:");
                    if ui.add(
                        egui::Slider::new(&mut self.config.weather_opacity, 0.1..=1.0)
                            .show_value(true)
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                    ).changed() {
                        if let Err(e) = self.config.save() {
                            eprintln!("Failed to save config: {}", e);
                        }
                    }
                });

                ui.add_space(8.0);

                // Layer toggles (disabled if no API key)
                ui.add_enabled_ui(has_api_key, |ui| {
                    let mut precipitation_changed = false;
                    let mut clouds_changed = false;
                    let mut wind_changed = false;

                    ui.horizontal(|ui| {
                        precipitation_changed = ui.checkbox(
                            &mut self.config.show_weather_precipitation,
                            "Precipitation (rain/snow radar)"
                        ).changed();
                    });

                    ui.horizontal(|ui| {
                        clouds_changed = ui.checkbox(
                            &mut self.config.show_weather_clouds,
                            "Cloud Coverage"
                        ).changed();
                    });

                    ui.horizontal(|ui| {
                        wind_changed = ui.checkbox(
                            &mut self.config.show_weather_wind,
                            "Wind Speed"
                        ).changed();
                    });

                    if precipitation_changed || clouds_changed || wind_changed {
                        if let Err(e) = self.config.save() {
                            eprintln!("Failed to save config: {}", e);
                        }
                    }
                });

                if !has_api_key {
                    ui.label(egui::RichText::new("Enter API key to enable weather layers")
                        .size(8.0)
                        .color(egui::Color32::from_rgb(150, 150, 150)));
                }
```

**Step 2: Build to verify**

Run: `cargo build`
Expected: Compiles without errors

**Step 3: Test manually**

Run: `cargo run`
- Open Settings window
- Verify Weather Layers section appears
- Verify API key field works
- Verify toggles are disabled without API key

**Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat(weather): add Weather Layers section to Settings UI"
```

---

## Task 5: Implement Weather Tile Rendering

**Files:**
- Modify: `src/main.rs` (draw_map function)

**Step 1: Add weather tile drawing helper**

Add this function before `impl AirjediApp` (around line 700):

```rust
/// Draw weather tiles with opacity
fn draw_weather_tiles(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    projector: &walkers::Projector,
    tiles: &mut walkers::HttpTiles,
    opacity: f32,
    rect: egui::Rect,
) {
    // Get visible tile range from projector
    let zoom = projector.zoom().round() as u8;

    // Calculate tile bounds from viewport
    let top_left = projector.unproject(rect.left_top());
    let bottom_right = projector.unproject(rect.right_bottom());

    let min_tile_x = walkers::mercator::lon_to_x(top_left.x(), zoom) as i32;
    let max_tile_x = walkers::mercator::lon_to_x(bottom_right.x(), zoom) as i32;
    let min_tile_y = walkers::mercator::lat_to_y(top_left.y(), zoom) as i32;
    let max_tile_y = walkers::mercator::lat_to_y(bottom_right.y(), zoom) as i32;

    let tint = egui::Color32::from_rgba_unmultiplied(255, 255, 255, (opacity * 255.0) as u8);

    // Draw each visible tile
    for tile_x in min_tile_x..=max_tile_x {
        for tile_y in min_tile_y..=max_tile_y {
            let tile_id = walkers::TileId {
                x: tile_x as u32,
                y: tile_y as u32,
                zoom,
            };

            // Try to get the tile texture
            if let Some(tile) = tiles.at(tile_id) {
                if let Some(texture) = tile.texture() {
                    // Calculate screen position for this tile
                    let tile_nw_lon = walkers::mercator::x_to_lon(tile_x as f64, zoom);
                    let tile_nw_lat = walkers::mercator::y_to_lat(tile_y as f64, zoom);
                    let tile_se_lon = walkers::mercator::x_to_lon((tile_x + 1) as f64, zoom);
                    let tile_se_lat = walkers::mercator::y_to_lat((tile_y + 1) as f64, zoom);

                    let nw_screen = projector.project(walkers::lat_lon(tile_nw_lat, tile_nw_lon));
                    let se_screen = projector.project(walkers::lat_lon(tile_se_lat, tile_se_lon));

                    let tile_rect = egui::Rect::from_two_pos(
                        egui::pos2(nw_screen.x, nw_screen.y),
                        egui::pos2(se_screen.x, se_screen.y),
                    );

                    // Only draw if tile intersects viewport
                    if tile_rect.intersects(rect) {
                        painter.image(
                            texture.id(),
                            tile_rect,
                            egui::Rect::from_min_max(
                                egui::pos2(0.0, 0.0),
                                egui::pos2(1.0, 1.0),
                            ),
                            tint,
                        );
                    }
                }
            }
        }
    }
}
```

**Step 2: Add weather rendering to draw_map**

Find the Map::show() closure in `draw_map()`. After the receiver location marker is drawn (search for "Draw receiver location marker"), add weather tile rendering BEFORE the aviation overlays:

```rust
            // Draw weather layers (after basemap, before aviation overlays)
            if self.weather_tiles.has_api_key() {
                let weather_opacity = self.config.weather_opacity;

                // Precipitation layer (bottom weather layer)
                if self.config.show_weather_precipitation {
                    if let Some(tiles) = self.weather_tiles.get_or_create_layer(
                        weather::WeatherLayer::Precipitation,
                        ui.ctx(),
                    ) {
                        draw_weather_tiles(ui, painter, projector, tiles, weather_opacity, rect);
                    }
                }

                // Cloud layer
                if self.config.show_weather_clouds {
                    if let Some(tiles) = self.weather_tiles.get_or_create_layer(
                        weather::WeatherLayer::Clouds,
                        ui.ctx(),
                    ) {
                        draw_weather_tiles(ui, painter, projector, tiles, weather_opacity, rect);
                    }
                }

                // Wind layer (top weather layer)
                if self.config.show_weather_wind {
                    if let Some(tiles) = self.weather_tiles.get_or_create_layer(
                        weather::WeatherLayer::Wind,
                        ui.ctx(),
                    ) {
                        draw_weather_tiles(ui, painter, projector, tiles, weather_opacity, rect);
                    }
                }
            }
```

**Step 3: Build to verify**

Run: `cargo build`
Expected: May have compilation errors - see Step 4

**Step 4: Fix potential import/API issues**

The walkers crate may have different API than expected. If compilation fails, check:
- `walkers::mercator` module exists and has `lon_to_x`, `lat_to_y`, `x_to_lon`, `y_to_lat`
- If not, use the existing `WebMercator` struct from `src/map/tiles.rs`

Alternative using existing WebMercator:

```rust
use crate::map::WebMercator;

// Replace walkers::mercator calls with:
let min_tile_x = WebMercator::lon_to_x(top_left.x(), zoom) as i32;
// etc.
```

**Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat(weather): implement weather tile rendering on map"
```

---

## Task 6: Add Weather Attribution

**Files:**
- Modify: `src/main.rs` (attribution area, around line 3024-3036)

**Step 1: Update attribution text**

Find the map attribution area (search for `"map_attribution"`). Modify to include weather attribution when layers are active:

```rust
        // Build attribution text
        let base_attribution = "© OpenStreetMap contributors © CARTO";
        let attribution_text = if self.config.show_weather_precipitation
            || self.config.show_weather_clouds
            || self.config.show_weather_wind
        {
            format!("{} | Weather © OpenWeatherMap", base_attribution)
        } else {
            base_attribution.to_string()
        };

        egui::Area::new("map_attribution".into())
            .fixed_pos(egui::pos2(
                viewport.right() - animated_width - total_spacing,
                viewport.bottom() - 20.0
            ))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(&attribution_text)
                        .size(10.0)
                        .color(egui::Color32::from_rgba_unmultiplied(200, 200, 200, 180))
                );
            });
```

**Step 2: Build and test**

Run: `cargo build && cargo run`
- Enable a weather layer
- Verify attribution updates

**Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat(weather): add dynamic OpenWeatherMap attribution"
```

---

## Task 7: Add Weather Toggles to Map Overlays Window

**Files:**
- Modify: `src/main.rs` (Map Overlays window, around line 3039-3147)

**Step 1: Add weather section to Map Overlays**

Find the Map Overlays window (search for `egui::Window::new("Map Overlays")`). Add weather toggles after the existing overlay toggles but before the separator:

```rust
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Weather overlays section
                    ui.label(egui::RichText::new("Weather Overlays")
                        .color(egui::Color32::from_rgb(150, 200, 200))
                        .size(10.0)
                        .strong());

                    let has_api_key = self.weather_tiles.has_api_key();

                    if !has_api_key {
                        ui.label(egui::RichText::new("⚠ Configure API key in Settings")
                            .color(egui::Color32::from_rgb(255, 200, 100))
                            .size(9.0));
                    }

                    ui.add_enabled_ui(has_api_key, |ui| {
                        let mut weather_changed = false;

                        ui.horizontal(|ui| {
                            ui.label("Precipitation:");
                            if ui.checkbox(&mut self.config.show_weather_precipitation, "").changed() {
                                weather_changed = true;
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("Clouds:");
                            if ui.checkbox(&mut self.config.show_weather_clouds, "").changed() {
                                weather_changed = true;
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("Wind:");
                            if ui.checkbox(&mut self.config.show_weather_wind, "").changed() {
                                weather_changed = true;
                            }
                        });

                        if weather_changed {
                            if let Err(e) = self.config.save() {
                                eprintln!("Failed to save config: {}", e);
                            }
                        }
                    });
```

**Step 2: Build and test**

Run: `cargo build && cargo run`
- Open Map Overlays window
- Verify weather section appears
- With API key: toggles work
- Without API key: message shown, toggles disabled

**Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat(weather): add weather toggles to Map Overlays window"
```

---

## Task 8: Final Testing and Polish

**Files:**
- No file changes - testing only

**Step 1: Test without API key**

Run: `cargo run`
- Open Settings, verify "API key required" message
- Open Map Overlays, verify toggles disabled
- Verify no errors in console

**Step 2: Test with environment variable**

Run: `OPENWEATHERMAP_API_KEY=your_key_here cargo run`
- Verify "API key from environment variable" shown in Settings
- Enable precipitation layer
- Pan/zoom map, verify tiles load
- Verify tiles appear below aircraft

**Step 3: Test with config file**

- Enter API key in Settings field
- Restart app
- Verify key persists and layers work

**Step 4: Test opacity slider**

- Adjust opacity from 10% to 100%
- Verify visual change on weather tiles

**Step 5: Test layer ordering**

- Enable all three weather layers
- Verify aircraft still visible on top
- Verify airports/navaids visible on top of weather

**Step 6: Test caching**

- Enable weather layer
- Check `~/.cache/airjedi-desktop/weather/` exists
- Verify tiles are cached

**Step 7: Final commit**

```bash
git add -A
git commit -m "feat(weather): complete weather layer implementation"
```

---

## Summary

| Task | Description | Files |
|------|-------------|-------|
| 1 | Config fields | `src/config.rs` |
| 2 | Weather module | `src/weather/mod.rs`, `src/weather/openweathermap.rs` |
| 3 | App integration | `src/main.rs` |
| 4 | Settings UI | `src/main.rs` |
| 5 | Tile rendering | `src/main.rs` |
| 6 | Attribution | `src/main.rs` |
| 7 | Map Overlays UI | `src/main.rs` |
| 8 | Testing | N/A |

**Total estimated tasks:** 8 major tasks, ~30 individual steps

**Dependencies:** None (OpenWeatherMap uses standard HTTP, walkers already handles tile fetching)
