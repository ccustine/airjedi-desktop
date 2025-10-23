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

use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Connection status for ADS-B feed
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    Error,
}

/// Diagnostic message with timestamp
#[derive(Debug, Clone)]
pub struct DiagnosticMessage {
    pub timestamp: DateTime<Utc>,
    pub level: DiagnosticLevel,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

/// Per-server connection status and statistics
#[derive(Debug, Clone)]
pub struct ServerStatus {
    /// Unique server ID
    #[allow(dead_code)]
    pub server_id: String,

    /// Server display name
    pub server_name: String,

    /// Server address (host:port)
    pub server_address: String,

    /// Current connection status
    pub status: ConnectionStatus,

    /// Last error message (if any)
    pub last_error: Option<String>,

    /// Total messages received from this server
    pub message_count: u64,

    /// Number of aircraft currently tracked from this server
    pub aircraft_count: usize,

    /// When the connection was established
    pub connected_at: Option<DateTime<Utc>>,

    /// Last time a message was received
    pub last_message_at: Option<DateTime<Utc>>,
}

impl ServerStatus {
    /// Create a new server status tracker
    pub fn new(server_id: String, server_name: String, server_address: String) -> Self {
        Self {
            server_id,
            server_name,
            server_address,
            status: ConnectionStatus::Disconnected,
            last_error: None,
            message_count: 0,
            aircraft_count: 0,
            connected_at: None,
            last_message_at: None,
        }
    }

    /// Get connection uptime in seconds
    #[allow(dead_code)]
    pub fn uptime_seconds(&self) -> u64 {
        if self.status == ConnectionStatus::Connected {
            if let Some(connected) = self.connected_at {
                (Utc::now() - connected).num_seconds() as u64
            } else {
                0
            }
        } else {
            0
        }
    }
}

/// System status tracking all metrics and diagnostics
pub struct SystemStatus {
    // Per-server status tracking
    pub servers: HashMap<String, ServerStatus>,
    // Connection status
    pub connection_status: ConnectionStatus,
    pub connection_address: String,
    #[allow(dead_code)]
    pub last_connection_attempt: Option<DateTime<Utc>>,
    pub last_successful_connection: Option<DateTime<Utc>>,
    pub connection_uptime_seconds: u64,

    // Message statistics
    pub total_messages_received: u64,

    // Position update statistics (for sparkline visualization)
    pub position_updates_per_second: f64,
    pub position_updates_history: VecDeque<(DateTime<Utc>, u32)>, // Last 60 seconds of position update counts

    // Aircraft statistics
    pub total_aircraft_tracked: usize,
    pub active_aircraft: usize,

    // Aviation data status
    pub aviation_data_loaded: bool,
    pub airports_loaded: usize,
    pub runways_loaded: usize,
    pub navaids_loaded: usize,

    // Aircraft database status
    pub aircraft_db_loaded: bool,
    pub aircraft_db_size: usize,

    // Diagnostic messages (keep last 50)
    pub diagnostics: VecDeque<DiagnosticMessage>,
    max_diagnostics: usize,

    // Performance metrics
    pub last_update_duration_ms: f64,
    pub average_update_duration_ms: f64,
}

impl Default for SystemStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemStatus {
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
            connection_status: ConnectionStatus::Disconnected,
            connection_address: String::new(),
            last_connection_attempt: None,
            last_successful_connection: None,
            connection_uptime_seconds: 0,

            total_messages_received: 0,

            position_updates_per_second: 0.0,
            position_updates_history: VecDeque::with_capacity(60),

            total_aircraft_tracked: 0,
            active_aircraft: 0,

            aviation_data_loaded: false,
            airports_loaded: 0,
            runways_loaded: 0,
            navaids_loaded: 0,

            aircraft_db_loaded: false,
            aircraft_db_size: 0,

            diagnostics: VecDeque::with_capacity(50),
            max_diagnostics: 50,

