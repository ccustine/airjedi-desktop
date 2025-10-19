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

use serde::{Deserialize, Serialize};

/// Default server address for ADS-B feed
pub const DEFAULT_SERVER_ADDRESS: &str = "localhost:30003";

/// Application configuration stored in TOML format
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    /// SBS-1/BaseStation server address (host:port)
    pub server_address: String,

    /// Show airports on map
    pub show_airports: bool,

    /// Show runways on map
    pub show_runways: bool,

    /// Show navaids (VOR, NDB, etc.) on map
    pub show_navaids: bool,

    /// Default map zoom level (6.0 - 12.0)
    pub default_zoom: f32,

    /// Enable time-limited trail display (fades over 5 minutes)
    pub time_limited_trails: bool,

    /// Airport filter mode: "All", "FrequentlyUsed", or "MajorOnly"
    pub airport_filter: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server_address: DEFAULT_SERVER_ADDRESS.to_string(),
            show_airports: true,
            show_runways: true,
            show_navaids: false,
            default_zoom: 8.0,
            time_limited_trails: false,
            airport_filter: "FrequentlyUsed".to_string(),
        }
    }
}

impl AppConfig {
    /// Load configuration from disk, creating default config file if it doesn't exist
    pub fn load() -> Result<Self, confy::ConfyError> {
        confy::load("airjedi-desktop", "config")
    }

    /// Save configuration to disk
    pub fn save(&self) -> Result<(), confy::ConfyError> {
        confy::store("airjedi-desktop", "config", self)
    }

    /// Get the config file path for display to user
    pub fn get_config_path() -> Result<std::path::PathBuf, confy::ConfyError> {
        confy::get_configuration_file_path("airjedi-desktop", "config")
    }
}
