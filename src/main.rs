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

mod aviation_data;
mod aircraft_db;
mod aircraft_metadata;
mod aircraft_types;
mod basestation;
mod photo_cache;
mod status;
mod status_pane;
mod tcp_client;
mod tiles;

use aircraft_db::AircraftDatabase;
use aircraft_types::AircraftTypeDatabase;
use aircraft_metadata::MetadataService;
use aviation_data::{AviationData, Airport, Navaid};
use basestation::{Aircraft, AircraftTracker};
use clap::Parser;
use eframe::egui;
use photo_cache::PhotoTextureManager;
use status::{SystemStatus, DiagnosticLevel};
use status_pane::StatusPane;
use std::sync::{Arc, Mutex};
use serde::Deserialize;
use tiles::{TileManager, WebMercator};

// Trail display constants
const TRAIL_MAX_AGE_SECONDS: f32 = 300.0;  // 5 minutes total
const TRAIL_SOLID_DURATION_SECONDS: f32 = 225.0;  // First 75% solid (3.75 minutes)
const TRAIL_FADE_DURATION_SECONDS: f32 = 75.0;  // Last 25% fade (1.25 minutes)

/// Validate server address format (host:port)
fn validate_server_address(s: &str) -> Result<String, String> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return Err("Server address must be in format host:port".to_string());
    }

    // Validate port number
    parts[1].parse::<u16>()
        .map_err(|_| "Invalid port number (must be 0-65535)".to_string())?;

    Ok(s.to_string())
}

/// AirJedi Desktop - Real-time ADS-B aircraft tracking application
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CliArgs {
    /// BaseStation/SBS-1 feed address
    #[arg(
        short,
        long,
        default_value = "localhost:30003",
        value_parser = validate_server_address
    )]
    server: String,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct GeoLocation {
    latitude: Option<f64>,
    longitude: Option<f64>,
}

#[cfg(target_os = "macos")]
fn get_gps_location() -> Option<(f64, f64)> {
    use objc2_core_location::{CLLocationManager, CLAuthorizationStatus};
    use std::time::Duration;

    println!("Attempting to get GPS location from CoreLocation...");

    unsafe {
        let manager = CLLocationManager::new();

        // Check authorization status
        let auth_status = manager.authorizationStatus();

        // Request authorization if needed
        if auth_status == CLAuthorizationStatus::NotDetermined {
            println!("Requesting location authorization...");
            manager.requestWhenInUseAuthorization();
            // Give it a moment to process
            std::thread::sleep(Duration::from_millis(500));
        }

        // Start updating location
        manager.startUpdatingLocation();

        // Wait a bit for location update
        std::thread::sleep(Duration::from_secs(2));

        // Get location
        if let Some(location) = manager.location() {
            let coord = location.coordinate();
            let latitude = coord.latitude;
            let longitude = coord.longitude;

            manager.stopUpdatingLocation();

            println!("GPS location found: {}, {}", latitude, longitude);
            return Some((latitude, longitude));
        } else {
            println!("No location available from GPS");
        }

        manager.stopUpdatingLocation();
    }

    None
}

#[cfg(not(target_os = "macos"))]
fn get_gps_location() -> Option<(f64, f64)> {
    None
}

fn get_current_location() -> Option<(f64, f64)> {
    println!("Fetching current location...");

    // Try GPS first (macOS only)
    if let Some(location) = get_gps_location() {
        return Some(location);
    }

    println!("Falling back to IP-based geolocation...");

    // Try ipapi.co first
    if let Ok(response) = reqwest::blocking::get("https://ipapi.co/json/") {
        if let Ok(text) = response.text() {
            // Try to parse as JSON value first to see the structure
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let (Some(lat), Some(lon)) = (
                    value.get("latitude").and_then(|v| v.as_f64()),
                    value.get("longitude").and_then(|v| v.as_f64())
                ) {
                    println!("Location found via ipapi.co: {}, {}", lat, lon);
                    return Some((lat, lon));
                }
            }
        }
    }

    // Fallback to ip-api.com (no API key needed)
    if let Ok(response) = reqwest::blocking::get("http://ip-api.com/json/") {
        if let Ok(text) = response.text() {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
                if let (Some(lat), Some(lon)) = (
                    value.get("lat").and_then(|v| v.as_f64()),
                    value.get("lon").and_then(|v| v.as_f64())
                ) {
                    println!("Location found via ip-api.com: {}, {}", lat, lon);
                    return Some((lat, lon));
                }
            }
        }
    }

    eprintln!("Failed to fetch location from all sources");
    None
}

fn main() -> Result<(), eframe::Error> {
    // Parse command-line arguments
    let args = CliArgs::parse();

    println!("Starting AirJedi Desktop...");
    println!("Connecting to SBS-1 server at: {}", args.server);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 800.0])
            .with_title("AirJedi Desktop"),
        ..Default::default()
    };

    println!("Initializing window...");
    let server_address = args.server.clone();
    eframe::run_native(
        "AirJedi Desktop",
        options,
        Box::new(move |_cc| {
            println!("Creating application...");
            Ok(Box::new(AdsbApp::new(server_address)))
        }),
    )
}

// Generic trait for map items that can show hover popups
trait MapItemPopup {
    fn render_popup(&self, ui: &mut egui::Ui, receiver_lat: f64, receiver_lon: f64, aircraft_types: &Arc<Mutex<AircraftTypeDatabase>>);
}

// Enum to hold any hovered map item (extensible for future items)
#[derive(Clone)]
enum HoveredMapItem {
    Airport(Airport),
    Navaid(Navaid),
    Aircraft(Aircraft),
}

// Implement popup rendering for Airport
impl MapItemPopup for Airport {
    fn render_popup(&self, ui: &mut egui::Ui, _receiver_lat: f64, _receiver_lon: f64, _aircraft_types: &Arc<Mutex<AircraftTypeDatabase>>) {
        ui.set_min_width(200.0);

        // ICAO header with color based on airport type
        let header_color = if self.is_major() {
            egui::Color32::from_rgb(255, 120, 120) // Red for large
        } else if self.is_medium() {
            egui::Color32::from_rgb(255, 200, 100) // Orange for medium
        } else {
            egui::Color32::from_rgb(180, 180, 255) // Light blue for small
        };

        ui.label(egui::RichText::new(&self.icao)
            .color(header_color)
            .size(16.0)
            .strong());

        // Airport name
        ui.label(egui::RichText::new(&self.name)
            .color(egui::Color32::from_rgb(220, 220, 220))
            .size(11.0));

        ui.add_space(4.0);

        // Type badge
        let (type_text, type_color) = if self.is_major() {
            ("Large Airport", egui::Color32::from_rgb(255, 100, 100))
        } else if self.is_medium() {
            ("Medium Airport", egui::Color32::from_rgb(255, 180, 80))
        } else {
            ("Small Airport", egui::Color32::from_rgb(150, 150, 200))
        };

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("●")
                .color(type_color)
                .size(10.0));
            ui.label(egui::RichText::new(type_text)
                .color(type_color)
                .size(10.0));
        });

        // Elevation
        if let Some(elevation) = self.elevation {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Elevation:")
                    .color(egui::Color32::from_rgb(150, 150, 150))
                    .size(9.0));
                ui.label(egui::RichText::new(format!("{} ft", elevation))
                    .color(egui::Color32::from_rgb(200, 200, 200))
                    .size(9.0));
            });
        }

        // Scheduled service
        if self.has_scheduled_service() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("✈")
                    .color(egui::Color32::from_rgb(100, 200, 100))
                    .size(10.0));
                ui.label(egui::RichText::new("Scheduled Service")
                    .color(egui::Color32::from_rgb(100, 200, 100))
                    .size(9.0));
            });
        }

        ui.add_space(2.0);

        // Coordinates (subtle)
        ui.label(egui::RichText::new(format!("{:.4}°, {:.4}°", self.latitude, self.longitude))
            .color(egui::Color32::from_rgb(120, 120, 120))
            .size(8.0));
    }
}

// Implement popup rendering for Navaid
impl MapItemPopup for Navaid {
    fn render_popup(&self, ui: &mut egui::Ui, _receiver_lat: f64, _receiver_lon: f64, _aircraft_types: &Arc<Mutex<AircraftTypeDatabase>>) {
        ui.set_min_width(180.0);

        // Ident header with color based on navaid type
        let (r, g, b) = self.get_color();
        let header_color = egui::Color32::from_rgb(r, g, b);

        ui.label(egui::RichText::new(&self.ident)
            .color(header_color)
            .size(16.0)
            .strong());

        // Navaid name
        ui.label(egui::RichText::new(&self.name)
            .color(egui::Color32::from_rgb(220, 220, 220))
            .size(11.0));

        ui.add_space(4.0);

        // Type badge
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("▲")
                .color(header_color)
                .size(10.0));
            ui.label(egui::RichText::new(&self.navaid_type)
                .color(header_color)
                .size(10.0));
        });

        // Frequency
        if let Some(freq_khz) = self.frequency_khz {
            let freq_mhz = freq_khz as f32 / 1000.0;
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Frequency:")
                    .color(egui::Color32::from_rgb(150, 150, 150))
                    .size(9.0));
                ui.label(egui::RichText::new(format!("{:.3} MHz", freq_mhz))
                    .color(egui::Color32::from_rgb(200, 200, 200))
                    .size(9.0));
            });
        }

        ui.add_space(2.0);

        // Coordinates (subtle)
        ui.label(egui::RichText::new(format!("{:.4}°, {:.4}°", self.latitude, self.longitude))
            .color(egui::Color32::from_rgb(120, 120, 120))
            .size(8.0));
    }
}

// Implement popup rendering for Aircraft
impl MapItemPopup for Aircraft {
    fn render_popup(&self, ui: &mut egui::Ui, receiver_lat: f64, receiver_lon: f64, aircraft_types: &Arc<Mutex<AircraftTypeDatabase>>) {
        ui.set_min_width(220.0);

        // Calculate range from receiver
        let range_nm = self.distance_from_nm(receiver_lat, receiver_lon);

        self.with_data(|data| {
            // Callsign or ICAO as header
            if let Some(ref callsign) = data.callsign {
                ui.label(egui::RichText::new(callsign.trim())
                    .color(egui::Color32::from_rgb(100, 255, 100))
                    .size(16.0)
                    .strong());

                // ICAO as subtitle
                ui.label(egui::RichText::new(&data.icao)
                    .color(egui::Color32::from_rgb(180, 180, 180))
                    .size(10.0)
                    .monospace());
            } else {
                // Just ICAO if no callsign
                ui.label(egui::RichText::new(&data.icao)
                    .color(egui::Color32::from_rgb(100, 255, 100))
                    .size(16.0)
                    .strong()
                    .monospace());
            }

            ui.add_space(4.0);

            // Altitude with color coding
            if let Some(alt) = data.altitude {
                let (r, g, b) = AdsbApp::altitude_to_color(Some(alt));
                let alt_color = egui::Color32::from_rgb(r, g, b);

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("▲")
                        .color(alt_color)
                        .size(10.0));
                    let alt_text = if alt >= 18000 {
                        format!("FL{:03}", alt / 100)
                    } else {
                        format!("{} ft", alt)
                    };
                    ui.label(egui::RichText::new(alt_text)
                        .color(alt_color)
                        .size(10.0)
                        .monospace());
                });
            }

