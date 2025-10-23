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
use std::sync::{Arc, RwLock, Mutex};
use chrono::{DateTime, Utc};
use crate::status::SystemStatus;

// Constants for position validation and tracking
const NAUTICAL_MILE_CONVERSION: f64 = 1.15078; // 1 nautical mile = 1.15078 statute miles
const JUMP_DETECTION_TIME_WINDOW_SECONDS: i64 = 20; // Only apply jump detection within this time window
const JUMP_DETECTION_THRESHOLD_MILES: f64 = 10.0; // Maximum allowed position jump in miles
const MAX_CONSECUTIVE_REJECTIONS: u32 = 3; // Accept position after this many rejections (likely data delay)
const POSITION_CHANGE_THRESHOLD_DEGREES: f64 = 0.001; // ~100 meters at mid-latitudes
const TRAIL_HISTORY_SECONDS: i64 = 300; // Keep 5 minutes of position history

// Calculate distance between two lat/lon points using Haversine formula (in miles)
fn haversine_distance(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 3958.8; // Earth's radius in miles

    let lat1_rad = lat1.to_radians();
    let lat2_rad = lat2.to_radians();
    let delta_lat = (lat2 - lat1).to_radians();
    let delta_lon = (lon2 - lon1).to_radians();

    let a = (delta_lat / 2.0).sin().powi(2)
        + lat1_rad.cos() * lat2_rad.cos() * (delta_lon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    r * c
}

// Calculate distance in nautical miles between two lat/lon points
pub fn haversine_distance_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let statute_miles = haversine_distance(lat1, lon1, lat2, lon2);
    // Convert statute miles to nautical miles
    statute_miles / NAUTICAL_MILE_CONVERSION
}

#[derive(Debug, Clone)]
pub struct PositionPoint {
    pub lat: f64,
    pub lon: f64,
    pub altitude: Option<i32>,
    pub timestamp: DateTime<Utc>,
}

/// Inner aircraft data protected by RwLock for thread-safe interior mutability
#[derive(Debug)]
pub struct AircraftData {
    pub icao: String,
    pub callsign: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<i32>,
    pub track: Option<f64>,
    pub velocity: Option<f64>,
    pub vertical_rate: Option<i32>,
    pub last_seen: DateTime<Utc>,
    pub position_history: Vec<PositionPoint>,
    pub consecutive_rejections: u32,
    // Server source tracking
    #[allow(dead_code)]
    pub source_server_id: String,
    pub source_server_name: String,
    // Metadata fields
    pub registration: Option<String>,
    pub aircraft_type: Option<String>,
    pub photo_url: Option<String>,
    pub photo_thumbnail_url: Option<String>,
    pub photographer: Option<String>,
    pub metadata_fetched: bool,
}

/// Aircraft wrapper that can be cheaply cloned via Arc
#[derive(Debug, Clone)]
pub struct Aircraft {
    inner: Arc<RwLock<AircraftData>>,
}

impl Aircraft {
    pub fn new(icao: String, source_server_id: String, source_server_name: String) -> Self {
        Self {
            inner: Arc::new(RwLock::new(AircraftData {
                icao,
                callsign: None,
                latitude: None,
                longitude: None,
                altitude: None,
                track: None,
                velocity: None,
                vertical_rate: None,
                last_seen: Utc::now(),
                position_history: Vec::new(),
                consecutive_rejections: 0,
                source_server_id,
                source_server_name,
                registration: None,
                aircraft_type: None,
                photo_url: None,
                photo_thumbnail_url: None,
                photographer: None,
                metadata_fetched: false,
            })),
        }
    }

    // Convenience accessor methods for common read-only operations
    pub fn icao(&self) -> String {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .icao.clone()
    }

