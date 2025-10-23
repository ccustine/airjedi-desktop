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
use uuid::Uuid;

/// Default server address for ADS-B feed
pub const DEFAULT_SERVER_ADDRESS: &str = "localhost:30003";

/// Server configuration for a single ADS-B feed connection
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    /// Unique identifier for this server (stable across renames)
    pub id: String,

    /// User-friendly display name
    pub name: String,

    /// Server address in host:port format
    pub address: String,

    /// Whether this server should auto-connect on startup
    pub enabled: bool,
}

impl ServerConfig {
    /// Create a new server configuration with a generated UUID
    pub fn new(name: String, address: String, enabled: bool) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            address,
            enabled,
        }
    }

    /// Create the default local server
    pub fn default_local() -> Self {
        Self::new(
            "Default Local Server".to_string(),
            DEFAULT_SERVER_ADDRESS.to_string(),
            true,
        )
    }
}

/// Legacy configuration format for migration (pre-multi-server)
#[derive(Debug, Default, Serialize, Deserialize)]
struct LegacyAppConfig {
    server_address: Option<String>,
    show_airports: Option<bool>,
    show_runways: Option<bool>,
    show_navaids: Option<bool>,
    default_zoom: Option<f32>,
    time_limited_trails: Option<bool>,
    airport_filter: Option<String>,
    aircraft_list_expanded: Option<bool>,
    aircraft_list_width: Option<f32>,
}

/// Application configuration stored in TOML format
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    /// List of configured ADS-B servers
    #[serde(default = "default_servers")]
    pub servers: Vec<ServerConfig>,

    /// Show airports on map
    #[serde(default = "default_true")]
    pub show_airports: bool,

    /// Show runways on map
    #[serde(default = "default_true")]
    pub show_runways: bool,

    /// Show navaids (VOR, NDB, etc.) on map
    #[serde(default)]
    pub show_navaids: bool,

    /// Default map zoom level (6.0 - 12.0)
    #[serde(default = "default_zoom")]
    pub default_zoom: f32,

    /// Enable time-limited trail display (fades over 5 minutes)
    #[serde(default)]
    pub time_limited_trails: bool,

    /// Airport filter mode: "All", "FrequentlyUsed", or "MajorOnly"
    #[serde(default = "default_airport_filter")]
    pub airport_filter: String,

    /// Aircraft list panel expanded state
    #[serde(default = "default_true")]
    pub aircraft_list_expanded: bool,

    /// Aircraft list panel width in pixels
    #[serde(default = "default_aircraft_list_width")]
    pub aircraft_list_width: f32,
}

// Default value functions for serde
fn default_servers() -> Vec<ServerConfig> {
    vec![ServerConfig::default_local()]
}

fn default_true() -> bool {
    true
}

fn default_zoom() -> f32 {
    7.0
}

fn default_airport_filter() -> String {
    "FrequentlyUsed".to_string()
}

fn default_aircraft_list_width() -> f32 {
    350.0
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            servers: default_servers(),
            show_airports: true,
            show_runways: true,
            show_navaids: false,
            default_zoom: 7.0,
            time_limited_trails: false,
            airport_filter: "FrequentlyUsed".to_string(),
            aircraft_list_expanded: true,
            aircraft_list_width: 350.0,
        }
    }
}

impl AppConfig {
    /// Load configuration from disk with automatic migration from legacy format
    pub fn load() -> Result<Self, confy::ConfyError> {
        // Try to load as new format first
        let config: AppConfig = confy::load("airjedi-desktop", "config")?;

        // Check if we need to migrate from legacy format
        // If servers list is empty, try to load legacy config
        if config.servers.is_empty() {
            if let Ok(legacy_config) = Self::try_load_legacy() {
                println!("Migrating from legacy single-server configuration...");
                let migrated = Self::migrate_from_legacy(legacy_config);

                // Save migrated config immediately
                migrated.save()?;
                println!("Configuration migrated successfully");

                return Ok(migrated);
            }
        }

        Ok(config)
    }

    /// Attempt to load legacy configuration format
    fn try_load_legacy() -> Result<LegacyAppConfig, confy::ConfyError> {
        confy::load("airjedi-desktop", "config")
    }

    /// Migrate from legacy single-server format to multi-server format
    fn migrate_from_legacy(legacy: LegacyAppConfig) -> Self {
        // Create server config from legacy server_address
        let servers = if let Some(address) = legacy.server_address {
            vec![ServerConfig::new(
                "Default Local Server".to_string(),
                address,
                true,
            )]
        } else {
            default_servers()
        };

        Self {
            servers,
            show_airports: legacy.show_airports.unwrap_or(true),
            show_runways: legacy.show_runways.unwrap_or(true),
            show_navaids: legacy.show_navaids.unwrap_or(false),
            default_zoom: legacy.default_zoom.unwrap_or(7.0),
            time_limited_trails: legacy.time_limited_trails.unwrap_or(false),
            airport_filter: legacy.airport_filter.unwrap_or_else(|| "FrequentlyUsed".to_string()),
            aircraft_list_expanded: legacy.aircraft_list_expanded.unwrap_or(true),
            aircraft_list_width: legacy.aircraft_list_width.unwrap_or(350.0),
        }
    }

    /// Save configuration to disk
    pub fn save(&self) -> Result<(), confy::ConfyError> {
        confy::store("airjedi-desktop", "config", self)
    }

    /// Get the config file path for display to user
    pub fn get_config_path() -> Result<std::path::PathBuf, confy::ConfyError> {
        confy::get_configuration_file_path("airjedi-desktop", "config")
    }

    /// Get a server by ID
    #[allow(dead_code)]
    pub fn get_server(&self, id: &str) -> Option<&ServerConfig> {
        self.servers.iter().find(|s| s.id == id)
    }

    /// Get a mutable server by ID
    #[allow(dead_code)]
    pub fn get_server_mut(&mut self, id: &str) -> Option<&mut ServerConfig> {
        self.servers.iter_mut().find(|s| s.id == id)
    }

    /// Add a new server
    pub fn add_server(&mut self, server: ServerConfig) {
        self.servers.push(server);
    }

    /// Remove a server by ID
    pub fn remove_server(&mut self, id: &str) -> bool {
        if let Some(pos) = self.servers.iter().position(|s| s.id == id) {
            self.servers.remove(pos);
            true
        } else {
            false
        }
    }
}