            // Velocity/Speed
            if let Some(vel) = data.velocity {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Speed:")
                        .color(egui::Color32::from_rgb(150, 150, 150))
                        .size(9.0));
                    ui.label(egui::RichText::new(format!("{} kts", vel as i32))
                        .color(egui::Color32::from_rgb(200, 200, 200))
                        .size(9.0)
                        .monospace());
                });
            }

            // Track/Heading
            if let Some(track) = data.track {
                let heading_indicator = match track as i32 {
                    0..=22 | 338..=360 => "N",
                    23..=67 => "NE",
                    68..=112 => "E",
                    113..=157 => "SE",
                    158..=202 => "S",
                    203..=247 => "SW",
                    248..=292 => "W",
                    293..=337 => "NW",
                    _ => "?",
                };

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Heading:")
                        .color(egui::Color32::from_rgb(150, 150, 150))
                        .size(9.0));
                    ui.label(egui::RichText::new(format!("{:03}° {}", track as i32, heading_indicator))
                        .color(egui::Color32::from_rgb(200, 200, 200))
                        .size(9.0)
                        .monospace());
                });
            }

            // Vertical rate with climbing/descending indicator
            if let Some(vr) = data.vertical_rate {
                let (indicator, vr_color) = if vr > 100 {
                    ("↑", egui::Color32::from_rgb(100, 255, 100)) // Climbing - green
                } else if vr < -100 {
                    ("↓", egui::Color32::from_rgb(255, 150, 100)) // Descending - orange
                } else {
                    ("→", egui::Color32::from_rgb(150, 150, 150)) // Level - gray
                };

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(indicator)
                        .color(vr_color)
                        .size(11.0));
                    ui.label(egui::RichText::new(format!("{:+} ft/min", vr))
                        .color(vr_color)
                        .size(9.0)
                        .monospace());
                });
            }

            ui.add_space(2.0);

            // Aircraft type
            if let Some(ref aircraft_type) = data.aircraft_type {
                // Lookup full aircraft type name from type database
                let type_display = if let Ok(type_db) = aircraft_types.lock() {
                    type_db.lookup(aircraft_type)
                        .unwrap_or(aircraft_type.as_str())
                        .to_string()
                } else {
                    aircraft_type.clone()
                };

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Type:")
                        .color(egui::Color32::from_rgb(150, 150, 150))
                        .size(9.0));
                    ui.label(egui::RichText::new(type_display)
                        .color(egui::Color32::from_rgb(180, 150, 200))
                        .size(9.0));
                });
            }

            // Range from receiver
            if let Some(range) = range_nm {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Range:")
                        .color(egui::Color32::from_rgb(150, 150, 150))
                        .size(9.0));
                    ui.label(egui::RichText::new(format!("{:.1} nm", range))
                        .color(egui::Color32::from_rgb(100, 200, 255))
                        .size(9.0)
                        .monospace());
                });
            }

            // Position coordinates
            if let (Some(lat), Some(lon)) = (data.latitude, data.longitude) {
                ui.label(egui::RichText::new(format!("{:.4}°, {:.4}°", lat, lon))
                    .color(egui::Color32::from_rgb(120, 120, 120))
                    .size(8.0));
            }

            // Last seen
            let seconds_ago = (chrono::Utc::now() - data.last_seen).num_seconds();
            let time_color = if seconds_ago < 5 {
                egui::Color32::from_rgb(100, 255, 100) // Recent - green
            } else if seconds_ago < 30 {
                egui::Color32::from_rgb(255, 200, 100) // Moderate - yellow
            } else {
                egui::Color32::from_rgb(150, 150, 150) // Old - gray
            };

            ui.label(egui::RichText::new(format!("Updated {}s ago", seconds_ago))
                .color(time_color)
                .size(8.0));
        });
    }
}

/// Startup sequence states
#[derive(Debug, Clone, Copy, PartialEq)]
enum StartupState {
    InitializingWindow,
    DetectingLocation,
    StartingTcpClient,
    LoadingAviationData,
    LoadingAircraftDB,
    Complete,
}

struct AdsbApp {
    tracker: Arc<Mutex<AircraftTracker>>,
    map_center_lat: f64,
    map_center_lon: f64,
    receiver_lat: f64,
    receiver_lon: f64,
    map_zoom_level: f32, // Float for smoother pinch-zoom
    tile_manager: TileManager,
    tile_error: Option<String>,
    selected_aircraft: Option<String>, // ICAO of selected aircraft
    previous_selected_aircraft: Option<String>, // Track selection changes for auto-scroll
    aviation_data: Arc<Mutex<AviationData>>,
    aviation_data_loading: Arc<Mutex<bool>>,
    show_airports: bool,
    show_runways: bool,
    show_navaids: bool,
    time_limited_trails: bool,
    airport_filter: AirportFilter,
    // Cached bounding box for spatial filtering
    cached_bounds: Option<(f64, f64, f64, f64)>, // (min_lat, max_lat, min_lon, max_lon)
    last_bounds_zoom: f32,
    last_bounds_center: (f64, f64),
    // Cached aviation data to avoid cloning thousands of objects every frame
    cached_aviation_data: Option<(Vec<Airport>, Vec<(String, Vec<aviation_data::Runway>)>, Vec<Navaid>)>,
    last_aviation_cache_bounds: Option<(f64, f64, f64, f64)>,
    last_aviation_cache_filter: AirportFilter,
    // Hover popup state
    hovered_map_item: Option<HoveredMapItem>,
    // Aircraft metadata
    aircraft_db: Arc<Mutex<AircraftDatabase>>,
    aircraft_types: Arc<Mutex<AircraftTypeDatabase>>,
    metadata_service: Arc<MetadataService>,
    pending_metadata: Arc<Mutex<std::collections::HashSet<String>>>, // Track aircraft being fetched
    photo_manager: PhotoTextureManager,
    // System status and monitoring
    system_status: Arc<Mutex<SystemStatus>>,
    status_pane: StatusPane,
    // Startup sequence tracking
    startup_state: StartupState,
    startup_frame_count: usize,
    // Aircraft list filtering and sorting
    sort_by: SortCriterion,
    sort_direction: SortDirection,
    filters_enabled: bool,
    filter_altitude_min: f32,
    filter_altitude_max: f32,
    filter_speed_min: f32,
    filter_speed_max: f32,
    filter_range_min: f32,
    filter_range_max: f32,
    filter_registration: String,
    filter_icao: String,
    // Auto-pan to selected aircraft
    stored_map_center: Option<(f64, f64)>, // (lat, lon) before auto-pan
    following_aircraft: bool, // Whether we've auto-panned to an aircraft
    // SBS-1 server configuration
    server_address: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AirportFilter {
    All,              // Show all airplane airports (large, medium, small)
    FrequentlyUsed,   // Show airports with scheduled service or large/medium
    MajorOnly,        // Show only large airports
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SortCriterion {
    Range,      // Sort by distance from receiver
    Speed,      // Sort by ground speed
    Altitude,   // Sort by altitude
}

impl Default for SortCriterion {
    fn default() -> Self {
        SortCriterion::Altitude
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SortDirection {
    Ascending,
    Descending,
}

impl Default for SortDirection {
    fn default() -> Self {
        SortDirection::Descending
    }
}

impl AdsbApp {
    // Draw an airplane icon at the given position with rotation based on track angle
    fn draw_aircraft_icon(
        painter: &egui::Painter,
        pos: egui::Pos2,
        track_degrees: f32,
        color: egui::Color32,
        size: f32,
    ) {
        // Define airplane shape vertices relative to center (pointing north/up by default)
        // Vertices in (x, y) format where y is negative for forward
        let base_vertices = [
            (0.0, -1.5),      // Nose (front)
            (-0.3, -0.5),     // Left side of fuselage
            (-1.0, 0.0),      // Left wing tip
            (-0.3, 0.2),      // Left wing back
            (-0.4, 0.8),      // Left tail
            (-0.2, 0.9),      // Left tail inner
            (0.0, 0.7),       // Center tail
            (0.2, 0.9),       // Right tail inner
            (0.4, 0.8),       // Right tail
            (0.3, 0.2),       // Right wing back
            (1.0, 0.0),       // Right wing tip
            (0.3, -0.5),      // Right side of fuselage
        ];

        // Convert track to radians (track is in degrees, 0 = north)
        let angle = track_degrees.to_radians();
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        // Rotate and scale vertices, then translate to position
        let points: Vec<egui::Pos2> = base_vertices
            .iter()
            .map(|(x, y)| {
                // Scale
                let sx = x * size;
                let sy = y * size;
                // Rotate
                let rx = sx * cos_a - sy * sin_a;
                let ry = sx * sin_a + sy * cos_a;
                // Translate to position
                egui::pos2(pos.x + rx, pos.y + ry)
            })
            .collect();

        // Draw filled airplane shape with no outline
        painter.add(egui::Shape::convex_polygon(
            points,
            color,
            egui::Stroke::NONE,
        ));
    }

    // Convert altitude to continuous color gradient
    // Low altitude (cyan) -> High altitude (purple) with smooth blending
    fn altitude_to_color(altitude_ft: Option<i32>) -> (u8, u8, u8) {
        let alt = altitude_ft.unwrap_or(0) as f32;

        // Clamp altitude to 0-45000 range for gradient calculation
        let clamped_alt = alt.clamp(0.0, 45000.0);

        // Define gradient stops with colors (in feet)
        // Each stop is (altitude, (r, g, b))
        let stops = [
            (0.0, (0.0, 200.0, 200.0)),        // Cyan
            (10000.0, (50.0, 150.0, 200.0)),   // Teal
            (20000.0, (150.0, 200.0, 0.0)),    // Yellow
            (30000.0, (255.0, 150.0, 0.0)),    // Orange
            (40000.0, (255.0, 50.0, 150.0)),   // Red/Magenta
            (45000.0, (150.0, 50.0, 255.0)),   // Purple
        ];

        // Find which two stops we're between
        for i in 0..stops.len() - 1 {
            let (alt1, color1) = stops[i];
            let (alt2, color2) = stops[i + 1];

            if clamped_alt >= alt1 && clamped_alt <= alt2 {
                // Linear interpolation between the two colors
                let t = (clamped_alt - alt1) / (alt2 - alt1);

                let r = color1.0 + (color2.0 - color1.0) * t;
                let g = color1.1 + (color2.1 - color1.1) * t;
                let b = color1.2 + (color2.2 - color1.2) * t;

                return (r as u8, g as u8, b as u8);
            }
        }

        // Fallback to highest color if somehow we didn't match
        (150, 50, 255)
    }

    fn new(server_address: String) -> Self {
        println!("Initializing ADSB app...");

        // Initialize core structures
        let tracker = Arc::new(Mutex::new(AircraftTracker::new()));
        let system_status = Arc::new(Mutex::new(SystemStatus::new()));
        let aviation_data = Arc::new(Mutex::new(AviationData::new()));
        let aviation_data_loading = Arc::new(Mutex::new(true));
        let aircraft_db = Arc::new(Mutex::new(AircraftDatabase::new()));
        let aircraft_types = Arc::new(Mutex::new(AircraftTypeDatabase::new()));
        let metadata_service = Arc::new(MetadataService::new());
        let photo_manager = PhotoTextureManager::new();

        // Load aircraft type database from CSV file
        if let Err(e) = aircraft_types.lock().unwrap().load_from_file("data/aircraft.csv") {
            eprintln!("Warning: Failed to load aircraft types: {}", e);
        }

        // Wire up status tracking in the tracker for position update sparkline
        tracker.lock().unwrap().set_status(system_status.clone());

        // Use default location initially - will be updated during startup sequence
        let default_lat = 37.7749;
        let default_lon = -122.4194;

        // Add initial startup diagnostic
        system_status.lock().unwrap().add_diagnostic(
            DiagnosticLevel::Info,
            "Starting AirJedi Desktop...".to_string()
        );

        println!("App structure initialized - startup will continue in first frames");

        Self {
            tracker,
            map_center_lat: default_lat,
            map_center_lon: default_lon,
            receiver_lat: default_lat,
            receiver_lon: default_lon,
            map_zoom_level: 8.0, // Zoom level 8 ≈ 150 mile range
            tile_manager: TileManager::new(),
            tile_error: None,
            selected_aircraft: None,
            previous_selected_aircraft: None,
            aviation_data,
            aviation_data_loading,
            show_airports: true,
            show_runways: true,
            show_navaids: false, // Off by default since there are many navaids
            time_limited_trails: false, // Off by default - show full history trails
            airport_filter: AirportFilter::FrequentlyUsed, // Default to frequently used
            cached_bounds: None,
            last_bounds_zoom: 0.0,
            last_bounds_center: (0.0, 0.0),
            cached_aviation_data: None,
            last_aviation_cache_bounds: None,
            last_aviation_cache_filter: AirportFilter::FrequentlyUsed,
            hovered_map_item: None,
            aircraft_db,
            aircraft_types,
            metadata_service,
            pending_metadata: Arc::new(Mutex::new(std::collections::HashSet::new())),
            photo_manager,
            system_status,
            status_pane: StatusPane::new(),
            startup_state: StartupState::InitializingWindow,
            startup_frame_count: 0,
            // Initialize filtering and sorting with sensible defaults
            sort_by: SortCriterion::default(),
            sort_direction: SortDirection::default(),
            filters_enabled: false,
            filter_altitude_min: 0.0,
            filter_altitude_max: 50000.0,
            filter_speed_min: 0.0,
            filter_speed_max: 600.0,
            filter_range_min: 0.0,
            filter_range_max: 400.0,
            filter_registration: String::new(),
            filter_icao: String::new(),
            // Auto-pan state
            stored_map_center: None,
            following_aircraft: false,
            server_address,
        }
    }

    /// Fetch metadata for an aircraft in the background
    fn fetch_aircraft_metadata(&self, icao: String) {
        // Check if already pending
        {
            let mut pending = self.pending_metadata.lock().unwrap();
            if pending.contains(&icao) {
                return; // Already fetching
            }
            pending.insert(icao.clone());
        }

        let aircraft_db = self.aircraft_db.clone();
        let metadata_service = self.metadata_service.clone();
        let tracker = self.tracker.clone();
        let pending_metadata = self.pending_metadata.clone();

        // Spawn background thread with its own tokio runtime
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                // First, lookup registration and aircraft type from database
                let (registration, aircraft_type) = {
                    let db = aircraft_db.lock().unwrap();
                    (db.get_registration(&icao), db.get_aircraft_type(&icao))
                };

                // Fetch photo - try by registration first, then by ICAO
                let photo_metadata = if let Some(ref reg) = registration {
                    metadata_service.fetch_photo_by_registration(reg).await
                } else {
                    metadata_service.fetch_photo_by_icao(&icao).await
                };

                // Update aircraft with metadata using the new API
                if let Ok(tracker) = tracker.lock() {
                    if let Some(aircraft) = tracker.get_aircraft_by_icao(&icao) {
                        aircraft.with_data_mut(|data| {
                            data.registration = registration;
                            data.aircraft_type = aircraft_type;
                            data.metadata_fetched = true;

                            if let Some(metadata) = photo_metadata {
                                data.photo_url = metadata.photo_url;
                                data.photo_thumbnail_url = metadata.photo_thumbnail_url;
                                data.photographer = metadata.photographer;
                            }
                        });
                    }
                }

                // Remove from pending
                pending_metadata.lock().unwrap().remove(&icao);
            });
        });
    }