    pub fn callsign(&self) -> Option<String> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .callsign.clone()
    }

    pub fn latitude(&self) -> Option<f64> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .latitude
    }

    pub fn longitude(&self) -> Option<f64> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .longitude
    }

    pub fn altitude(&self) -> Option<i32> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .altitude
    }

    pub fn track(&self) -> Option<f64> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .track
    }

    pub fn velocity(&self) -> Option<f64> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .velocity
    }

    #[allow(dead_code)]
    pub fn vertical_rate(&self) -> Option<i32> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .vertical_rate
    }

    pub fn last_seen(&self) -> DateTime<Utc> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .last_seen
    }

    pub fn registration(&self) -> Option<String> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .registration.clone()
    }

    pub fn aircraft_type(&self) -> Option<String> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .aircraft_type.clone()
    }

    #[allow(dead_code)]
    pub fn photo_url(&self) -> Option<String> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .photo_url.clone()
    }

    pub fn photo_thumbnail_url(&self) -> Option<String> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .photo_thumbnail_url.clone()
    }

    #[allow(dead_code)]
    pub fn photographer(&self) -> Option<String> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .photographer.clone()
    }

    pub fn metadata_fetched(&self) -> bool {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .metadata_fetched
    }

    #[allow(dead_code)]
    pub fn source_server_id(&self) -> String {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .source_server_id.clone()
    }

    pub fn source_server_name(&self) -> String {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .source_server_name.clone()
    }

    /// Execute a closure with read-only access to position history
    /// This avoids cloning the entire vector, which is expensive when called every frame
    #[allow(dead_code)]
    pub fn with_position_history<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&[PositionPoint]) -> R,
    {
        let data = self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state");
        f(&data.position_history)
    }

    /// Get a cloned copy of the position history
    /// Note: This clones the entire vector - prefer `with_position_history()` for read-only access
    #[allow(dead_code)]
    pub fn position_history(&self) -> Vec<PositionPoint> {
        self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state")
            .position_history.clone()
    }

    /// Calculate distance in nautical miles from a given point to this aircraft
    pub fn distance_from_nm(&self, from_lat: f64, from_lon: f64) -> Option<f64> {
        let data = self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state");
        if let (Some(lat), Some(lon)) = (data.latitude, data.longitude) {
            Some(haversine_distance_nm(from_lat, from_lon, lat, lon))
        } else {
            None
        }
    }

    // Method to execute a read closure with locked data
    pub fn with_data<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&AircraftData) -> R,
    {
        let data = self.inner.read()
            .expect("Aircraft data lock poisoned - unrecoverable state");
        f(&data)
    }

    // Method to execute a write closure with locked data
    pub fn with_data_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut AircraftData) -> R,
    {
        let mut data = self.inner.write()
            .expect("Aircraft data lock poisoned - unrecoverable state");
        f(&mut data)
    }

    pub fn update_position(&self, lat: f64, lon: f64, center_lat: f64, center_lon: f64, max_distance: f64) -> bool {
        let mut data = self.inner.write()
            .expect("Aircraft data lock poisoned - unrecoverable state");

        // Check if position is within max distance from center
        let distance_from_center = haversine_distance(center_lat, center_lon, lat, lon);
        if distance_from_center > max_distance {
            return false; // Position rejected - too far from center
        }

        // Check if position is within threshold of previous position (only if recent update)
        if let (Some(last_lat), Some(last_lon)) = (data.latitude, data.longitude) {
            let time_since_last_update = (Utc::now() - data.last_seen).num_seconds();

            // Only apply jump detection if last update was recent
            // This prevents false rejections after connectivity gaps
            if time_since_last_update <= JUMP_DETECTION_TIME_WINDOW_SECONDS {
                let distance_from_last = haversine_distance(last_lat, last_lon, lat, lon);
                if distance_from_last > JUMP_DETECTION_THRESHOLD_MILES {
                    // Check if we've already rejected multiple times in a row
                    // If so, assume the data is actually correct (likely a delay/gap)
                    if data.consecutive_rejections >= MAX_CONSECUTIVE_REJECTIONS {
                        info!("Accepting position for {} after {} consecutive rejections (jumped {:.1} miles)",
                            data.icao, data.consecutive_rejections, distance_from_last);
                        data.consecutive_rejections = 0;
                        // Continue with position update
                    } else {
                        // Position jump too large - reject and increment counter
                        data.consecutive_rejections += 1;
                        warn!("Rejected position for {}: jumped {:.1} miles (rejection {} of 3)",
                            data.icao, distance_from_last, data.consecutive_rejections);
                        return false;
                    }
                }
            }
        }

        // Only add to history if position has changed significantly
        let should_add = if let (Some(last_lat), Some(last_lon)) = (data.latitude, data.longitude) {
            // Fast Euclidean approximation - accurate enough for ~100m threshold
            let distance = ((lat - last_lat).powi(2) + (lon - last_lon).powi(2)).sqrt();
            distance > POSITION_CHANGE_THRESHOLD_DEGREES
        } else {
            true
        };

        if should_add {
            let altitude = data.altitude;  // Read altitude first
            data.position_history.push(PositionPoint {
                lat,
                lon,
                altitude,
                timestamp: Utc::now(),
            });
        }

        data.latitude = Some(lat);
        data.longitude = Some(lon);

        // Reset rejection counter on successful position update
        data.consecutive_rejections = 0;

        true
    }

    pub fn cleanup_old_history(&self, max_age_seconds: i64) {
        let mut data = self.inner.write()
            .expect("Aircraft data lock poisoned - unrecoverable state");
        let now = Utc::now();
        data.position_history.retain(|point| {
            (now - point.timestamp).num_seconds() < max_age_seconds
        });
    }
}

