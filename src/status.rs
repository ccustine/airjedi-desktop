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
use std::collections::VecDeque;
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

/// System status tracking all metrics and diagnostics
pub struct SystemStatus {
    // Connection status
    pub connection_status: ConnectionStatus,
    pub connection_address: String,
    pub last_connection_attempt: Option<DateTime<Utc>>,
    pub last_successful_connection: Option<DateTime<Utc>>,
    pub connection_uptime_seconds: u64,

    // Message statistics
    pub total_messages_received: u64,
    pub messages_per_second: f64,
    pub messages_last_second: VecDeque<(DateTime<Utc>, u64)>, // Ring buffer for rate calculation

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
            connection_status: ConnectionStatus::Disconnected,
            connection_address: "localhost:30003".to_string(),
            last_connection_attempt: None,
            last_successful_connection: None,
            connection_uptime_seconds: 0,

            total_messages_received: 0,
            messages_per_second: 0.0,
            messages_last_second: VecDeque::with_capacity(60),

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
    pub fn set_connection_error(&mut self, error: String) {
        self.connection_status = ConnectionStatus::Error;
        self.connection_uptime_seconds = 0;
        self.add_diagnostic(DiagnosticLevel::Error,
            format!("Connection error: {}", error));
    }

    /// Increment message counter and update rate
    pub fn increment_message_count(&mut self) {
        self.total_messages_received += 1;

        // Update messages per second calculation
        let now = Utc::now();
        self.messages_last_second.push_back((now, self.total_messages_received));

        // Remove entries older than 1 second
        while let Some((timestamp, _)) = self.messages_last_second.front() {
            if (now - *timestamp).num_milliseconds() > 1000 {
                self.messages_last_second.pop_front();
            } else {
                break;
            }
        }

        // Calculate messages per second
        if self.messages_last_second.len() >= 2 {
            if let (Some((oldest_time, oldest_count)), Some((newest_time, newest_count))) =
                (self.messages_last_second.front(), self.messages_last_second.back()) {
                let duration_secs = (newest_time.timestamp_millis() - oldest_time.timestamp_millis()) as f64 / 1000.0;
                if duration_secs > 0.0 {
                    let message_diff = newest_count.saturating_sub(*oldest_count) as f64;
                    self.messages_per_second = message_diff / duration_secs;
                }
            }
        }
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
}

/// Thread-safe wrapper for SystemStatus
pub type SharedSystemStatus = Arc<Mutex<SystemStatus>>;