    fn draw_aircraft_list(&mut self, ui: &mut egui::Ui) {
        // Get aircraft list with cheap Arc clones - no expensive deep copying!
        let aircraft_data: Vec<Aircraft> = {
            let tracker = self.tracker.lock()
                .expect("Aircraft tracker mutex poisoned");
            tracker.get_aircraft()  // Returns Vec<Aircraft> where Aircraft is Arc<RwLock<...>>
        };

        let total_count = aircraft_data.len();

        // Military-style header
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("◈ CONTACT LIST")
                    .color(egui::Color32::from_rgb(100, 200, 100))
                    .size(14.0)
                    .strong());
            });
        });

        ui.add_space(2.0);

        // Filters & Sort Controls
        egui::CollapsingHeader::new(egui::RichText::new("⚙ FILTERS & SORT")
            .color(egui::Color32::from_rgb(100, 200, 200))
            .size(11.0)
            .strong())
            .default_open(false)
            .show(ui, |ui| {
                // Enable/Disable filters toggle
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.filters_enabled, "");
                    ui.label(egui::RichText::new("Enable Filters")
                        .color(egui::Color32::from_rgb(180, 180, 180))
                        .size(10.0));
                });

                ui.add_space(4.0);

                // Altitude filter
                ui.label(egui::RichText::new("Altitude (ft)")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(9.0));
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.filter_altitude_min, 0.0..=50000.0)
                        .text("Min")
                        .show_value(true));
                });
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.filter_altitude_max, 0.0..=50000.0)
                        .text("Max")
                        .show_value(true));
                });

                ui.add_space(2.0);

                // Speed filter
                ui.label(egui::RichText::new("Speed (kts)")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(9.0));
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.filter_speed_min, 0.0..=600.0)
                        .text("Min")
                        .show_value(true));
                });
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.filter_speed_max, 0.0..=600.0)
                        .text("Max")
                        .show_value(true));
                });

                ui.add_space(2.0);

                // Range filter
                ui.label(egui::RichText::new("Range (nm)")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(9.0));
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.filter_range_min, 0.0..=400.0)
                        .text("Min")
                        .show_value(true));
                });
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut self.filter_range_max, 0.0..=400.0)
                        .text("Max")
                        .show_value(true));
                });

                ui.add_space(2.0);

                // ICAO filter
                ui.label(egui::RichText::new("ICAO")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(9.0));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.filter_icao)
                        .hint_text("e.g., A1234")
                        .desired_width(150.0));
                    if !self.filter_icao.is_empty() {
                        if ui.small_button("✖").clicked() {
                            self.filter_icao.clear();
                        }
                    }
                });

                ui.add_space(2.0);

                // Registration filter
                ui.label(egui::RichText::new("Registration")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(9.0));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.filter_registration)
                        .hint_text("e.g., N12345")
                        .desired_width(150.0));
                    if !self.filter_registration.is_empty() {
                        if ui.small_button("✖").clicked() {
                            self.filter_registration.clear();
                        }
                    }
                });

                ui.add_space(4.0);
                ui.separator();
                ui.add_space(2.0);

                // Sort by
                ui.label(egui::RichText::new("Sort By")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(9.0));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.sort_by, SortCriterion::Altitude, "Altitude");
                    ui.radio_value(&mut self.sort_by, SortCriterion::Speed, "Speed");
                    ui.radio_value(&mut self.sort_by, SortCriterion::Range, "Range");
                });

                // Sort direction
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("Direction:")
                        .color(egui::Color32::from_rgb(150, 200, 200))
                        .size(9.0));
                    if ui.button(match self.sort_direction {
                        SortDirection::Ascending => "↑ Ascending",
                        SortDirection::Descending => "↓ Descending",
                    }).clicked() {
                        self.sort_direction = match self.sort_direction {
                            SortDirection::Ascending => SortDirection::Descending,
                            SortDirection::Descending => SortDirection::Ascending,
                        };
                    }
                });

                ui.add_space(2.0);

                // Reset filters button
                if ui.button("Reset Filters").clicked() {
                    self.filters_enabled = false;
                    self.filter_altitude_min = 0.0;
                    self.filter_altitude_max = 50000.0;
                    self.filter_speed_min = 0.0;
                    self.filter_speed_max = 600.0;
                    self.filter_range_min = 0.0;
                    self.filter_range_max = 400.0;
                    self.filter_registration.clear();
                    self.filter_icao.clear();
                }
            });

        ui.add_space(4.0);

        // Apply filtering if enabled
        let mut aircraft_list: Vec<&Aircraft> = if self.filters_enabled {
            aircraft_data.iter().filter(|aircraft| {
                // Altitude filter
                let alt_ok = if let Some(alt) = aircraft.altitude() {
                    alt as f32 >= self.filter_altitude_min && alt as f32 <= self.filter_altitude_max
                } else {
                    false // Exclude aircraft without altitude data when filtering
                };

                // Speed filter
                let speed_ok = if let Some(vel) = aircraft.velocity() {
                    vel as f32 >= self.filter_speed_min && vel as f32 <= self.filter_speed_max
                } else {
                    false // Exclude aircraft without speed data when filtering
                };

                // Range filter
                let range_ok = if let Some(range) = aircraft.distance_from_nm(self.receiver_lat, self.receiver_lon) {
                    range as f32 >= self.filter_range_min && range as f32 <= self.filter_range_max
                } else {
                    false // Exclude aircraft without position data when filtering
                };

                // ICAO filter (case-insensitive substring match)
                let icao_ok = if self.filter_icao.is_empty() {
                    true // No filter applied
                } else {
                    aircraft.icao().to_lowercase().contains(&self.filter_icao.to_lowercase())
                };

                // Registration filter (case-insensitive substring match)
                let registration_ok = if self.filter_registration.is_empty() {
                    true // No filter applied
                } else {
                    if let Some(reg) = aircraft.registration() {
                        reg.to_lowercase().contains(&self.filter_registration.to_lowercase())
                    } else {
                        false // Exclude aircraft without registration when filtering by it
                    }
                };

                alt_ok && speed_ok && range_ok && icao_ok && registration_ok
            }).collect()
        } else {
            aircraft_data.iter().collect()
        };

        let filtered_count = aircraft_list.len();

        // Apply dynamic sorting based on sort criterion and direction
        aircraft_list.sort_unstable_by(|a, b| {
            let ordering = match self.sort_by {
                SortCriterion::Altitude => {
                    let a_alt = a.altitude().unwrap_or(0);
                    let b_alt = b.altitude().unwrap_or(0);
                    a_alt.cmp(&b_alt)
                }
                SortCriterion::Speed => {
                    let a_speed = a.velocity().unwrap_or(0.0);
                    let b_speed = b.velocity().unwrap_or(0.0);
                    a_speed.partial_cmp(&b_speed).unwrap_or(std::cmp::Ordering::Equal)
                }
                SortCriterion::Range => {
                    let a_range = a.distance_from_nm(self.receiver_lat, self.receiver_lon).unwrap_or(f64::MAX);
                    let b_range = b.distance_from_nm(self.receiver_lat, self.receiver_lon).unwrap_or(f64::MAX);
                    a_range.partial_cmp(&b_range).unwrap_or(std::cmp::Ordering::Equal)
                }
            };

            // Apply sort direction
            match self.sort_direction {
                SortDirection::Ascending => ordering,
                SortDirection::Descending => ordering.reverse(),
            }
        });

        // Display count with filter status
        ui.horizontal(|ui| {
            if self.filters_enabled && filtered_count < total_count {
                ui.label(egui::RichText::new(format!("SHOWING: {} of {}", filtered_count, total_count))
                    .color(egui::Color32::from_rgb(255, 200, 100))
                    .size(10.0)
                    .monospace());
            } else {
                ui.label(egui::RichText::new(format!("TOTAL: {}", total_count))
                    .color(egui::Color32::from_rgb(150, 150, 150))
                    .size(10.0)
                    .monospace());
            }
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.push_id("aircraft_list", |ui| {
                for aircraft in aircraft_list {
                    // Trigger metadata fetch if not yet fetched
                    if !aircraft.metadata_fetched() {
                        self.fetch_aircraft_metadata(aircraft.icao());
                    }

                    // Determine status color based on altitude and recency
                    let seconds_ago = (chrono::Utc::now() - aircraft.last_seen()).num_seconds();
                    let (status_color, status_symbol) = if seconds_ago < 10 {
                        (egui::Color32::from_rgb(100, 255, 100), "●") // Active - green
                    } else if seconds_ago < 60 {
                        (egui::Color32::from_rgb(255, 200, 50), "●") // Recent - amber
                    } else {
                        (egui::Color32::from_rgb(150, 150, 150), "○") // Stale - grey
                    };

                    // Altitude-based threat level
                    let (alt_color, alt_indicator) = match aircraft.altitude() {
                        Some(alt) if alt >= 30000 => (egui::Color32::from_rgb(200, 100, 255), "▲"), // High - purple
                        Some(alt) if alt >= 20000 => (egui::Color32::from_rgb(255, 150, 50), "▲"),  // Medium-high - orange
                        Some(alt) if alt >= 10000 => (egui::Color32::from_rgb(200, 200, 100), "▲"), // Medium - yellow
                        Some(_) => (egui::Color32::from_rgb(100, 200, 200), "▼"),                    // Low - cyan
                        None => (egui::Color32::from_rgb(100, 100, 100), "─"),                       // Unknown - grey
                    };

                    // Check if this aircraft is selected
                    let icao = aircraft.icao();
                    let is_selected = self.selected_aircraft.as_ref() == Some(&icao);

                    // Create a frame with background color if selected
                    let frame = if is_selected {
                        egui::Frame::group(ui.style())
                            .fill(egui::Color32::from_rgba_unmultiplied(100, 140, 180, 26)) // 10% opaque
                    } else {
                        egui::Frame::group(ui.style())
                    };

                    let inner_response = frame.show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // Photo thumbnail on the left
                            let texture = if let Some(ref photo_url) = aircraft.photo_thumbnail_url() {
                                self.photo_manager.get_or_load_texture(ui.ctx(), &photo_url, &icao)
                            } else {
                                None
                            };

                            if let Some(tex) = texture {
                                ui.image((tex.id(), egui::vec2(48.0, 32.0)));
                            } else if let Some(placeholder) = self.photo_manager.get_placeholder() {
                                ui.image((placeholder.id(), egui::vec2(48.0, 32.0)));
                            } else {
                                // Fallback: empty space
                                ui.add_space(48.0);
                            }

                            ui.add_space(4.0);

                            // Right side: all aircraft info
                            ui.vertical(|ui| {
                                // Status line with ICAO and callsign
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(status_symbol)
                                        .color(status_color)
                                        .size(12.0));

                                    ui.label(egui::RichText::new(&icao)
                                        .color(egui::Color32::from_rgb(200, 220, 255))
                                        .size(11.0)
                                        .monospace()
                                        .strong());

                                    if let Some(ref callsign) = aircraft.callsign() {
                                        let callsign_color = if is_selected {
                                            egui::Color32::from_rgb(255, 50, 50) // Bright red when selected
                                        } else {
                                            egui::Color32::from_rgb(150, 220, 150) // Green when not selected
                                        };
                                        ui.label(egui::RichText::new(format!("│ {}", callsign.trim()))
                                            .color(callsign_color)
                                            .size(11.0)
                                            .strong());
                                    }

                                    // Altitude indicator on the right
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        if let Some(alt) = aircraft.altitude() {
                                            let alt_text = if alt >= 18000 {
                                                format!("{} FL{:03}", alt_indicator, alt / 100)
                                            } else {
                                                format!("{} {} ft", alt_indicator, alt)
                                            };
                                            ui.label(egui::RichText::new(alt_text)
                                                .color(alt_color)
                                                .size(10.0)
                                                .monospace());
                                        }
                                    });
                                });

                                // Data grid - compact military style
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;

                                    // Speed
                                    if let Some(vel) = aircraft.velocity() {
                                        ui.label(egui::RichText::new(format!("SPD {:03}", vel as i32))
                                            .color(egui::Color32::from_rgb(180, 180, 180))
                                            .size(9.0)
                                            .monospace());
                                    }

                                    // Track/Heading
                                    if let Some(track) = aircraft.track() {
                                        ui.label(egui::RichText::new(format!("HDG {:03}°", track as i32))
                                            .color(egui::Color32::from_rgb(180, 180, 180))
                                            .size(9.0)
                                            .monospace());
                                    }

                                    // Range from receiver
                                    if let Some(range) = aircraft.distance_from_nm(self.receiver_lat, self.receiver_lon) {
                                        ui.label(egui::RichText::new(format!("RNG {:.1}", range))
                                            .color(egui::Color32::from_rgb(100, 200, 255))
                                            .size(9.0)
                                            .monospace());
                                    }
                                });

                                // Position coordinates - dim
                                if let (Some(lat), Some(lon)) = (aircraft.latitude(), aircraft.longitude()) {
                                    ui.label(egui::RichText::new(format!("{:>7.3}° {:>8.3}°", lat, lon))
                                        .color(egui::Color32::from_rgb(120, 120, 120))
                                        .size(8.5)
                                        .monospace());
                                }

                                // Registration and Aircraft Type
                                ui.horizontal(|ui| {
                                    if let Some(ref registration) = aircraft.registration() {
                                        ui.label(egui::RichText::new(format!("REG: {}", registration))
                                            .color(egui::Color32::from_rgb(150, 180, 200))
                                            .size(8.5)
                                            .monospace());
                                    }
                                    if let Some(ref aircraft_type) = aircraft.aircraft_type() {
                                        // Lookup full aircraft type name from type database
                                        let type_display = if let Ok(type_db) = self.aircraft_types.lock() {
                                            type_db.lookup(aircraft_type)
                                                .unwrap_or(aircraft_type.as_str())
                                                .to_string()
                                        } else {
                                            aircraft_type.clone()
                                        };

                                        ui.label(egui::RichText::new(format!("TYPE: {}", type_display))
                                            .color(egui::Color32::from_rgb(180, 150, 200))
                                            .size(8.5)
                                            .monospace());
                                    }
                                });

                                // Last seen timestamp
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(format!("T-{:03}s", seconds_ago))
                                        .color(egui::Color32::from_rgb(100, 100, 100))
                                        .size(8.0)
                                        .monospace());
                                });
                            }); // Close vertical layout
                        }); // Close horizontal layout with photo
                    });

                    // Make the entire frame area clickable
                    let response = ui.interact(
                        inner_response.response.rect,
                        ui.id().with(&icao),
                        egui::Sense::click()
                    );

                    // Handle click to select this aircraft
                    if response.clicked() {
                        self.selected_aircraft = Some(icao.clone());
                    }

                    // Auto-scroll to selected aircraft if it's a new selection
                    if is_selected && self.previous_selected_aircraft.as_ref() != Some(&icao) {
                        response.scroll_to_me(Some(egui::Align::Center));
                    }

                    ui.add_space(3.0);
                }
            });
        });
    }

    fn draw_map(&mut self, ui: &mut egui::Ui) {
        // Allocate space for the map
        let (response, painter) = ui.allocate_painter(
            egui::vec2(ui.available_width(), ui.available_height()),
            egui::Sense::click_and_drag(),
        );

        let rect = response.rect;
        let center = rect.center();

        // Draw background - black to blend with dark map tiles during loading
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(0, 0, 0));

        // Auto-pan to newly selected aircraft if it's outside center 30% of viewport
        if let Some(ref selected_icao) = self.selected_aircraft {
            // Check if this is a new selection (different from previous)
            let is_new_selection = self.previous_selected_aircraft.as_ref() != Some(selected_icao);

            // Reset following flag when a new aircraft is selected
            if is_new_selection {
                self.following_aircraft = false;
                self.stored_map_center = None;
            }

            if is_new_selection && !self.following_aircraft {
                // Get the selected aircraft
                let aircraft_opt = {
                    let tracker = self.tracker.lock().unwrap();
                    tracker.get_aircraft_by_icao(selected_icao)
                };

                if let Some(aircraft) = aircraft_opt {
                    if let (Some(lat), Some(lon)) = (aircraft.latitude(), aircraft.longitude()) {
                        // Calculate where this aircraft would appear on screen with current map center
                        let tile_zoom_level = self.map_zoom_level.round() as u8;
                        let tile_pixel_size = 256.0;
                        let zoom_fraction = self.map_zoom_level - tile_zoom_level as f32;
                        let scale_factor = 2.0_f32.powf(zoom_fraction);

                        let aircraft_tile_x = WebMercator::lon_to_x(lon, tile_zoom_level);
                        let aircraft_tile_y = WebMercator::lat_to_y(lat, tile_zoom_level);
                        let center_tile_x = WebMercator::lon_to_x(self.map_center_lon, tile_zoom_level);
                        let center_tile_y = WebMercator::lat_to_y(self.map_center_lat, tile_zoom_level);

                        let pixel_x = (aircraft_tile_x - center_tile_x) * tile_pixel_size as f64 * scale_factor as f64;
                        let pixel_y = (aircraft_tile_y - center_tile_y) * tile_pixel_size as f64 * scale_factor as f64;

                        let aircraft_screen_pos = egui::pos2(
                            center.x + pixel_x as f32,
                            center.y + pixel_y as f32,
                        );

                        // Calculate center 30% bounds (15% from center in each direction)
                        let center_30_width = rect.width() * 0.30;
                        let center_30_height = rect.height() * 0.30;
                        let center_rect = egui::Rect::from_center_size(
                            center,
                            egui::vec2(center_30_width, center_30_height),
                        );

                        // Check if aircraft is outside the center 30%
                        if !center_rect.contains(aircraft_screen_pos) {
                            // Store current map center before panning
                            self.stored_map_center = Some((self.map_center_lat, self.map_center_lon));
                            self.following_aircraft = true;

                            // Pan to aircraft position
                            self.map_center_lat = lat;
                            self.map_center_lon = lon;
                        }
                    }
                }
            }
        }

        // Update previous selection AFTER checking for new selection
        // This ensures that on the next frame, we can detect if the selection changed
        self.previous_selected_aircraft = self.selected_aircraft.clone();

        // Reset hover state at start of frame
        self.hovered_map_item = None;

        // Calculate tile size in pixels at current zoom level
        let tile_pixel_size = 256.0;

        // Handle mouse wheel and scroll zoom (mouse wheel + two-finger trackpad drag)
        // Only capture scroll when hovering over map to avoid conflicts with other scroll areas
        let scroll_delta = if response.hovered() {
            ui.ctx().input(|i| i.smooth_scroll_delta)
        } else {
            egui::Vec2::ZERO
        };

        // Check for scroll events (mouse wheel or two-finger trackpad)
        let effective_zoom_delta = if scroll_delta.y.abs() > 0.1 {
            // Convert scroll to zoom (positive scroll = zoom in, negative = zoom out)
            // Scale factor: scroll of 100 pixels = 1 zoom level
            let zoom_factor = scroll_delta.y / 100.0;
            2.0_f32.powf(zoom_factor * 0.5) // 0.5 makes it less sensitive
        } else {
            // Fallback to pinch-zoom gesture
            ui.ctx().input(|i| i.zoom_delta())
        };

        // Handle zoom (from either source)
        if (effective_zoom_delta - 1.0).abs() > 0.001 {
            // Get cursor position for zoom-to-cursor behavior
            let cursor_pos = response.hover_pos().unwrap_or(center);

            // Calculate cursor position in map coordinates before zoom
            let old_zoom_level = self.map_zoom_level;
            let old_tile_zoom = old_zoom_level.round() as u8;

            // Calculate cursor offset from center in pixels
            let cursor_offset_x = (cursor_pos.x - center.x) as f64;
            let cursor_offset_y = (cursor_pos.y - center.y) as f64;

            // Calculate old scale factor for accurate pixel-to-tile conversion
            let old_zoom_fraction = old_zoom_level - old_tile_zoom as f32;
            let old_scale_factor = 2.0_f64.powf(old_zoom_fraction as f64);

            // Convert to tile coordinates (accounting for current scale factor)
            let cursor_tile_offset_x = cursor_offset_x / (tile_pixel_size as f64 * old_scale_factor);
            let cursor_tile_offset_y = cursor_offset_y / (tile_pixel_size as f64 * old_scale_factor);

            // Get lat/lon at cursor position before zoom
            let cursor_tile_x = WebMercator::lon_to_x(self.map_center_lon, old_tile_zoom) + cursor_tile_offset_x;
            let cursor_tile_y = WebMercator::lat_to_y(self.map_center_lat, old_tile_zoom) + cursor_tile_offset_y;
            let cursor_lat = tiles::WebMercator::tile_to_lat(cursor_tile_y, old_tile_zoom);
            let cursor_lon = tiles::WebMercator::tile_to_lon(cursor_tile_x, old_tile_zoom);

            // Apply zoom delta
            let zoom_change = effective_zoom_delta.log2();
            self.map_zoom_level += zoom_change;
            self.map_zoom_level = self.map_zoom_level.clamp(6.0, 12.0);

            // Calculate new tile zoom level
            let new_tile_zoom = self.map_zoom_level.round() as u8;

            // Calculate new scale factor for accurate pixel-to-tile conversion
            let new_zoom_fraction = self.map_zoom_level - new_tile_zoom as f32;
            let new_scale_factor = 2.0_f64.powf(new_zoom_fraction as f64);

            // Calculate where cursor should be in new zoom level
            let new_cursor_tile_x = WebMercator::lon_to_x(cursor_lon, new_tile_zoom);
            let new_cursor_tile_y = WebMercator::lat_to_y(cursor_lat, new_tile_zoom);

            // Recalculate cursor tile offset with new scale factor
            let new_cursor_tile_offset_x = cursor_offset_x / (tile_pixel_size as f64 * new_scale_factor);
            let new_cursor_tile_offset_y = cursor_offset_y / (tile_pixel_size as f64 * new_scale_factor);

            // Adjust map center so cursor stays at same screen position
            let new_center_tile_x = new_cursor_tile_x - new_cursor_tile_offset_x;
            let new_center_tile_y = new_cursor_tile_y - new_cursor_tile_offset_y;

            self.map_center_lat = tiles::WebMercator::tile_to_lat(new_center_tile_y, new_tile_zoom);
            self.map_center_lon = tiles::WebMercator::tile_to_lon(new_center_tile_x, new_tile_zoom);

            // Clamp latitude
            self.map_center_lat = self.map_center_lat.clamp(-85.0, 85.0);
        }

        // Use round for tile fetching for balanced sharpness when zooming
        let tile_zoom_level = self.map_zoom_level.round() as u8;

        // Calculate smooth scale factor for interpolation between zoom levels
        let zoom_fraction = self.map_zoom_level - tile_zoom_level as f32;
        let scale_factor = 2.0_f32.powf(zoom_fraction);

        // Render map tiles with smooth scaling
        let visible_tiles = self.tile_manager.get_visible_tiles(
            self.map_center_lat,
            self.map_center_lon,
            tile_zoom_level,
            rect.width(),
            rect.height(),
        );

        let mut tiles_rendered = 0;
        for (tile_coord, offset_x, offset_y) in visible_tiles {
            if let Some(texture) = self.tile_manager.get_tile(tile_coord, ui.ctx()) {
                // Apply scale factor to position and size for smooth zooming
                let scaled_offset_x = offset_x * scale_factor;
                let scaled_offset_y = offset_y * scale_factor;
                let scaled_tile_size = tile_pixel_size * scale_factor;

                let tile_pos = egui::pos2(
                    center.x + scaled_offset_x,
                    center.y + scaled_offset_y,
                );

                let tile_rect = egui::Rect::from_min_size(
                    tile_pos,
                    egui::vec2(scaled_tile_size, scaled_tile_size),
                );

                painter.image(
                    texture.id(),
                    tile_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                tiles_rendered += 1;
            }
        }

        // Update error state based on tile loading
        if self.tile_manager.get_error_count() > 0 {
            self.tile_error = Some(format!("Failed to load {} tiles", self.tile_manager.get_error_count()));
        } else if self.tile_manager.has_loading_tiles() {
            self.tile_error = Some("Loading map tiles...".to_string());
        } else if tiles_rendered > 0 {
            self.tile_error = None;
        }

        // Handle dragging with Web Mercator
        if response.dragged() {
            let delta = response.drag_delta();

            // Convert pixel movement to lat/lon change at current zoom
            let scale = 2.0_f64.powf(self.map_zoom_level as f64);
            let lat_per_pixel = 180.0 / (tile_pixel_size as f64 * scale);
            let lon_per_pixel = 360.0 / (tile_pixel_size as f64 * scale);

            // Note: We need to account for Mercator distortion
            let lat_rad = self.map_center_lat.to_radians();
            let cos_lat = lat_rad.cos();

            self.map_center_lat += delta.y as f64 * lat_per_pixel;
            self.map_center_lon -= delta.x as f64 * lon_per_pixel / cos_lat.max(0.1);

            // Clamp latitude to valid range
            self.map_center_lat = self.map_center_lat.clamp(-85.0, 85.0);
        }

        // Helper function to convert lat/lon to screen coordinates using Web Mercator
        let to_screen = |lat: f64, lon: f64| -> egui::Pos2 {
            let tile_x = WebMercator::lon_to_x(lon, tile_zoom_level);
            let tile_y = WebMercator::lat_to_y(lat, tile_zoom_level);

            let center_tile_x = WebMercator::lon_to_x(self.map_center_lon, tile_zoom_level);
            let center_tile_y = WebMercator::lat_to_y(self.map_center_lat, tile_zoom_level);

            // Apply scale factor for smooth zoom
            let pixel_x = (tile_x - center_tile_x) * tile_pixel_size as f64 * scale_factor as f64;
            let pixel_y = (tile_y - center_tile_y) * tile_pixel_size as f64 * scale_factor as f64;

            egui::pos2(
                center.x + pixel_x as f32,
                center.y + pixel_y as f32,
            )
        };

        // Calculate visible bounds for spatial filtering with caching
        let needs_recalc = self.cached_bounds.is_none()
            || (self.map_zoom_level - self.last_bounds_zoom).abs() > 0.05
            || (self.map_center_lat - self.last_bounds_center.0).abs() > 0.001
            || (self.map_center_lon - self.last_bounds_center.1).abs() > 0.001;

        let (min_lat, max_lat, min_lon, max_lon) = if needs_recalc {
            // Calculate the bounds by converting viewport corners to lat/lon
            let tile_pixel_size_f64 = tile_pixel_size as f64;
            let scale = 2.0_f64.powf(tile_zoom_level as f64);

            // Calculate how many degrees the viewport represents at this zoom level
            let half_viewport_width = (rect.width() as f64) / 2.0;
            let half_viewport_height = (rect.height() as f64) / 2.0;

            // Degrees per pixel at current zoom (simplified, not accounting for Mercator distortion)
            let degrees_per_pixel_lon = 360.0 / (tile_pixel_size_f64 * scale);
            let degrees_per_pixel_lat = 180.0 / (tile_pixel_size_f64 * scale);

            // Calculate bounds with some padding (1.5x viewport to handle edge cases)
            let padding_multiplier = 1.5;
            let lon_range = (half_viewport_width * degrees_per_pixel_lon) * padding_multiplier;
            let lat_range = (half_viewport_height * degrees_per_pixel_lat) * padding_multiplier;

            let min_lat = (self.map_center_lat - lat_range).max(-85.0);
            let max_lat = (self.map_center_lat + lat_range).min(85.0);
            let min_lon = self.map_center_lon - lon_range;
            let max_lon = self.map_center_lon + lon_range;

            // Cache the calculated bounds
            self.cached_bounds = Some((min_lat, max_lat, min_lon, max_lon));
            self.last_bounds_zoom = self.map_zoom_level;
            self.last_bounds_center = (self.map_center_lat, self.map_center_lon);

            (min_lat, max_lat, min_lon, max_lon)
        } else {
            // Use cached bounds
            self.cached_bounds.unwrap()
        };

        // PERFORMANCE: Cache aviation data to avoid cloning thousands of objects every frame
        // Only recalculate when bounds or filter settings change significantly
        // Check if bounds have changed enough to warrant a cache rebuild (10% of viewport)
        let bounds_changed_significantly = if let Some((last_min_lat, last_max_lat, last_min_lon, last_max_lon)) = self.last_aviation_cache_bounds {
            let lat_threshold = (last_max_lat - last_min_lat) * 0.1;
            let lon_threshold = (last_max_lon - last_min_lon) * 0.1;
            (min_lat - last_min_lat).abs() > lat_threshold
                || (max_lat - last_max_lat).abs() > lat_threshold
                || (min_lon - last_min_lon).abs() > lon_threshold
                || (max_lon - last_max_lon).abs() > lon_threshold
        } else {
            true  // No previous bounds, need to build cache
        };

        let cache_needs_update = self.cached_aviation_data.is_none()
            || bounds_changed_significantly
            || self.last_aviation_cache_filter != self.airport_filter;

        if cache_needs_update {
            // Cache is stale - rebuild it by cloning from locked data
            let cache_rebuild_start = std::time::Instant::now();
            if let Ok(aviation_data) = self.aviation_data.lock() {
                let airports: Vec<_> = aviation_data.get_airports_in_bounds(min_lat, max_lat, min_lon, max_lon)
                    .into_iter()
                    .cloned()
                    .collect();

                // Get runways for all visible airports
                let runways: Vec<(String, Vec<_>)> = airports.iter()
                    .map(|airport| {
                        let airport_runways = aviation_data.get_runways_for_airport(&airport.icao)
                            .into_iter()
                            .cloned()
                            .collect();
                        (airport.icao.clone(), airport_runways)
                    })
                    .collect();

                let navaids: Vec<_> = aviation_data.get_navaids_in_bounds(min_lat, max_lat, min_lon, max_lon)
                    .into_iter()
                    .cloned()
                    .collect();

                // Update cache
                self.cached_aviation_data = Some((airports.clone(), runways.clone(), navaids.clone()));
                self.last_aviation_cache_bounds = Some((min_lat, max_lat, min_lon, max_lon));
                self.last_aviation_cache_filter = self.airport_filter;

                let cache_rebuild_time = cache_rebuild_start.elapsed().as_millis();
                if cache_rebuild_time > 5 {
                    println!("Aviation cache rebuilt: {} airports, {} runway groups, {} navaids in {}ms",
                        airports.len(), runways.len(), navaids.len(), cache_rebuild_time);
                }
            } // Lock released here
        }

        // Use cached data (cheap reference, no cloning!)
        let (visible_airports, airport_runways, visible_navaids) = if let Some((ref airports, ref runways, ref navaids)) = self.cached_aviation_data {
            (airports, runways, navaids)
        } else {
            // Fallback to empty data if cache somehow failed
            (&Vec::new(), &Vec::new(), &Vec::new())
        };

        // Draw aviation overlays (now using cloned data, no lock held)
        // Runways (draw first, so they appear under airports)
        // PERFORMANCE: Only render runways at high zoom levels where they're distinguishable
        if self.show_runways && self.map_zoom_level >= 9.5 {
            // Limit runway rendering count when zoomed out
            let max_runways = if self.map_zoom_level >= 11.0 { usize::MAX } else { 500 };
            let mut runways_drawn = 0;

            for (_airport_icao, runways) in airport_runways {
                if runways_drawn >= max_runways {
                    break;
                }
                for runway in runways {
                    if runways_drawn >= max_runways {
                        break;
                    }
                    if let (Some(le_lat), Some(le_lon), Some(he_lat), Some(he_lon)) =
                        (runway.le_latitude, runway.le_longitude, runway.he_latitude, runway.he_longitude)
                    {
                        let le_pos = to_screen(le_lat, le_lon);
                        let he_pos = to_screen(he_lat, he_lon);

                        // Only draw if at least one endpoint is visible
                        if rect.contains(le_pos) || rect.contains(he_pos) {
                            let runway_color = egui::Color32::from_rgb(80, 80, 100);
                            painter.line_segment(
                                [le_pos, he_pos],
                                egui::Stroke::new(runway.stroke_width(), runway_color)
                            );
                            runways_drawn += 1;
                        }
                    }
                }
            }
        }

        // Airports - with aggressive LOD optimization for performance
        if self.show_airports {
            // PERFORMANCE: Limit airport count based on zoom level
            // When zoomed out, only show the most important airports
            let max_airports = if self.map_zoom_level >= 10.0 {
                usize::MAX  // Show all at high zoom
            } else if self.map_zoom_level >= 9.0 {
                1000  // Moderate limit for medium zoom
            } else if self.map_zoom_level >= 8.0 {
                500   // Stricter limit for lower zoom
            } else {
                200   // Very strict limit when zoomed out
            };

            let mut airports_drawn = 0;

            // PERFORMANCE: Prioritize major airports when zoomed out
            // Sort airports by importance (major > medium > small) before rendering
            let mut prioritized_airports: Vec<_> = visible_airports.iter().collect();
            prioritized_airports.sort_by_key(|a| {
                if a.is_major() { 0 }
                else if a.is_medium() { 1 }
                else { 2 }
            });

            for airport in prioritized_airports {
                if airports_drawn >= max_airports {
                    break;
                }

                // Apply airport filter with zoom-based refinement
                let should_show = match self.airport_filter {
                    AirportFilter::All => {
                        // Show all airplane airports, but filter small ones by zoom
                        airport.is_public_airplane_airport() &&
                        (airport.is_major() || airport.is_medium() || self.map_zoom_level >= 9.5)
                    }
                    AirportFilter::FrequentlyUsed => {
                        // Show frequently used airports (scheduled service or large/medium)
                        airport.is_frequently_used()
                    }
                    AirportFilter::MajorOnly => {
                        // Show only large airports
                        airport.is_major()
                    }
                };

                if !should_show {
                    continue;
                }

                let pos = to_screen(airport.latitude, airport.longitude);

                // Only draw if within visible area
                if rect.contains(pos) {
                    let airport_color = if airport.is_major() {
                        egui::Color32::from_rgb(200, 100, 100) // Red for large airports
                    } else if airport.is_medium() {
                        egui::Color32::from_rgb(150, 150, 100) // Yellow for medium
                    } else {
                        egui::Color32::from_rgb(120, 120, 120) // Gray for small
                    };

                    painter.circle_filled(pos, airport.render_radius(), airport_color);
                    painter.circle_stroke(
                        pos,
                        airport.render_radius(),
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 255, 255)),
                    );

                    // PERFORMANCE: Only draw labels at high zoom levels (text rendering is expensive)
                    if self.map_zoom_level >= 9.0 {
                        painter.text(
                            pos + egui::vec2(0.0, -12.0),
                            egui::Align2::CENTER_BOTTOM,
                            &airport.icao,
                            egui::FontId::proportional(9.0),
                            egui::Color32::from_rgb(220, 220, 220),
                        );
                    }

                    // Check for hover
                    if let Some(hover_pos) = response.hover_pos() {
                        let distance = hover_pos.distance(pos);
                        let hover_radius = airport.render_radius() + 8.0; // Add some margin for easier hovering
                        if distance <= hover_radius {
                            self.hovered_map_item = Some(HoveredMapItem::Airport(airport.clone()));
                        }
                    }

                    airports_drawn += 1;
                }
            }
        }

        // Navaids - with count limiting for performance
        if self.show_navaids && self.map_zoom_level >= 9.0 {
            // PERFORMANCE: Limit navaid rendering based on zoom
            let max_navaids = if self.map_zoom_level >= 10.0 {
                1000  // Show more at high zoom
            } else {
                300   // Limit at medium zoom
            };

            let mut navaids_drawn = 0;

            for navaid in visible_navaids {
                if navaids_drawn >= max_navaids {
                    break;
                }

                let pos = to_screen(navaid.latitude, navaid.longitude);

                // Only draw if within visible area
                if rect.contains(pos) {
                    let (r, g, b) = navaid.get_color();
                    let navaid_color = egui::Color32::from_rgb(r, g, b);
                    let size = navaid.symbol_size();

                    // Draw as a triangle
                    let points = vec![
                        pos + egui::vec2(0.0, -size),
                        pos + egui::vec2(size * 0.866, size * 0.5),
                        pos + egui::vec2(-size * 0.866, size * 0.5),
                    ];
                    painter.add(egui::Shape::convex_polygon(
                        points,
                        navaid_color,
                        egui::Stroke::new(1.0, egui::Color32::WHITE),
                    ));

                    // PERFORMANCE: Only draw labels at higher zoom levels (text is expensive)
                    if self.map_zoom_level >= 10.0 {
                        painter.text(
                            pos + egui::vec2(0.0, size + 8.0),
                            egui::Align2::CENTER_TOP,
                            &navaid.ident,
                            egui::FontId::proportional(8.0),
                            navaid_color,
                        );
                    }

                    // Check for hover
                    if let Some(hover_pos) = response.hover_pos() {
                        let distance = hover_pos.distance(pos);
                        let hover_radius = size + 8.0; // Add some margin for easier hovering
                        if distance <= hover_radius {
                            self.hovered_map_item = Some(HoveredMapItem::Navaid(navaid.clone()));
                        }
                    }

                    navaids_drawn += 1;
                }
            }
        }

        // Get aircraft with cheap Arc clones - eliminates the second expensive clone!
        let aircraft_list: Vec<Aircraft> = {
            let tracker = self.tracker.lock()
                .expect("Aircraft tracker mutex poisoned");
            tracker.get_aircraft()  // Now returns Vec<Aircraft> with Arc clones
        };

        // PERFORMANCE: Trail rendering with aggressive LOD optimization
        // LOD system reduces point count when zoomed out for performance
        let should_draw_trails = true;  // Always show trails regardless of zoom level
        let trail_detail_level = if self.map_zoom_level >= 10.0 {
            1  // Full detail: render every point
        } else if self.map_zoom_level >= 9.0 {
            2  // Medium detail: render every 2nd point
        } else {
            4  // Low detail: render every 4th point
        };

        // Get time-limited trails setting once to avoid repeated locking
        let time_limited_trails = self.tracker.lock().unwrap().get_time_limited_trails();

        for aircraft in &aircraft_list {
            // Draw trail first (so aircraft appears on top)
            // PERFORMANCE: Skip trail rendering entirely when zoomed out (trails invisible)
            if should_draw_trails {
                // Use with_data to efficiently access position history
                aircraft.with_data(|data| {
                if data.position_history.is_empty() {
                    return;
                }

                // PERFORMANCE: Skip trail processing if aircraft is far outside viewport
                // Quick bounds check before expensive trail rendering
                if let (Some(lat), Some(lon)) = (data.latitude, data.longitude) {
                    let screen_pos = to_screen(lat, lon);
                    let margin = 100.0; // Extra margin to catch trails extending beyond aircraft
                    let expanded_rect = rect.expand(margin);

                    if !expanded_rect.contains(screen_pos) {
                        // Aircraft is off-screen, skip trail rendering
                        return;
                    }
                }

                let now = chrono::Utc::now();

                // PERFORMANCE: Render trails with level-of-detail decimation
                // Skip points based on zoom level to reduce computation
                let mut points_drawn = 0;
                for i in (0..data.position_history.len()).step_by(trail_detail_level) {
                    let point = &data.position_history[i];
                    let age = (now - point.timestamp).num_milliseconds() as f32 / 1000.0;

                    // Only draw if age is within trail duration (when time-limited trails enabled)
                    if time_limited_trails && age > TRAIL_MAX_AGE_SECONDS {
                        continue;
                    }

                    // Calculate opacity based on age (only when time-limited trails enabled)
                    let alpha = if time_limited_trails {
                        if age <= TRAIL_SOLID_DURATION_SECONDS {
                            255 // Solid for first half
                        } else {
                            let fade_age = age - TRAIL_SOLID_DURATION_SECONDS;
                            let opacity = (1.0 - (fade_age / TRAIL_FADE_DURATION_SECONDS)).clamp(0.0, 1.0);
                            (opacity * 255.0) as u8
                        }
                    } else {
                        255 // Full opacity for unlimited trails
                    };

                    let trail_pos = to_screen(point.lat, point.lon);

                    // Draw line to next point (accounting for step size)
                    let next_idx = i + trail_detail_level;
                    if next_idx < data.position_history.len() {
                        let next_point = &data.position_history[next_idx];
                        let next_age = (now - next_point.timestamp).num_milliseconds() as f32 / 1000.0;

                        // Only draw if next point is also within trail duration (when time-limited)
                        if !time_limited_trails || next_age <= TRAIL_MAX_AGE_SECONDS {
                            let next_pos = to_screen(next_point.lat, next_point.lon);

                            // Get altitude-based color
                            let (r, g, b) = Self::altitude_to_color(point.altitude);

                            // Apply time-based transparency to the altitude color
                            let trail_color = egui::Color32::from_rgba_unmultiplied(r, g, b, alpha);
                            painter.line_segment(
                                [trail_pos, next_pos],
                                egui::Stroke::new(2.0, trail_color)
                            );
                            points_drawn += 1;
                        }
                    }

                    // PERFORMANCE: Limit total trail segments drawn per aircraft when zoomed out
                    if points_drawn >= 100 && self.map_zoom_level < 9.0 {
                        break;
                    }
                }

                // Always draw line from last history point to current position for smooth connection
                if let (Some(lat), Some(lon)) = (data.latitude, data.longitude) {
                    if let Some(last_point) = data.position_history.last() {
                        let last_pos = to_screen(last_point.lat, last_point.lon);
                        let current_pos = to_screen(lat, lon);

                        // Most recent segment is fully opaque with altitude-based color
                        let (r, g, b) = Self::altitude_to_color(data.altitude);
                        let trail_color = egui::Color32::from_rgb(r, g, b);
                        painter.line_segment(
                            [last_pos, current_pos],
                            egui::Stroke::new(2.5, trail_color)
                        );
                    }
                }
                });
            }

            if let (Some(lat), Some(lon)) = (aircraft.latitude(), aircraft.longitude()) {
                let pos = to_screen(lat, lon);

                // Only draw if within visible area
                if rect.contains(pos) {
                    // Check if this aircraft is selected
                    let icao = aircraft.icao();
                    let is_selected = self.selected_aircraft.as_ref() == Some(&icao);

                    // Draw aircraft as an airplane icon with selection feedback
                    let (color, size) = if is_selected {
                        (egui::Color32::from_rgb(255, 100, 100), 7.0) // Larger red airplane when selected
                    } else {
                        (egui::Color32::from_rgb(120, 220, 120), 5.0) // Normal green
                    };

                    // Draw airplane icon pointing in direction of travel
                    let track = aircraft.track().unwrap_or(0.0) as f32;
                    Self::draw_aircraft_icon(&painter, pos, track, color, size);

                    // Add selection ring
                    if is_selected {
                        painter.circle_stroke(
                            pos,
                            size * 1.8,
                            egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 50)),
                        );
                    }

                    // Draw callsign and altitude labels to the right of the aircraft icon
                    let mut label_offset_y = -10.0; // Start slightly above the icon

                    // Draw callsign first (top)
                    if let Some(ref callsign) = aircraft.callsign() {
                        let text = callsign.trim();
                        let text_pos = pos + egui::vec2(10.0, label_offset_y);

                        // Create a text galley to measure the text size
                        let galley = painter.layout_no_wrap(
                            text.to_string(),
                            egui::FontId::proportional(11.0),
                            egui::Color32::WHITE,
                        );

                        // Draw background box
                        let padding = egui::vec2(3.0, 2.0);
                        let box_rect = egui::Rect::from_min_size(
                            text_pos - egui::vec2(padding.x, galley.size().y / 2.0 + padding.y),
                            galley.size() + padding * 2.0,
                        );
                        painter.rect_filled(
                            box_rect,
                            2.0,
                            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180),
                        );

                        // Draw text
                        painter.text(
                            text_pos,
                            egui::Align2::LEFT_CENTER,
                            text,
                            egui::FontId::proportional(11.0),
                            egui::Color32::WHITE,
                        );
                        label_offset_y += 14.0; // Move down for next label
                    }

                    // Draw altitude below callsign
                    if let Some(alt) = aircraft.altitude() {
                        let alt_text = if alt >= 18000 {
                            format!("FL{:03}", alt / 100)
                        } else {
                            format!("{}ft", alt)
                        };
                        let text_pos = pos + egui::vec2(10.0, label_offset_y);

                        // Create a text galley to measure the text size
                        let galley = painter.layout_no_wrap(
                            alt_text.clone(),
                            egui::FontId::proportional(10.0),
                            egui::Color32::from_rgb(200, 200, 200),
                        );

                        // Draw background box
                        let padding = egui::vec2(3.0, 2.0);
                        let box_rect = egui::Rect::from_min_size(
                            text_pos - egui::vec2(padding.x, galley.size().y / 2.0 + padding.y),
                            galley.size() + padding * 2.0,
                        );
                        painter.rect_filled(
                            box_rect,
                            2.0,
                            egui::Color32::from_rgba_unmultiplied(0, 0, 0, 180),
                        );

                        // Draw text
                        painter.text(
                            text_pos,
                            egui::Align2::LEFT_CENTER,
                            &alt_text,
                            egui::FontId::proportional(10.0),
                            egui::Color32::from_rgb(200, 200, 200),
                        );
                    }

                    // Check for hover
                    if let Some(hover_pos) = response.hover_pos() {
                        let distance = hover_pos.distance(pos);
                        let hover_radius = size * 1.8 + 5.0; // Size of airplane + margin for easier hovering
                        if distance <= hover_radius {
                            self.hovered_map_item = Some(HoveredMapItem::Aircraft(aircraft.clone()));
                        }
                    }
                }
            }
        }

        // Handle map clicks for aircraft selection/deselection
        let mut should_restore_map_position = false;
        if response.clicked() {
            if let Some(click_pos) = response.interact_pointer_pos() {
                let mut clicked_aircraft: Option<String> = None;

                // Check all aircraft to see if any were clicked
                for aircraft in aircraft_list.iter() {
                    if let (Some(lat), Some(lon)) = (aircraft.latitude(), aircraft.longitude()) {
                        let pos = to_screen(lat, lon);
                        if rect.contains(pos) {
                            let icao = aircraft.icao();
                            let distance = ((click_pos.x - pos.x).powi(2) + (click_pos.y - pos.y).powi(2)).sqrt();
                            let click_radius = if self.selected_aircraft.as_ref() == Some(&icao) { 10.0 } else { 8.0 };
                            if distance <= click_radius {
                                clicked_aircraft = Some(icao);
                                break; // Found a clicked aircraft, stop searching
                            }
                        }
                    }
                }

                // If clicking empty space while following, mark for restoration
                if clicked_aircraft.is_none() && self.following_aircraft {
                    should_restore_map_position = true;
                }

                // Update selection: select clicked aircraft or deselect if empty space clicked
                self.selected_aircraft = clicked_aircraft;
            }
        }

        // Draw receiver location marker
        let receiver_pos = to_screen(self.receiver_lat, self.receiver_lon);
        if rect.contains(receiver_pos) {
            // Draw a green circle for the receiver
            painter.circle_filled(receiver_pos, 8.0, egui::Color32::from_rgb(50, 255, 50));
            painter.circle_stroke(
                receiver_pos,
                8.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 180, 0)),
            );

            // Draw crosshair
            let crosshair_size = 12.0;
            painter.line_segment(
                [
                    receiver_pos + egui::vec2(-crosshair_size, 0.0),
                    receiver_pos + egui::vec2(crosshair_size, 0.0),
                ],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 180, 0)),
            );
            painter.line_segment(
                [
                    receiver_pos + egui::vec2(0.0, -crosshair_size),
                    receiver_pos + egui::vec2(0.0, crosshair_size),
                ],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(0, 180, 0)),
            );

            // Draw label
            painter.text(
                receiver_pos + egui::vec2(0.0, -20.0),
                egui::Align2::CENTER_BOTTOM,
                "Receiver",
                egui::FontId::proportional(11.0),
                egui::Color32::from_rgb(0, 180, 0),
            );
        }

        // Instructions
        painter.text(
            rect.left_top() + egui::vec2(10.0, 10.0),
            egui::Align2::LEFT_TOP,
            "Drag to pan | Scroll/pinch to zoom",
            egui::FontId::proportional(12.0),
            egui::Color32::BLACK,
        );

        // Attribution (required by Carto)
        painter.text(
            rect.right_bottom() + egui::vec2(-10.0, -10.0),
            egui::Align2::RIGHT_BOTTOM,
            "© OpenStreetMap contributors © CARTO",
            egui::FontId::proportional(10.0),
            egui::Color32::from_black_alpha(180),
        );

        // Error display at the top
        if let Some(ref error_msg) = self.tile_error {
            let is_error = error_msg.contains("Failed");
            let bg_color = if is_error {
                egui::Color32::from_rgb(220, 50, 50)
            } else {
                egui::Color32::from_rgb(255, 200, 100)
            };

            // Draw error bubble
            let error_pos = rect.center_top() + egui::vec2(0.0, 20.0);
            let text_galley = painter.layout_no_wrap(
                error_msg.clone(),
                egui::FontId::proportional(12.0),
                egui::Color32::WHITE,
            );

            let padding = egui::vec2(12.0, 6.0);
            let bubble_rect = egui::Rect::from_center_size(
                error_pos,
                text_galley.size() + padding * 2.0,
            );

            painter.rect_filled(bubble_rect, 5.0, bg_color);
            painter.text(
                error_pos,
                egui::Align2::CENTER_CENTER,
                error_msg,
                egui::FontId::proportional(12.0),
                egui::Color32::WHITE,
            );
        }

        // Render hover popup if hovering over a map item
        if let Some(ref hovered_item) = self.hovered_map_item {
            if let Some(hover_pos) = response.hover_pos() {
                // Position popup with offset to avoid obscuring the item
                let popup_pos = hover_pos + egui::vec2(15.0, 10.0);

                egui::Area::new("map_item_popup".into())
                    .fixed_pos(popup_pos)
                    .order(egui::Order::Tooltip)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style())
                            .show(ui, |ui| {
                                match hovered_item {
                                    HoveredMapItem::Airport(airport) => airport.render_popup(ui, self.receiver_lat, self.receiver_lon, &self.aircraft_types),
                                    HoveredMapItem::Navaid(navaid) => navaid.render_popup(ui, self.receiver_lat, self.receiver_lon, &self.aircraft_types),
                                    HoveredMapItem::Aircraft(aircraft) => aircraft.render_popup(ui, self.receiver_lat, self.receiver_lon, &self.aircraft_types),
                                }
                            });
                    });
            }
        }

        // Restore map position if we clicked on empty space while following an aircraft
        if should_restore_map_position {
            if let Some((stored_lat, stored_lon)) = self.stored_map_center {
                self.map_center_lat = stored_lat;
                self.map_center_lon = stored_lon;
                self.stored_map_center = None;
                self.following_aircraft = false;
            }
        }
    }
}