pub struct AircraftTracker {
    aircraft: HashMap<String, Aircraft>,
    center_lat: f64,
    center_lon: f64,
    max_distance_miles: f64,
    status: Option<Arc<Mutex<SystemStatus>>>,
    time_limited_trails: bool,
    // Server source information
    server_id: String,
    server_name: String,
}

impl Default for AircraftTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl AircraftTracker {
    pub fn new() -> Self {
        Self {
            aircraft: HashMap::new(),
            center_lat: 0.0,
            center_lon: 0.0,
            max_distance_miles: 400.0,
            status: None,
            time_limited_trails: false,  // Default to full history trails
            server_id: String::new(),
            server_name: String::new(),
        }
    }

    #[allow(dead_code)]
    pub fn set_status(&mut self, status: Arc<Mutex<SystemStatus>>) {
        self.status = Some(status);
    }

    pub fn set_center(&mut self, lat: f64, lon: f64) {
        self.center_lat = lat;
        self.center_lon = lon;
    }

    /// Set server information for this tracker
    pub fn set_server_info(&mut self, server_id: String, server_name: String) {
        self.server_id = server_id;
        self.server_name = server_name;
    }

    pub fn set_time_limited_trails(&mut self, enabled: bool) {
        self.time_limited_trails = enabled;
    }

    pub fn get_time_limited_trails(&self) -> bool {
        self.time_limited_trails
    }

    /// Get all aircraft - returns cheap Arc clones
    pub fn get_aircraft(&self) -> Vec<Aircraft> {
        self.aircraft.values().cloned().collect()
    }

    /// Get a specific aircraft by ICAO - returns cheap Arc clone
    pub fn get_aircraft_by_icao(&self, icao: &str) -> Option<Aircraft> {
        self.aircraft.get(icao).cloned()
    }

    pub fn cleanup_old(&mut self, max_age_seconds: i64) {
        let now = Utc::now();

        // Clean up old position history only if time-limited trails are enabled
        if self.time_limited_trails {
            for aircraft in self.aircraft.values() {
                aircraft.cleanup_old_history(TRAIL_HISTORY_SECONDS);
            }
        }

        // Remove aircraft that haven't been seen recently
        self.aircraft.retain(|_, aircraft| {
            (now - aircraft.last_seen()).num_seconds() < max_age_seconds
        });
    }

