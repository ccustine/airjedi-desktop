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

use log::{info, warn};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::basestation::{Aircraft, AircraftTracker};
use crate::config::ServerConfig;
use crate::status::SharedSystemStatus;
use crate::tcp_client;

/// Represents a single server connection with its own tracker and lifecycle management
struct ServerConnection {
    /// Server configuration
    config: ServerConfig,

    /// Dedicated aircraft tracker for this server
    tracker: Arc<Mutex<AircraftTracker>>,

    /// Cancellation token for clean shutdown
    cancel_token: CancellationToken,

    /// Watch sender for hot-reloading server address
    address_tx: watch::Sender<String>,
}

impl ServerConnection {
    /// Create a new server connection
    fn new(config: ServerConfig, center_lat: f64, center_lon: f64) -> Self {
        // Create dedicated tracker for this server
        let mut tracker = AircraftTracker::new();
        tracker.set_center(center_lat, center_lon);
        tracker.set_server_info(config.id.clone(), config.name.clone());

        let tracker = Arc::new(Mutex::new(tracker));

        // Create watch channel for address hot-reload
        let (address_tx, _) = watch::channel(config.address.clone());

        // Create cancellation token
        let cancel_token = CancellationToken::new();

        Self {
            config,
            tracker,
            cancel_token,
            address_tx,
        }
    }

    /// Start the connection in a background task
    fn start(&self, status: SharedSystemStatus) {
        let server_id = self.config.id.clone();
        let server_name = self.config.name.clone();
        let address_rx = self.address_tx.subscribe();
        let tracker = self.tracker.clone();
        let status_clone = status.clone();
        let cancel_token = self.cancel_token.clone();

        // Register server in status tracking
        status.lock().unwrap().register_server(
            server_id.clone(),
            server_name.clone(),
            self.config.address.clone(),
        );

        info!("Starting connection to server '{}' ({})", server_name, self.config.address);

        // Spawn connection task
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(tcp_client::connect_adsb_feed(
                server_id,
                server_name,
                address_rx,
                tracker,
                status_clone,
                cancel_token,
            ));
        });
    }

    /// Stop the connection gracefully
    fn stop(&self, status: SharedSystemStatus) {
        info!("Stopping connection to server '{}'", self.config.name);
        self.cancel_token.cancel();

        // Update status to disconnected
        status.lock().unwrap().update_server_status(
            &self.config.id,
            crate::status::ConnectionStatus::Disconnected,
        );
    }

    /// Update the server address (hot-reload)
    fn update_address(&mut self, new_address: String) {
        info!("Updating address for server '{}': {} -> {}",
            self.config.name, self.config.address, new_address);
        self.config.address = new_address.clone();
        let _ = self.address_tx.send(new_address);
    }

    /// Get aircraft from this server
    fn get_aircraft(&self) -> Vec<Aircraft> {
        self.tracker.lock().unwrap().get_aircraft()
    }

    /// Update aircraft count in status
    fn update_status_aircraft_count(&self, status: &SharedSystemStatus) {
        let count = self.tracker.lock().unwrap().get_aircraft().len();
        status.lock().unwrap().update_server_aircraft_count(&self.config.id, count);
    }
}

/// Manages multiple server connections with independent lifecycle control
pub struct ConnectionManager {
    /// Active server connections (keyed by server_id)
    connections: HashMap<String, ServerConnection>,

    /// System status tracker
    status: SharedSystemStatus,

    /// Center location for distance filtering (shared across all connections)
    center_lat: f64,
    center_lon: f64,
}

impl ConnectionManager {
    /// Create a new connection manager
    pub fn new(status: SharedSystemStatus, center_lat: f64, center_lon: f64) -> Self {
        Self {
            connections: HashMap::new(),
            status,
            center_lat,
            center_lon,
        }
    }

    /// Set center location for all trackers
    pub fn set_center(&mut self, lat: f64, lon: f64) {
        self.center_lat = lat;
        self.center_lon = lon;

        // Update all existing trackers
        for connection in self.connections.values() {
            connection.tracker.lock().unwrap().set_center(lat, lon);
        }
    }

    /// Add and start a new server connection
    pub fn add_server(&mut self, config: ServerConfig) {
        let server_id = config.id.clone();
        let enabled = config.enabled;

        info!("Adding server '{}' ({}) - enabled: {}", config.name, config.address, enabled);

        // Create connection
        let connection = ServerConnection::new(config, self.center_lat, self.center_lon);

        // Start if enabled
        if enabled {
            connection.start(self.status.clone());
        }

        // Store connection
        self.connections.insert(server_id, connection);
    }

    /// Remove a server connection
    pub fn remove_server(&mut self, server_id: &str) {
        if let Some(connection) = self.connections.remove(server_id) {
            info!("Removing server '{}'", connection.config.name);

            // Stop connection
            connection.stop(self.status.clone());

            // Unregister from status
            self.status.lock().unwrap().unregister_server(server_id);
        } else {
            warn!("Attempted to remove non-existent server: {}", server_id);
        }
    }