impl eframe::App for AdsbApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let frame_start = std::time::Instant::now();

        // Startup sequence - perform initialization steps across first few frames
        if self.startup_state != StartupState::Complete {
            self.startup_frame_count += 1;

            match self.startup_state {
                StartupState::InitializingWindow => {
                    // Window is now visible, wait 2 frames for UI to settle
                    if self.startup_frame_count >= 2 {
                        self.startup_state = StartupState::DetectingLocation;
                        self.system_status.lock().unwrap().add_diagnostic(
                            DiagnosticLevel::Info,
                            "Detecting location...".to_string()
                        );
                    }
                }
                StartupState::DetectingLocation => {
                    // Get current GPS location (this may block briefly)
                    let (lat, lon) = get_current_location()
                        .unwrap_or_else(|| {
                            self.system_status.lock().unwrap().add_diagnostic(
                                DiagnosticLevel::Info,
                                "Using default location (San Francisco)".to_string()
                            );
                            (37.7749, -122.4194)
                        });

                    // Update location
                    self.map_center_lat = lat;
                    self.map_center_lon = lon;
                    self.receiver_lat = lat;
                    self.receiver_lon = lon;

                    // Set the center location in the tracker for distance filtering
                    self.tracker.lock().unwrap().set_center(lat, lon);

                    self.system_status.lock().unwrap().add_diagnostic(
                        DiagnosticLevel::Info,
                        format!("Location set: {:.4}°, {:.4}°", lat, lon)
                    );

                    self.startup_state = StartupState::StartingTcpClient;
                }
                StartupState::StartingTcpClient => {
                    // Spawn TCP connection in background
                    self.system_status.lock().unwrap().add_diagnostic(
                        DiagnosticLevel::Info,
                        format!("Starting TCP client (connecting to {})...", self.server_address)
                    );

                    let tracker_clone = self.tracker.clone();
                    let status_clone = self.system_status.clone();
                    let server_address = self.server_address.clone();
                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(tcp_client::connect_adsb_feed(&server_address, tracker_clone, status_clone));
                    });

                    self.startup_state = StartupState::LoadingAviationData;
                }
                StartupState::LoadingAviationData => {
                    // Load aviation data in background
                    self.system_status.lock().unwrap().add_diagnostic(
                        DiagnosticLevel::Info,
                        "Loading aviation data...".to_string()
                    );

                    let aviation_data_clone = self.aviation_data.clone();
                    let loading_clone = self.aviation_data_loading.clone();
                    let status_clone = self.system_status.clone();

                    std::thread::spawn(move || {
                        let rt = tokio::runtime::Runtime::new().unwrap();
                        rt.block_on(async {
                            match AviationData::load_or_download("data".into()).await {
                                Ok(data) => {
                                    let airports_count = data.airports.len();
                                    let runways_count = data.runways.len();
                                    let navaids_count = data.navaids.len();

                                    *aviation_data_clone.lock().unwrap() = data;
                                    *loading_clone.lock().unwrap() = false;

                                    // Update status
                                    status_clone.lock().unwrap().set_aviation_data(
                                        airports_count,
                                        runways_count,
                                        navaids_count
                                    );
                                }
                                Err(e) => {
                                    eprintln!("Failed to load aviation data: {}", e);
                                    *loading_clone.lock().unwrap() = false;
                                    status_clone.lock().unwrap().add_diagnostic(
                                        DiagnosticLevel::Error,
                                        format!("Failed to load aviation data: {}", e)
                                    );
                                }
                            }
                        });
                    });

                    self.startup_state = StartupState::LoadingAircraftDB;
                }
                StartupState::LoadingAircraftDB => {
                    // Load aircraft database in background
                    self.system_status.lock().unwrap().add_diagnostic(
                        DiagnosticLevel::Info,
                        "Loading aircraft database...".to_string()
                    );

                    let aircraft_db_clone = self.aircraft_db.clone();
                    let status_clone = self.system_status.clone();

                    std::thread::spawn(move || {
                        match aircraft_db_clone.lock().unwrap().load_or_download() {
                            Ok(size) => {
                                status_clone.lock().unwrap().set_aircraft_db(size);
                            }
                            Err(e) => {
                                eprintln!("Failed to load aircraft database: {}", e);
                                status_clone.lock().unwrap().add_diagnostic(
                                    DiagnosticLevel::Error,
                                    format!("Failed to load aircraft database: {}", e)
                                );
                            }
                        }
                    });

                    self.startup_state = StartupState::Complete;
                    self.system_status.lock().unwrap().add_diagnostic(
                        DiagnosticLevel::Info,
                        "Startup sequence complete - background loading in progress".to_string()
                    );
                }
                StartupState::Complete => {
                    // Nothing to do, startup is complete
                }
            }
        }

        // Initialize placeholder texture on first frame
        if self.photo_manager.get_placeholder().is_none() {
            self.photo_manager.init_placeholder(ctx);
        }

        // Update system status with current aircraft stats
        {
            let tracker = self.tracker.lock().unwrap();
            let aircraft_list = tracker.get_aircraft();  // Cheap Arc clones
            let total = aircraft_list.len();
            let active = aircraft_list.iter().filter(|a| {
                (chrono::Utc::now() - a.last_seen()).num_seconds() < 60
            }).count();

            self.system_status.lock().unwrap().update_aircraft_stats(total, active);
            self.system_status.lock().unwrap().update_uptime();
        }

        // Request continuous repaints for smooth interaction
        // Check for active interactions: dragging, zooming, or clicking
        let is_interacting = ctx.input(|i| {
            i.pointer.any_down()  // Currently pressing (dragging)
            || i.smooth_scroll_delta != egui::Vec2::ZERO  // Mouse wheel or trackpad scroll zoom
            || i.zoom_delta() != 1.0  // Pinch zoom
        });

        if is_interacting {
            // During interaction, request next frame immediately for smooth 60fps
            ctx.request_repaint_after(std::time::Duration::ZERO);
        } else {
            // When idle, update every 100ms for smooth aircraft movement
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        // Map takes full width
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                self.draw_map(ui);
            });

        // Floating aircraft list on the right with gradient sheen effect
        let screen_height = ctx.screen_rect().height();
        egui::Window::new("Aircraft List")
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-10.0, 10.0))
            .fixed_size(egui::vec2(350.0, screen_height - 20.0))
            .resizable(false)
            .collapsible(true)
            .frame(egui::Frame::NONE
                .fill(egui::Color32::TRANSPARENT)
                .corner_radius(8.0)
            )
            .show(ctx, |ui| {
                // Draw gradient background for sheen effect
                let rect = ui.available_rect_before_wrap();
                let painter = ui.painter();

                // Layer 1: Solid background for clarity and separation from map
                painter.rect_filled(
                    rect,
                    8.0,  // Corner radius matches window frame
                    egui::Color32::from_rgba_unmultiplied(25, 30, 35, 153)  // 60% opacity dark background
                );

                // Layer 2: Gradient overlay for sheen effect (vivid with higher opacity and brightness)
                let top_color = egui::Color32::from_rgba_unmultiplied(55, 64, 72, 179);     // Top (70% opacity, 15% less bright)
                let bottom_color = egui::Color32::from_rgba_unmultiplied(15, 20, 25, 128); // Darker bottom (50% opacity)

                // Draw gradient using mesh with vertices
                let mut mesh = egui::epaint::Mesh::default();

                // Add vertices: top-left, top-right, bottom-right, bottom-left
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: rect.left_top(),
                    uv: egui::epaint::WHITE_UV,
                    color: top_color,
                });
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: rect.right_top(),
                    uv: egui::epaint::WHITE_UV,
                    color: top_color,
                });
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: rect.right_bottom(),
                    uv: egui::epaint::WHITE_UV,
                    color: bottom_color,
                });
                mesh.vertices.push(egui::epaint::Vertex {
                    pos: rect.left_bottom(),
                    uv: egui::epaint::WHITE_UV,
                    color: bottom_color,
                });

                // Add triangles (two triangles make a rectangle)
                mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);

                painter.add(egui::Shape::mesh(mesh));

                self.draw_aircraft_list(ui);
            });

        // Overlay controls window (top-left)
        egui::Window::new("Map Overlays")
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(10.0, 10.0))
            .resizable(false)
            .collapsible(true)
            .default_open(false)
            .show(ctx, |ui| {
                // Check if data is still loading
                let is_loading = *self.aviation_data_loading.lock().unwrap();

                if is_loading {
                    ui.label(egui::RichText::new("⏳ Loading aviation data...")
                        .color(egui::Color32::from_rgb(255, 200, 100)));
                    ui.label(egui::RichText::new("(Downloading if needed)")
                        .color(egui::Color32::from_rgb(150, 150, 150))
                        .size(9.0));
                } else {
                    ui.horizontal(|ui| {
                        ui.label("Airports:");
                        ui.checkbox(&mut self.show_airports, "");
                    });

                    // Airport filter options (indented)
                    if self.show_airports {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Airport Filter:")
                            .size(10.0)
                            .color(egui::Color32::from_rgb(180, 180, 180)));

                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.radio_value(&mut self.airport_filter, AirportFilter::FrequentlyUsed, "Public/Frequent");
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.radio_value(&mut self.airport_filter, AirportFilter::All, "All Airports");
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            ui.radio_value(&mut self.airport_filter, AirportFilter::MajorOnly, "Major Only");
                        });
                        ui.add_space(4.0);
                    }

                    ui.horizontal(|ui| {
                        ui.label("Runways:");
                        ui.checkbox(&mut self.show_runways, "");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Navaids:");
                        ui.checkbox(&mut self.show_navaids, "");
                    });
                    ui.horizontal(|ui| {
                        ui.label("Time-Limited Trails:");
                        if ui.checkbox(&mut self.time_limited_trails, "").changed() {
                            // Sync checkbox state to tracker
                            self.tracker.lock().unwrap().set_time_limited_trails(self.time_limited_trails);
                        }
                    });
                    ui.separator();

                    // Get counts from locked data
                    if let Ok(data) = self.aviation_data.lock() {
                        let airports_count = data.airports.len();
                        let runways_count = data.runways.len();
                        let navaids_count = data.navaids.len();

                        if airports_count > 0 || runways_count > 0 || navaids_count > 0 {
                            ui.label(format!("Loaded: {} airports", airports_count));
                            ui.label(format!("         {} runways", runways_count));
                            ui.label(format!("         {} navaids", navaids_count));
                        } else {
                            ui.label(egui::RichText::new("No data loaded")
                                .color(egui::Color32::from_rgb(150, 150, 150)));
                        }
                    }
                }
            });

        // Render status pane (bottom-left overlay)
        {
            let status = self.system_status.lock().unwrap();
            self.status_pane.render(ctx, &status);
        }

        // Update frame time performance metrics
        let frame_duration = frame_start.elapsed().as_secs_f64() * 1000.0;
        self.system_status.lock().unwrap().update_performance(frame_duration);
    }
}