    pub fn parse_basestation_message(&mut self, line: &str) {
        let parts: Vec<&str> = line.split(',').collect();

        if parts.is_empty() {
            return;
        }

        let msg_type = parts[0];

        // We need at least the ICAO field (index 4)
        if parts.len() < 5 {
            return;
        }

        let icao = parts[4].to_string();
        if icao.is_empty() {
            return;
        }

        let aircraft = self.aircraft.entry(icao.clone()).or_insert_with(|| {
            Aircraft::new(icao, self.server_id.clone(), self.server_name.clone())
        });

        // Update last seen timestamp
        aircraft.with_data_mut(|data| {
            data.last_seen = Utc::now();
        });

        match msg_type {
            "MSG" => {
                if parts.len() < 11 {
                    return;
                }

                let transmission_type = parts[1];

                match transmission_type {
                    "1" => {
                        // Aircraft identification (callsign)
                        if parts.len() > 10 && !parts[10].is_empty() {
                            aircraft.with_data_mut(|data| {
                                data.callsign = Some(parts[10].trim().to_string());
                            });
                        }
                    }
                    "3" => {
                        // Airborne position
                        if parts.len() > 15 {
                            if !parts[11].is_empty() {
                                if let Ok(alt) = parts[11].parse::<i32>() {
                                    aircraft.with_data_mut(|data| {
                                        data.altitude = Some(alt);
                                    });
                                }
                            }
                            if !parts[14].is_empty() && !parts[15].is_empty() {
                                if let (Ok(lat), Ok(lon)) = (parts[14].parse::<f64>(), parts[15].parse::<f64>()) {
                                    let updated = aircraft.update_position(lat, lon, self.center_lat, self.center_lon, self.max_distance_miles);
                                    // Record position update for sparkline tracking
                                    if updated {
                                        if let Some(ref status) = self.status {
                                            status.lock()
                                                .expect("System status lock poisoned - unrecoverable state")
                                                .record_position_update();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "4" => {
                        // Airborne velocity
                        if parts.len() > 13 {
                            aircraft.with_data_mut(|data| {
                                if !parts[12].is_empty() {
                                    if let Ok(speed) = parts[12].parse::<f64>() {
                                        data.velocity = Some(speed);
                                    }
                                }
                                if !parts[13].is_empty() {
                                    if let Ok(track) = parts[13].parse::<f64>() {
                                        data.track = Some(track);
                                    }
                                }
                                if parts.len() > 16 && !parts[16].is_empty() {
                                    if let Ok(vr) = parts[16].parse::<i32>() {
                                        data.vertical_rate = Some(vr);
                                    }
                                }
                            });
                        }
                    }
                    "5" => {
                        // Surveillance altitude
                        if parts.len() > 11 && !parts[11].is_empty() {
                            if let Ok(alt) = parts[11].parse::<i32>() {
                                aircraft.with_data_mut(|data| {
                                    data.altitude = Some(alt);
                                });
                            }
                        }
                    }
                    "6" => {
                        // Surveillance position
                        if parts.len() > 15 {
                            if !parts[11].is_empty() {
                                if let Ok(alt) = parts[11].parse::<i32>() {
                                    aircraft.with_data_mut(|data| {
                                        data.altitude = Some(alt);
                                    });
                                }
                            }
                            if !parts[14].is_empty() && !parts[15].is_empty() {
                                if let (Ok(lat), Ok(lon)) = (parts[14].parse::<f64>(), parts[15].parse::<f64>()) {
                                    let updated = aircraft.update_position(lat, lon, self.center_lat, self.center_lon, self.max_distance_miles);
                                    // Record position update for sparkline tracking
                                    if updated {
                                        if let Some(ref status) = self.status {
                                            status.lock()
                                                .expect("System status lock poisoned - unrecoverable state")
                                                .record_position_update();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "7" => {
                        // Air-to-air message
                        if parts.len() > 11 && !parts[11].is_empty() {
                            if let Ok(alt) = parts[11].parse::<i32>() {
                                aircraft.with_data_mut(|data| {
                                    data.altitude = Some(alt);
                                });
                            }
                        }
                    }
                    "8" => {
                        // All call reply
                        if parts.len() > 11 && !parts[11].is_empty() {
                            if let Ok(alt) = parts[11].parse::<i32>() {
                                aircraft.with_data_mut(|data| {
                                    data.altitude = Some(alt);
                                });
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {
                // Ignore other message types for now
            }
        }
    }
}