            last_update_duration_ms: 0.0,
            average_update_duration_ms: 0.0,
        }
    }

    /// Update connection status
    #[allow(dead_code)]
    pub fn set_connection_status(&mut self, status: ConnectionStatus) {
        self.connection_status = status;

        match status {
            ConnectionStatus::Connecting => {
                self.last_connection_attempt = Some(Utc::now());
                self.add_diagnostic(DiagnosticLevel::Info,
                    format!("Connecting to {}...", self.connection_address));
            }
            ConnectionStatus::Connected => {
                self.last_successful_connection = Some(Utc::now());
                self.add_diagnostic(DiagnosticLevel::Info,
                    format!("Connected to {}", self.connection_address));
            }
            ConnectionStatus::Disconnected => {
                self.connection_uptime_seconds = 0;
                self.add_diagnostic(DiagnosticLevel::Warning,
                    "Disconnected from ADS-B feed".to_string());
            }
            ConnectionStatus::Error => {
                self.connection_uptime_seconds = 0;
            }
        }
    }

    /// Record a connection error
    #[allow(dead_code)]
    pub fn set_connection_error(&mut self, error: String) {
        self.connection_status = ConnectionStatus::Error;
        self.connection_uptime_seconds = 0;
        self.add_diagnostic(DiagnosticLevel::Error,
            format!("Connection error: {}", error));
    }

    /// Increment message counter
    #[allow(dead_code)]
    pub fn increment_message_count(&mut self) {
        self.total_messages_received += 1;
    }

    /// Record a position update for sparkline visualization
    pub fn record_position_update(&mut self) {
        let now = Utc::now();

        // Find or create entry for the current second
        if let Some((last_time, count)) = self.position_updates_history.back_mut() {
            // If the last entry is from the same second, increment its count
            if (now - *last_time).num_milliseconds() < 1000 {
                *count += 1;
            } else {
                // New second - add a new entry
                self.position_updates_history.push_back((now, 1));
            }
        } else {
            // First entry
            self.position_updates_history.push_back((now, 1));
        }

        // Remove entries older than 60 seconds
        while let Some((timestamp, _)) = self.position_updates_history.front() {
            if (now - *timestamp).num_seconds() > 60 {
                self.position_updates_history.pop_front();
            } else {
                break;
            }
        }

        // Calculate average position updates per second over the last 10 seconds
        let ten_secs_ago = now - chrono::Duration::seconds(10);
        let recent_updates: u32 = self.position_updates_history
            .iter()
            .filter(|(timestamp, _)| *timestamp >= ten_secs_ago)
            .map(|(_, count)| count)
            .sum();

        let recent_duration = self.position_updates_history
            .iter()
            .filter(|(timestamp, _)| *timestamp >= ten_secs_ago)
            .count() as f64;

        if recent_duration > 0.0 {
            self.position_updates_per_second = recent_updates as f64 / recent_duration;
        }
    }

    /// Update aircraft statistics
    pub fn update_aircraft_stats(&mut self, total: usize, active: usize) {
        self.total_aircraft_tracked = total;
        self.active_aircraft = active;
    }

    /// Set aviation data status
    pub fn set_aviation_data(&mut self, airports: usize, runways: usize, navaids: usize) {
        self.aviation_data_loaded = true;
        self.airports_loaded = airports;
        self.runways_loaded = runways;
        self.navaids_loaded = navaids;
        self.add_diagnostic(DiagnosticLevel::Info,
            format!("Aviation data loaded: {} airports, {} runways, {} navaids",
                airports, runways, navaids));
    }

    /// Set aircraft database status
    pub fn set_aircraft_db(&mut self, size: usize) {
        self.aircraft_db_loaded = true;
        self.aircraft_db_size = size;
        self.add_diagnostic(DiagnosticLevel::Info,
            format!("Aircraft database loaded: {} aircraft", size));
    }

    /// Add a diagnostic message
    pub fn add_diagnostic(&mut self, level: DiagnosticLevel, message: String) {
        let diagnostic = DiagnosticMessage {
            timestamp: Utc::now(),
            level,
            message,
        };

        self.diagnostics.push_back(diagnostic);

        // Keep only the last N messages
        while self.diagnostics.len() > self.max_diagnostics {
            self.diagnostics.pop_front();
        }
    }

    /// Update connection uptime
    pub fn update_uptime(&mut self) {
        if self.connection_status == ConnectionStatus::Connected {
            if let Some(connect_time) = self.last_successful_connection {
                self.connection_uptime_seconds = (Utc::now() - connect_time).num_seconds() as u64;
            }
        }
    }

    /// Update performance metrics
    pub fn update_performance(&mut self, duration_ms: f64) {
        self.last_update_duration_ms = duration_ms;

        // Simple moving average
        const ALPHA: f64 = 0.1; // Smoothing factor
        if self.average_update_duration_ms == 0.0 {
            self.average_update_duration_ms = duration_ms;
        } else {
            self.average_update_duration_ms =
                ALPHA * duration_ms + (1.0 - ALPHA) * self.average_update_duration_ms;
        }
    }

    // ===== Per-Server Status Management =====

    /// Register a new server for tracking
    pub fn register_server(&mut self, server_id: String, server_name: String, server_address: String) {
        let status = ServerStatus::new(server_id.clone(), server_name, server_address);
        self.servers.insert(server_id, status);
    }

    /// Remove a server from tracking
    pub fn unregister_server(&mut self, server_id: &str) {
        self.servers.remove(server_id);
    }

    /// Update server connection status
    pub fn update_server_status(&mut self, server_id: &str, status: ConnectionStatus) {
        // Extract server info first to avoid borrow conflicts
        let diagnostic_message = if let Some(server_status) = self.servers.get_mut(server_id) {
            server_status.status = status;

            let msg = match status {
                ConnectionStatus::Connected => {
                    server_status.connected_at = Some(Utc::now());
                    server_status.last_error = None;
                    Some((DiagnosticLevel::Info,
                        format!("[{}] Connected to {}", server_status.server_name, server_status.server_address)))
                }
                ConnectionStatus::Connecting => {
                    Some((DiagnosticLevel::Info,
                        format!("[{}] Connecting to {}...", server_status.server_name, server_status.server_address)))
                }
                ConnectionStatus::Disconnected => {
                    server_status.connected_at = None;
                    Some((DiagnosticLevel::Warning,
                        format!("[{}] Disconnected from {}", server_status.server_name, server_status.server_address)))
                }
                ConnectionStatus::Error => {
                    server_status.connected_at = None;
                    None
                }
            };
            msg
        } else {
            None
        };

        // Add diagnostic after releasing the borrow
        if let Some((level, message)) = diagnostic_message {
            self.add_diagnostic(level, message);
        }
    }

    /// Record a connection error for a server
    pub fn update_server_error(&mut self, server_id: &str, error: String) {
        // Extract server info first to avoid borrow conflicts
        let diagnostic_message = if let Some(server_status) = self.servers.get_mut(server_id) {
            server_status.status = ConnectionStatus::Error;
            server_status.last_error = Some(error.clone());
            server_status.connected_at = None;
            Some(format!("[{}] Connection error: {}", server_status.server_name, error))
        } else {
            None
        };

        // Add diagnostic after releasing the borrow
        if let Some(message) = diagnostic_message {
            self.add_diagnostic(DiagnosticLevel::Error, message);
        }
    }

    /// Increment message count for a server
    pub fn increment_server_message_count(&mut self, server_id: &str) {
        if let Some(server_status) = self.servers.get_mut(server_id) {
            server_status.message_count += 1;
            server_status.last_message_at = Some(Utc::now());
        }
    }

    /// Update aircraft count for a server
    pub fn update_server_aircraft_count(&mut self, server_id: &str, count: usize) {
        if let Some(server_status) = self.servers.get_mut(server_id) {
            server_status.aircraft_count = count;
        }
    }

    /// Get status for a specific server
    #[allow(dead_code)]
    pub fn get_server_status(&self, server_id: &str) -> Option<&ServerStatus> {
        self.servers.get(server_id)
    }

    /// Get all server statuses
    #[allow(dead_code)]
    pub fn get_all_server_statuses(&self) -> Vec<&ServerStatus> {
        self.servers.values().collect()
    }

    /// Get total message count across all servers
    #[allow(dead_code)]
    pub fn get_total_server_messages(&self) -> u64 {
        self.servers.values().map(|s| s.message_count).sum()
    }

    /// Get total aircraft count across all servers
    #[allow(dead_code)]
    pub fn get_total_server_aircraft(&self) -> usize {
        self.servers.values().map(|s| s.aircraft_count).sum()
    }

    /// Get number of connected servers
    pub fn get_connected_server_count(&self) -> usize {
        self.servers.values().filter(|s| s.status == ConnectionStatus::Connected).count()
    }

    /// Update server name and address in status display
    pub fn update_server_info(&mut self, server_id: &str, name: String, address: String) {
        if let Some(server_status) = self.servers.get_mut(server_id) {
            server_status.server_name = name;
            server_status.server_address = address;
        }
    }
}

/// Thread-safe wrapper for SystemStatus
pub type SharedSystemStatus = Arc<Mutex<SystemStatus>>;