    /// Enable a server (start connection)
    pub fn enable_server(&mut self, server_id: &str) {
        if let Some(connection) = self.connections.get_mut(server_id) {
            if !connection.config.enabled {
                info!("Enabling server '{}'", connection.config.name);
                connection.config.enabled = true;
                connection.start(self.status.clone());
            }
        } else {
            warn!("Attempted to enable non-existent server: {}", server_id);
        }
    }

    /// Disable a server (stop connection, keep config)
    pub fn disable_server(&mut self, server_id: &str) {
        if let Some(connection) = self.connections.get_mut(server_id) {
            if connection.config.enabled {
                info!("Disabling server '{}'", connection.config.name);
                connection.config.enabled = false;
                connection.stop(self.status.clone());
            }
        } else {
            warn!("Attempted to disable non-existent server: {}", server_id);
        }
    }

    /// Update server configuration (hot-reload address)
    pub fn update_server(&mut self, server_id: &str, new_config: ServerConfig) {
        if let Some(connection) = self.connections.get_mut(server_id) {
            info!("Updating server '{}' configuration", connection.config.name);

            // Update name if changed
            if connection.config.name != new_config.name {
                connection.config.name = new_config.name.clone();
            }

            // Update address if changed (hot-reload)
            if connection.config.address != new_config.address {
                connection.update_address(new_config.address.clone());
            }

            // Handle enabled state change
            if connection.config.enabled != new_config.enabled {
                if new_config.enabled {
                    connection.config.enabled = true;
                    connection.start(self.status.clone());
                } else {
                    connection.config.enabled = false;
                    connection.stop(self.status.clone());
                }
            }
        } else {
            warn!("Attempted to update non-existent server: {}", server_id);
        }
    }

    /// Get aircraft from a specific server
    #[allow(dead_code)]
    pub fn get_aircraft_by_server(&self, server_id: &str) -> Option<Vec<Aircraft>> {
        self.connections.get(server_id).map(|conn| conn.get_aircraft())
    }

    /// Find a specific aircraft by ICAO across all servers
    pub fn get_aircraft_by_icao(&self, icao: &str) -> Option<Aircraft> {
        for connection in self.connections.values() {
            if let Some(aircraft) = connection.tracker.lock().unwrap().get_aircraft_by_icao(icao) {
                return Some(aircraft);
            }
        }
        None
    }

    /// Get all aircraft grouped by server
    #[allow(dead_code)]
    pub fn get_all_aircraft_by_server(&self) -> HashMap<String, Vec<Aircraft>> {
        let mut result = HashMap::new();

        for (server_id, connection) in &self.connections {
            let aircraft = connection.get_aircraft();
            result.insert(server_id.clone(), aircraft);
        }

        result
    }

    /// Get all aircraft merged from all servers
    pub fn get_all_aircraft_merged(&self) -> Vec<Aircraft> {
        let mut all_aircraft = Vec::new();

        for connection in self.connections.values() {
            all_aircraft.extend(connection.get_aircraft());
        }

        all_aircraft
    }

    /// Get server configurations
    #[allow(dead_code)]
    pub fn get_server_configs(&self) -> Vec<ServerConfig> {
        self.connections.values()
            .map(|conn| conn.config.clone())
            .collect()
    }

    /// Update aircraft counts in status for all servers
    pub fn update_all_status_aircraft_counts(&self) {
        for connection in self.connections.values() {
            connection.update_status_aircraft_count(&self.status);
        }
    }

    /// Get the number of managed connections
    #[allow(dead_code)]
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// Get the number of enabled/active connections
    #[allow(dead_code)]
    pub fn active_connection_count(&self) -> usize {
        self.connections.values().filter(|conn| conn.config.enabled).count()
    }

    /// Set time-limited trails for all trackers
    pub fn set_time_limited_trails(&self, enabled: bool) {
        for connection in self.connections.values() {
            connection.tracker.lock().unwrap().set_time_limited_trails(enabled);
        }
    }

    /// Get time-limited trails setting (from first tracker, assumed same for all)
    pub fn get_time_limited_trails(&self) -> bool {
        self.connections.values()
            .next()
            .map(|conn| conn.tracker.lock().unwrap().get_time_limited_trails())
            .unwrap_or(false)
    }

    /// Cleanup old aircraft and position history for all trackers
    #[allow(dead_code)]
    pub fn cleanup_all(&self, max_age_seconds: i64) {
        for connection in self.connections.values() {
            connection.tracker.lock().unwrap().cleanup_old(max_age_seconds);
        }
    }
}

impl Drop for ConnectionManager {
    fn drop(&mut self) {
        info!("Shutting down ConnectionManager - stopping all connections");

        // Stop all connections gracefully
        for (_, connection) in &self.connections {
            connection.stop(self.status.clone());
        }
    }
}
