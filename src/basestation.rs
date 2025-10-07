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

use std::collections::HashMap;
use chrono::{DateTime, Utc};

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

#[derive(Debug, Clone)]
pub struct PositionPoint {
    pub lat: f64,
    pub lon: f64,
    pub altitude: Option<i32>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Aircraft {
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
    // Metadata fields
    pub registration: Option<String>,
    pub aircraft_type: Option<String>,
    pub photo_url: Option<String>,
    pub photo_thumbnail_url: Option<String>,
    pub photographer: Option<String>,
    pub metadata_fetched: bool,
}

impl Aircraft {
    pub fn new(icao: String) -> Self {
        Self {
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
            registration: None,
            aircraft_type: None,
            photo_url: None,
            photo_thumbnail_url: None,
            photographer: None,
            metadata_fetched: false,
        }
    }

    pub fn update_position(&mut self, lat: f64, lon: f64, center_lat: f64, center_lon: f64, max_distance: f64) -> bool {
        // Check if position is within max distance from center
        let distance_from_center = haversine_distance(center_lat, center_lon, lat, lon);
        if distance_from_center > max_distance {
            return false; // Position rejected - too far from center
        }

        // Check if position is within 10 miles of previous position
        if let (Some(last_lat), Some(last_lon)) = (self.latitude, self.longitude) {
            let distance_from_last = haversine_distance(last_lat, last_lon, lat, lon);
            if distance_from_last > 10.0 {
                // Position jump too large - reject
                println!("Rejected position for {}: jumped {:.1} miles (max 10 miles allowed)",
                    self.icao, distance_from_last);
                return false;
            }
        }

        // Only add to history if position has changed significantly (> ~100 meters)
        let should_add = if let (Some(last_lat), Some(last_lon)) = (self.latitude, self.longitude) {
            let distance = ((lat - last_lat).powi(2) + (lon - last_lon).powi(2)).sqrt();
            distance > 0.001 // roughly 100 meters at mid-latitudes
        } else {
            true
        };

        if should_add {
            self.position_history.push(PositionPoint {
                lat,
                lon,
                altitude: self.altitude,
                timestamp: Utc::now(),
            });
        }

        self.latitude = Some(lat);
        self.longitude = Some(lon);
        true
    }

    pub fn cleanup_old_history(&mut self, max_age_seconds: i64) {
        let now = Utc::now();
        self.position_history.retain(|point| {
            (now - point.timestamp).num_seconds() < max_age_seconds
        });
    }
}

pub struct AircraftTracker {
    aircraft: HashMap<String, Aircraft>,
    center_lat: f64,
    center_lon: f64,
    max_distance_miles: f64,
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
        }
    }

    pub fn set_center(&mut self, lat: f64, lon: f64) {
        self.center_lat = lat;
        self.center_lon = lon;
    }

    pub fn get_aircraft(&self) -> Vec<&Aircraft> {
        self.aircraft.values().collect()
    }

    pub fn get_aircraft_mut(&mut self, icao: &str) -> Option<&mut Aircraft> {
        self.aircraft.get_mut(icao)
    }

    pub fn cleanup_old(&mut self, max_age_seconds: i64) {
        let now = Utc::now();

        // Clean up old position history for all aircraft
        for aircraft in self.aircraft.values_mut() {
            aircraft.cleanup_old_history(300); // Keep 5 minutes of history
        }

        // Remove aircraft that haven't been seen recently
        self.aircraft.retain(|_, aircraft| {
            (now - aircraft.last_seen).num_seconds() < max_age_seconds
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

        let aircraft = self.aircraft.entry(icao.clone()).or_insert_with(|| Aircraft::new(icao));
        aircraft.last_seen = Utc::now();

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
                            aircraft.callsign = Some(parts[10].trim().to_string());
                        }
                    }
                    "3" => {
                        // Airborne position
                        if parts.len() > 15 {
                            if !parts[11].is_empty() {
                                if let Ok(alt) = parts[11].parse::<i32>() {
                                    aircraft.altitude = Some(alt);
                                }
                            }
                            if !parts[14].is_empty() && !parts[15].is_empty() {
                                if let (Ok(lat), Ok(lon)) = (parts[14].parse::<f64>(), parts[15].parse::<f64>()) {
                                    aircraft.update_position(lat, lon, self.center_lat, self.center_lon, self.max_distance_miles);
                                }
                            }
                        }
                    }
                    "4" => {
                        // Airborne velocity
                        if parts.len() > 13 {
                            if !parts[12].is_empty() {
                                if let Ok(speed) = parts[12].parse::<f64>() {
                                    aircraft.velocity = Some(speed);
                                }
                            }
                            if !parts[13].is_empty() {
                                if let Ok(track) = parts[13].parse::<f64>() {
                                    aircraft.track = Some(track);
                                }
                            }
                            if parts.len() > 16 && !parts[16].is_empty() {
                                if let Ok(vr) = parts[16].parse::<i32>() {
                                    aircraft.vertical_rate = Some(vr);
                                }
                            }
                        }
                    }
                    "5" => {
                        // Surveillance altitude
                        if parts.len() > 11 && !parts[11].is_empty() {
                            if let Ok(alt) = parts[11].parse::<i32>() {
                                aircraft.altitude = Some(alt);
                            }
                        }
                    }
                    "6" => {
                        // Surveillance position
                        if parts.len() > 15 {
                            if !parts[11].is_empty() {
                                if let Ok(alt) = parts[11].parse::<i32>() {
                                    aircraft.altitude = Some(alt);
                                }
                            }
                            if !parts[14].is_empty() && !parts[15].is_empty() {
                                if let (Ok(lat), Ok(lon)) = (parts[14].parse::<f64>(), parts[15].parse::<f64>()) {
                                    aircraft.update_position(lat, lon, self.center_lat, self.center_lon, self.max_distance_miles);
                                }
                            }
                        }
                    }
                    "7" => {
                        // Air-to-air message
                        if parts.len() > 11 && !parts[11].is_empty() {
                            if let Ok(alt) = parts[11].parse::<i32>() {
                                aircraft.altitude = Some(alt);
                            }
                        }
                    }
                    "8" => {
                        // All call reply
                        if parts.len() > 11 && !parts[11].is_empty() {
                            if let Ok(alt) = parts[11].parse::<i32>() {
                                aircraft.altitude = Some(alt);
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
