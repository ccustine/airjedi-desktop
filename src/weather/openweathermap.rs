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
    #[allow(dead_code)]
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
    pub fn set_api_key(&mut self, api_key: Option<String>, _ctx: &egui::Context) {
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
