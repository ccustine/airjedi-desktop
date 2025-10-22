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
mod carto_tiles;
mod config;
mod connection_manager;
mod photo_cache;
mod status;
mod status_pane;
mod tcp_client;
mod tiles;

use aircraft_db::AircraftDatabase;
use aircraft_types::AircraftTypeDatabase;
use aircraft_metadata::MetadataService;
use aviation_data::{AviationData, Airport, Navaid};
use basestation::Aircraft;
use carto_tiles::CartoTileSource;
use clap::Parser;
use eframe::egui;
use photo_cache::PhotoTextureManager;
use status::{SystemStatus, DiagnosticLevel};
use status_pane::StatusPane;
use std::sync::{Arc, Mutex};
use serde::Deserialize;
use tiles::{TileManager, WebMercator};
use config::DEFAULT_SERVER_ADDRESS;
use walkers::{HttpTiles, MapMemory, HttpOptions, lat_lon};

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
        default_value = DEFAULT_SERVER_ADDRESS,
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
    // Initialize logging
    env_logger::init();

    // Load configuration from disk (or create default if it doesn't exist)
    let mut config = match config::AppConfig::load() {
        Ok(cfg) => {
            println!("Configuration loaded successfully");
            cfg
        }
        Err(e) => {
            eprintln!("Warning: Failed to load config: {}. Using defaults.", e);
            config::AppConfig::default()
        }
    };

    // Parse command-line arguments (these override config file)
    let args = CliArgs::parse();

    // CLI args override config file
    if args.server != DEFAULT_SERVER_ADDRESS {
        // User provided a non-default server via CLI
        // Replace the first server or add if none exist
        if let Some(first_server) = config.servers.first_mut() {
            first_server.address = args.server.clone();
            first_server.enabled = true;
        } else {
            config.servers.push(config::ServerConfig::new(
                "CLI Server".to_string(),
                args.server.clone(),
                true,
            ));
        }
    }

    println!("Starting AirJedi Desktop...");

    // Display config file path
    if let Ok(config_path) = config::AppConfig::get_config_path() {
        println!("Config file: {}", config_path.display());
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1400.0, 800.0])
            .with_title("AirJedi Desktop"),
        ..Default::default()
    };

    println!("Initializing window...");
    eframe::run_native(
        "AirJedi Desktop",
        options,
        Box::new(move |cc| {
            println!("Creating application...");
            Ok(Box::new(AirjediApp::new(config, &cc.egui_ctx)))
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
                let (r, g, b) = AirjediApp::altitude_to_color(Some(alt));
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

struct AirjediApp {
    connection_manager: Arc<Mutex<connection_manager::ConnectionManager>>,
    map_center_lat: f64,
    map_center_lon: f64,
    receiver_lat: f64,
    receiver_lon: f64,
    map_zoom_level: f32, // Float for smoother pinch-zoom
    // Loading screen
    logo_texture: Option<egui::TextureHandle>,
    // Walkers tile management
    http_tiles: HttpTiles,
    map_memory: MapMemory,
    // Legacy tile manager (will be removed)
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
    // Application configuration
    config: config::AppConfig,
    // Server UI edit state (server_id -> (name, address))
    server_edit_state: std::collections::HashMap<String, (String, String)>,
    // UI window state
    show_map_overlays_window: bool,
    show_settings_window: bool,
    show_filters_window: bool,
    // Aircraft list panel state
    aircraft_list_expanded: bool,
    aircraft_list_width: f32,
    // Store panel rect from previous frame to detect pointer position before rendering
    aircraft_list_rect: Option<egui::Rect>,
    // Smoothed scroll zoom velocity for jitter-free zooming
    scroll_zoom_velocity: f32,
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

impl AirjediApp {
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

    // Load the AirJedi logo SVG for the loading screen
    fn load_logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
        // Read SVG file from disk
        let svg_path = "assets/airjedi.svg";
        match std::fs::read(svg_path) {
            Ok(svg_bytes) => {
                // Parse SVG tree
                let opt = usvg::Options::default();
                match usvg::Tree::from_data(&svg_bytes, &opt) {
                    Ok(tree) => {
                        // Render at high resolution (2048x2048) for crisp scaling
                        let target_size = 2048;
                        let svg_size = tree.size();

                        // Calculate scale to fit SVG into target size while preserving aspect ratio
                        let scale = (target_size as f32 / svg_size.width().max(svg_size.height())).min(target_size as f32);

                        let width = (svg_size.width() * scale) as u32;
                        let height = (svg_size.height() * scale) as u32;

                        // Create pixmap for rendering
                        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).unwrap();

                        // Render SVG to pixmap
                        let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
                        resvg::render(&tree, transform, &mut pixmap.as_mut());

                        // Convert pixmap to egui ColorImage
                        let pixels = pixmap.pixels();
                        let rgba_pixels: Vec<egui::Color32> = pixels.iter()
                            .map(|p| egui::Color32::from_rgba_premultiplied(p.red(), p.green(), p.blue(), p.alpha()))
                            .collect();

                        let color_image = egui::ColorImage {
                            size: [width as usize, height as usize],
                            source_size: egui::vec2(width as f32, height as f32),
                            pixels: rgba_pixels,
                        };

                        // Upload as texture with LINEAR filtering for smooth scaling
                        let texture = ctx.load_texture(
                            "airjedi_logo",
                            color_image,
                            egui::TextureOptions::LINEAR
                        );
                        println!("Logo SVG loaded successfully at {}x{}", width, height);
                        Some(texture)
                    }
                    Err(e) => {
                        eprintln!("Failed to parse SVG tree: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to read logo SVG from {}: {}", svg_path, e);
                None
            }
        }
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

    /// Convert HSL to RGB (hue 0-360, saturation 0-1, lightness 0-1)
    fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> (u8, u8, u8) {
        let c = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
        let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
        let m = lightness - c / 2.0;

        let (r, g, b) = match hue {
            h if h < 60.0 => (c, x, 0.0),
            h if h < 120.0 => (x, c, 0.0),
            h if h < 180.0 => (0.0, c, x),
            h if h < 240.0 => (0.0, x, c),
            h if h < 300.0 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };

        (
            ((r + m) * 255.0) as u8,
            ((g + m) * 255.0) as u8,
            ((b + m) * 255.0) as u8,
        )
    }

    fn new(config: config::AppConfig, egui_ctx: &egui::Context) -> Self {
        println!("Initializing ADSB app...");

        // Load logo for loading screen
        let logo_texture = Self::load_logo_texture(egui_ctx);

        // Initialize core structures
        let system_status = Arc::new(Mutex::new(SystemStatus::new()));

        // Initialize ConnectionManager (connections will be started in startup sequence)
        let connection_manager = Arc::new(Mutex::new(
            connection_manager::ConnectionManager::new(system_status.clone(), 37.7749, -122.4194)
        ));
        let aviation_data = Arc::new(Mutex::new(AviationData::new()));
        let aviation_data_loading = Arc::new(Mutex::new(true));
        let aircraft_db = Arc::new(Mutex::new(AircraftDatabase::new()));
        let aircraft_types = Arc::new(Mutex::new(AircraftTypeDatabase::new()));
        let metadata_service = Arc::new(MetadataService::new());
        let photo_manager = PhotoTextureManager::new();

        // Initialize Walkers tile management
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from(".cache"))
            .join("airjedi-desktop")
            .join("tiles");

        let http_options = HttpOptions {
            cache: Some(cache_dir),
            ..Default::default()
        };

        let http_tiles = HttpTiles::with_options(CartoTileSource, http_options, egui_ctx.clone());

        // Initialize MapMemory with configured default zoom level
        let mut map_memory = MapMemory::default();
        if let Err(e) = map_memory.set_zoom(config.default_zoom as f64) {
            eprintln!("Warning: Failed to set default zoom level: {:?}", e);
        }

        // Load aircraft type database from CSV file
        if let Err(e) = aircraft_types.lock().unwrap().load_from_file("data/aircraft.csv") {
            eprintln!("Warning: Failed to load aircraft types: {}", e);
        }

        // Use default location initially - will be updated during startup sequence
        let default_lat = 37.7749;
        let default_lon = -122.4194;

        // Add initial startup diagnostic
        system_status.lock().unwrap().add_diagnostic(
            DiagnosticLevel::Info,
            "Starting AirJedi Desktop...".to_string()
        );

        // Parse airport filter from config
        let airport_filter = match config.airport_filter.as_str() {
            "All" => AirportFilter::All,
            "MajorOnly" => AirportFilter::MajorOnly,
            _ => AirportFilter::FrequentlyUsed, // Default fallback
        };

        println!("App structure initialized - startup will continue in first frames");

        Self {
            connection_manager,
            map_center_lat: default_lat,
            map_center_lon: default_lon,
            receiver_lat: default_lat,
            receiver_lon: default_lon,
            map_zoom_level: config.default_zoom,
            logo_texture,
            http_tiles,
            map_memory,
            tile_manager: TileManager::new(),
            tile_error: None,
            selected_aircraft: None,
            previous_selected_aircraft: None,
            aviation_data,
            aviation_data_loading,
            show_airports: config.show_airports,
            show_runways: config.show_runways,
            show_navaids: config.show_navaids,
            time_limited_trails: config.time_limited_trails,
            airport_filter,
            cached_bounds: None,
            last_bounds_zoom: 0.0,
            last_bounds_center: (0.0, 0.0),
            cached_aviation_data: None,
            last_aviation_cache_bounds: None,
            last_aviation_cache_filter: airport_filter,
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
            config: config.clone(),
            server_edit_state: std::collections::HashMap::new(),
            show_map_overlays_window: false,
            show_settings_window: false,
            show_filters_window: false,
            aircraft_list_expanded: config.aircraft_list_expanded,
            aircraft_list_width: config.aircraft_list_width,
            aircraft_list_rect: None,
            scroll_zoom_velocity: 0.0,
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
        let connection_manager = self.connection_manager.clone();
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
                if let Some(aircraft) = connection_manager.lock().unwrap().get_aircraft_by_icao(&icao) {
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

                // Remove from pending
                pending_metadata.lock().unwrap().remove(&icao);
            });
        });
    }

    fn draw_aircraft_list(&mut self, ui: &mut egui::Ui) {
        // Get aircraft list with cheap Arc clones - no expensive deep copying!
        let aircraft_data: Vec<Aircraft> = {
            let connection_manager = self.connection_manager.lock()
                .expect("Connection manager mutex poisoned");
            connection_manager.get_all_aircraft_merged()  // Returns Vec<Aircraft> merged from all servers
        };

        let total_count = aircraft_data.len();

        // Military-style header with collapse button
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("◈ CONTACT LIST")
                    .color(egui::Color32::from_rgb(100, 200, 100))
                    .size(14.0)
                    .strong());

                // Collapse button on the right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let collapse_button = egui::Button::new("▶")
                        .fill(egui::Color32::from_rgba_unmultiplied(45, 50, 55, 150))
                        .frame(false);

                    if ui.add(collapse_button).clicked() {
                        self.aircraft_list_expanded = false;
                    }
                });
            });
        });

        ui.add_space(2.0);

        // Sort Controls (Filters moved to separate window)
        egui::CollapsingHeader::new(egui::RichText::new("⚙ SORT")
            .color(egui::Color32::from_rgb(100, 200, 200))
            .size(11.0)
            .strong())
            .default_open(false)
            .show(ui, |ui| {
                // Sort by
                ui.label(egui::RichText::new("Sort By")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(9.0));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.sort_by, SortCriterion::Altitude, "Altitude");
                    ui.radio_value(&mut self.sort_by, SortCriterion::Speed, "Speed");
                    ui.radio_value(&mut self.sort_by, SortCriterion::Range, "Range");
                });

                ui.add_space(2.0);

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

        let _scroll_area = egui::ScrollArea::vertical()
            .auto_shrink([false, false]) // Don't shrink, always take full space
            .show(ui, |ui| {
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
                        let mut photo_double_clicked = false; // Track if photo was double-clicked
                        let mut photo_single_clicked = false; // Track if photo was single-clicked
                        let mut photo_rect = egui::Rect::NOTHING; // Track photo position
                        let mut text_clicked = false; // Track if text area was clicked

                        ui.horizontal(|ui| {
                            // Left column: All text information (make this clickable for card selection)
                            let text_response = ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;

                                // Row 1: Status + ICAO + Callsign + Altitude
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(status_symbol)
                                        .color(status_color)
                                        .size(11.0));

                                    ui.label(egui::RichText::new(&icao)
                                        .color(egui::Color32::from_rgb(200, 220, 255))
                                        .size(10.5)
                                        .monospace()
                                        .strong());

                                    if let Some(ref callsign) = aircraft.callsign() {
                                        let callsign_color = if is_selected {
                                            egui::Color32::from_rgb(255, 50, 50)
                                        } else {
                                            egui::Color32::from_rgb(150, 220, 150)
                                        };
                                        ui.label(egui::RichText::new(format!("│ {}", callsign.trim()))
                                            .color(callsign_color)
                                            .size(10.5)
                                            .strong());

                                        // Server badge - show which server this aircraft came from
                                        let server_name = aircraft.source_server_name();
                                        if !server_name.is_empty() {
                                            // Generate consistent color based on server name hash
                                            let hash = server_name.bytes().fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
                                            let hue = (hash % 360) as f32;
                                            let (r, g, b) = Self::hsl_to_rgb(hue, 0.6, 0.5);

                                            ui.label(egui::RichText::new(format!("[{}]", server_name))
                                                .color(egui::Color32::from_rgb(r, g, b))
                                                .size(7.0))
                                                .on_hover_text(format!("Source: {}", server_name));
                                        }
                                    }

                                    if let Some(alt) = aircraft.altitude() {
                                        let alt_text = if alt >= 18000 {
                                            format!("│ {} FL{:03}", alt_indicator, alt / 100)
                                        } else {
                                            format!("│ {} {}", alt_indicator, alt)
                                        };
                                        ui.label(egui::RichText::new(alt_text)
                                            .color(alt_color)
                                            .size(9.5)
                                            .monospace());
                                    }
                                });

                                // Row 2: Flight data (speed, heading, range) + metadata
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 6.0;

                                    if let Some(vel) = aircraft.velocity() {
                                        ui.label(egui::RichText::new(format!("{:03}kt", vel as i32))
                                            .color(egui::Color32::from_rgb(170, 170, 170))
                                            .size(8.0)
                                            .monospace());
                                    }

                                    if let Some(track) = aircraft.track() {
                                        ui.label(egui::RichText::new(format!("{:03}°", track as i32))
                                            .color(egui::Color32::from_rgb(170, 170, 170))
                                            .size(8.0)
                                            .monospace());
                                    }

                                    if let Some(range) = aircraft.distance_from_nm(self.receiver_lat, self.receiver_lon) {
                                        ui.label(egui::RichText::new(format!("{:.1}nm", range))
                                            .color(egui::Color32::from_rgb(100, 200, 255))
                                            .size(8.0)
                                            .monospace());
                                    }
                                });

                                // Row 3: Registration + Aircraft Type + Timestamp
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;

                                    if let Some(ref registration) = aircraft.registration() {
                                        ui.label(egui::RichText::new(registration)
                                            .color(egui::Color32::from_rgb(140, 170, 190))
                                            .size(7.5)
                                            .monospace());
                                    }

                                    if let Some(ref aircraft_type) = aircraft.aircraft_type() {
                                        let type_display = if let Ok(type_db) = self.aircraft_types.lock() {
                                            let full_name = type_db.lookup(aircraft_type)
                                                .unwrap_or(aircraft_type.as_str());
                                            // Truncate if too long (keep first 18 chars for tighter fit)
                                            if full_name.len() > 18 {
                                                format!("{}…", &full_name[..17])
                                            } else {
                                                full_name.to_string()
                                            }
                                        } else {
                                            aircraft_type.clone()
                                        };

                                        if aircraft.registration().is_some() {
                                            ui.label(egui::RichText::new(format!("│ {}", type_display))
                                                .color(egui::Color32::from_rgb(170, 140, 190))
                                                .size(7.5));
                                        } else {
                                            ui.label(egui::RichText::new(type_display)
                                                .color(egui::Color32::from_rgb(170, 140, 190))
                                                .size(7.5));
                                        }
                                    }

                                    // Add timestamp with appropriate separator
                                    let has_metadata = aircraft.registration().is_some() || aircraft.aircraft_type().is_some();
                                    if has_metadata {
                                        ui.label(egui::RichText::new(format!("│ {}s", seconds_ago))
                                            .color(egui::Color32::from_rgb(100, 100, 100))
                                            .size(7.5)
                                            .monospace());
                                    } else {
                                        ui.label(egui::RichText::new(format!("{}s", seconds_ago))
                                            .color(egui::Color32::from_rgb(100, 100, 100))
                                            .size(7.5)
                                            .monospace());
                                    }
                                });
                            }); // Close left column

                            // Make text area clickable for card selection
                            let text_click_response = ui.interact(
                                text_response.response.rect,
                                ui.id().with(format!("{}_text", icao)),
                                egui::Sense::click()
                            );

                            if text_click_response.clicked() {
                                text_clicked = true;
                            }

                            // Flexible spacer to push photo to the right
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                                // Right column: Photo at top corner - full card height (clickable link to Planespotters)
                                let texture = if let Some(ref photo_url) = aircraft.photo_thumbnail_url() {
                                    self.photo_manager.get_or_load_texture(ui.ctx(), &photo_url, &icao)
                                } else {
                                    None
                                };

                                // Larger photo to span full card height (80×60 from 48×32)
                                // Make it double-clickable to open Planespotters page
                                if let Some(tex) = texture {
                                    let image_response = ui.add(
                                        egui::Image::new((tex.id(), egui::vec2(80.0, 60.0)))
                                            .sense(egui::Sense::click())
                                    ).on_hover_text("Double-click to view on Planespotters.net");

                                    // Store photo rectangle
                                    photo_rect = image_response.rect;

                                    // Add hover cursor
                                    if image_response.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }

                                    // Check for double-click FIRST (before single-click)
                                    if image_response.double_clicked() {
                                        let url = format!("https://www.planespotters.net/hex/{}", icao);
                                        if let Err(e) = webbrowser::open(&url) {
                                            eprintln!("Failed to open browser: {}", e);
                                        }
                                        photo_double_clicked = true;
                                    } else if image_response.clicked() {
                                        photo_single_clicked = true;
                                    }
                                } else if let Some(placeholder) = self.photo_manager.get_placeholder() {
                                    let image_response = ui.add(
                                        egui::Image::new((placeholder.id(), egui::vec2(80.0, 60.0)))
                                            .sense(egui::Sense::click())
                                    ).on_hover_text("Double-click to view on Planespotters.net");

                                    // Store photo rectangle
                                    photo_rect = image_response.rect;

                                    // Add hover cursor for placeholder
                                    if image_response.hovered() {
                                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                                    }

                                    // Check for double-click FIRST (before single-click)
                                    if image_response.double_clicked() {
                                        let url = format!("https://www.planespotters.net/hex/{}", icao);
                                        if let Err(e) = webbrowser::open(&url) {
                                            eprintln!("Failed to open browser: {}", e);
                                        }
                                        photo_double_clicked = true;
                                    } else if image_response.clicked() {
                                        photo_single_clicked = true;
                                    }
                                }
                            });
                        }); // Close horizontal layout

                        (photo_double_clicked, photo_single_clicked, photo_rect, text_clicked) // Return all interaction state
                    });

                    // Extract interaction state
                    let (photo_was_double_clicked, photo_was_single_clicked, _photo_rect, text_was_clicked) = inner_response.inner;

                    // Handle click to select this aircraft
                    // Select if: single-click on photo, OR click on text area (but not double-click on photo)
                    if !photo_was_double_clicked {
                        if photo_was_single_clicked || text_was_clicked {
                            self.selected_aircraft = Some(icao.clone());
                        }
                    }

                    // Auto-scroll to selected aircraft if it's a new selection
                    if is_selected && self.previous_selected_aircraft.as_ref() != Some(&icao) {
                        inner_response.response.scroll_to_me(Some(egui::Align::Center));
                    }

                    ui.add_space(1.0); // Reduced from 3.0 for more compact list
                }
            });
        });

        // The scroll area automatically consumes scroll events when pointer is over it
        // Combined with the panel's input blocking layer, this prevents map zoom/pan conflicts
    }

    fn draw_loading_screen(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        // Allocate full screen space
        let (_, painter) = ui.allocate_painter(
            egui::vec2(ui.available_width(), ui.available_height()),
            egui::Sense::hover(),
        );

        let rect = ui.max_rect();
        let center = rect.center();

        // Draw dark background matching map theme
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(15, 18, 20));

        // Draw logo if loaded
        if let Some(ref logo) = self.logo_texture {
            // Get time for animation
            let time = ctx.input(|i| i.time);

            // Pulsing/breathing animation - smooth sinusoidal scale
            // Cycle: 2 seconds, scale range: 1.0 -> 1.1 -> 1.0
            let pulse = (time * std::f64::consts::PI).sin(); // -1 to 1
            let scale = 1.0 + 0.05 * pulse; // 0.95 to 1.05

            // Logo size - scale based on screen size but keep reasonable
            let logo_base_size = rect.height().min(rect.width()) * 0.3; // 30% of smallest dimension
            let logo_size = logo_base_size * scale as f32;

            // Center the logo
            let logo_rect = egui::Rect::from_center_size(
                center,
                egui::vec2(logo_size, logo_size),
            );

            // Draw the logo texture with opacity for polish
            painter.image(
                logo.id(),
                logo_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // Display loading status text below logo
        let status_text = match self.startup_state {
            StartupState::InitializingWindow => "Initializing...",
            StartupState::DetectingLocation => "Detecting location...",
            StartupState::StartingTcpClient => "Starting ADS-B client...",
            StartupState::LoadingAviationData => "Loading aviation data...",
            StartupState::LoadingAircraftDB => "Loading aircraft database...",
            StartupState::Complete => "Ready",
        };

        let text_pos = center + egui::vec2(0.0, rect.height() * 0.2);
        painter.text(
            text_pos,
            egui::Align2::CENTER_CENTER,
            status_text,
            egui::FontId::proportional(16.0),
            egui::Color32::from_rgb(150, 180, 200),
        );

        // App name/title above the logo
        let title_pos = center - egui::vec2(0.0, rect.height() * 0.25);
        painter.text(
            title_pos,
            egui::Align2::CENTER_CENTER,
            "AirJedi Desktop",
            egui::FontId::proportional(24.0),
            egui::Color32::from_rgb(100, 200, 200),
        );
    }

    fn draw_map(&mut self, ui: &mut egui::Ui) {
        // Check if pointer is over the aircraft list panel (using rect from previous frame)
        let pointer_over_panel = if let Some(panel_rect) = self.aircraft_list_rect {
            ui.ctx().input(|i| {
                i.pointer.hover_pos().map_or(false, |pos| panel_rect.contains(pos))
            })
        } else {
            false
        };

        // Sync zoom level from MapMemory
        self.map_zoom_level = self.map_memory.zoom() as f32;

        // Auto-pan to newly selected aircraft
        if let Some(ref selected_icao) = self.selected_aircraft {
            let is_new_selection = self.previous_selected_aircraft.as_ref() != Some(selected_icao);

            if is_new_selection {
                self.following_aircraft = false;
                self.stored_map_center = None;
            }

            if is_new_selection && !self.following_aircraft {
                let aircraft_opt = {
                    let connection_manager = self.connection_manager.lock().unwrap();
                    connection_manager.get_aircraft_by_icao(selected_icao)
                };

                if let Some(aircraft) = aircraft_opt {
                    if let (Some(lat), Some(lon)) = (aircraft.latitude(), aircraft.longitude()) {
                        self.map_memory.center_at(lat_lon(lat, lon));
                        self.stored_map_center = Some((lat, lon));
                        self.following_aircraft = true;
                    }
                }
            }
        }

        self.previous_selected_aircraft = self.selected_aircraft.clone();
        self.hovered_map_item = None;

        // PREPARE DATA BEFORE RENDERING (can't mutate self inside closure)
        // Calculate viewport bounds using simple approximation
        let tile_zoom_level = self.map_zoom_level.round() as u8;
        let tile_pixel_size = 256.0;
        let scale = 2.0_f64.powf(tile_zoom_level as f64);

        // Approximate bounds (will be refined by Walkers)
        let viewport_size = ui.available_size();
        let half_viewport_width = (viewport_size.x as f64) / 2.0;
        let half_viewport_height = (viewport_size.y as f64) / 2.0;
        let degrees_per_pixel_lon = 360.0 / (tile_pixel_size * scale);
        let degrees_per_pixel_lat = 180.0 / (tile_pixel_size * scale);
        let padding_multiplier = 1.5;
        let lon_range = (half_viewport_width * degrees_per_pixel_lon) * padding_multiplier;
        let lat_range = (half_viewport_height * degrees_per_pixel_lat) * padding_multiplier;

        // Get map center from MapMemory (use x() for lon, y() for lat since Point is in (lon, lat) order)
        let map_position = self.map_memory.detached().unwrap_or_else(|| lat_lon(self.receiver_lat, self.receiver_lon));
        let map_center_lat = map_position.y();
        let map_center_lon = map_position.x();

        let min_lat = (map_center_lat - lat_range).max(-85.0);
        let max_lat = (map_center_lat + lat_range).min(85.0);
        let min_lon = map_center_lon - lon_range;
        let max_lon = map_center_lon + lon_range;

        // Update aviation data cache if needed
        let bounds_changed_significantly = if let Some((last_min_lat, last_max_lat, last_min_lon, last_max_lon)) = self.last_aviation_cache_bounds {
            let lat_threshold = (last_max_lat - last_min_lat) * 0.1;
            let lon_threshold = (last_max_lon - last_min_lon) * 0.1;
            (min_lat - last_min_lat).abs() > lat_threshold
                || (max_lat - last_max_lat).abs() > lat_threshold
                || (min_lon - last_min_lon).abs() > lon_threshold
                || (max_lon - last_max_lon).abs() > lon_threshold
        } else {
            true
        };

        let cache_needs_update = self.cached_aviation_data.is_none()
            || bounds_changed_significantly
            || self.last_aviation_cache_filter != self.airport_filter;

        if cache_needs_update {
            if let Ok(aviation_data) = self.aviation_data.lock() {
                let airports: Vec<_> = aviation_data.get_airports_in_bounds(min_lat, max_lat, min_lon, max_lon)
                    .into_iter()
                    .cloned()
                    .collect();

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

                self.cached_aviation_data = Some((airports, runways, navaids));
                self.last_aviation_cache_bounds = Some((min_lat, max_lat, min_lon, max_lon));
                self.last_aviation_cache_filter = self.airport_filter;
            }
        }

        // Get references to cached data
        let (visible_airports, airport_runways, visible_navaids) = if let Some((ref airports, ref runways, ref navaids)) = self.cached_aviation_data {
            (airports, runways, navaids)
        } else {
            (&Vec::new(), &Vec::new(), &Vec::new())
        };

        // Get aircraft list
        let aircraft_list: Vec<Aircraft> = {
            let connection_manager = self.connection_manager.lock().unwrap();
            connection_manager.get_all_aircraft_merged()
        };

        // Get trail settings
        let time_limited_trails = self.connection_manager.lock().unwrap().get_time_limited_trails();

        // Capture values needed inside closure
        let show_airports = self.show_airports;
        let show_runways = self.show_runways;
        let show_navaids = self.show_navaids;
        let airport_filter = self.airport_filter;
        let selected_aircraft = self.selected_aircraft.clone();
        let receiver_lat = self.receiver_lat;
        let receiver_lon = self.receiver_lon;

        // Handle scroll events: either for map zoom or for panel scrolling
        let scroll_delta;
        let (saved_smooth_scroll, saved_raw_scroll);

        if pointer_over_panel {
            // Pointer over panel - save scroll for panel, block from map
            let saved = ui.ctx().input(|i| {
                (i.smooth_scroll_delta, i.raw_scroll_delta)
            });

            ui.ctx().input_mut(|i| {
                i.smooth_scroll_delta = egui::Vec2::ZERO;
                i.raw_scroll_delta = egui::Vec2::ZERO;
            });

            scroll_delta = egui::Vec2::ZERO;
            saved_smooth_scroll = Some(saved.0);
            saved_raw_scroll = Some(saved.1);
        } else {
            // Pointer over map - capture scroll for our custom zoom, block from Map widget
            scroll_delta = ui.ctx().input(|i| i.smooth_scroll_delta);

            // Consume scroll events to prevent Map widget from also processing them
            // This ensures only our cursor-centered zoom logic runs
            ui.ctx().input_mut(|i| {
                i.smooth_scroll_delta = egui::Vec2::ZERO;
                i.raw_scroll_delta = egui::Vec2::ZERO;
            });

            saved_smooth_scroll = None;
            saved_raw_scroll = None;
        }

        // Store zoom level before map widget processes gestures
        // This allows us to detect and slow down pinch-to-zoom
        let zoom_before_map = self.map_memory.zoom();

        // Create Walkers Map widget
        let receiver_position = lat_lon(receiver_lat, receiver_lon);

        use walkers::Map;

        // Variable to track hovered items inside the map closure
        let mut detected_hover: Option<HoveredMapItem> = None;
        // Variable to track clicked aircraft
        let mut clicked_aircraft_icao: Option<String> = None;

        let map_response = Map::new(
            Some(&mut self.http_tiles),
            &mut self.map_memory,
            receiver_position,
        )
        .show(ui, |ui, projector, map_memory| {
            let painter = ui.painter();
            let rect = ui.max_rect();
            let map_zoom_level = map_memory.zoom() as f32;
            let hover_pos = ui.input(|i| i.pointer.hover_pos());

            // Detect clicks on the map
            let click_pos = ui.input(|i| {
                if i.pointer.primary_clicked() {
                    i.pointer.interact_pos()
                } else {
                    None
                }
            });

            // Helper function using Walkers Projector
            let to_screen = |lat: f64, lon: f64| -> egui::Pos2 {
                let pos = lat_lon(lat, lon);
                let screen_pos = projector.project(pos);
                egui::pos2(screen_pos.x, screen_pos.y)
            };

            // Draw receiver location marker
            let receiver_pos = to_screen(receiver_lat, receiver_lon);
            if rect.contains(receiver_pos) {
                painter.circle_filled(receiver_pos, 6.0, egui::Color32::from_rgb(100, 200, 100));
                painter.circle_stroke(
                    receiver_pos,
                    6.0,
                    egui::Stroke::new(2.0, egui::Color32::from_rgb(200, 255, 200)),
                );
            }

            // Draw aviation overlays
            // Runways (draw first, under airports)
            if show_runways && map_zoom_level >= 9.5 {
                let max_runways = if map_zoom_level >= 11.0 { usize::MAX } else { 500 };
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

            // Airports with LOD optimization
            if show_airports {
                let max_airports = if map_zoom_level >= 10.0 {
                    usize::MAX
                } else if map_zoom_level >= 9.0 {
                    1000
                } else if map_zoom_level >= 8.0 {
                    500
                } else {
                    200
                };

                let mut airports_drawn = 0;
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

                    let should_show = match airport_filter {
                        AirportFilter::All => {
                            airport.is_public_airplane_airport() &&
                            (airport.is_major() || airport.is_medium() || map_zoom_level >= 9.5)
                        }
                        AirportFilter::FrequentlyUsed => {
                            airport.is_frequently_used()
                        }
                        AirportFilter::MajorOnly => {
                            airport.is_major()
                        }
                    };

                    if !should_show {
                        continue;
                    }

                    let pos = to_screen(airport.latitude, airport.longitude);

                    if rect.contains(pos) {
                        let airport_color = if airport.is_major() {
                            egui::Color32::from_rgb(200, 100, 100)
                        } else if airport.is_medium() {
                            egui::Color32::from_rgb(150, 150, 100)
                        } else {
                            egui::Color32::from_rgb(120, 120, 120)
                        };

                        painter.circle_filled(pos, airport.render_radius(), airport_color);
                        painter.circle_stroke(
                            pos,
                            airport.render_radius(),
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(255, 255, 255)),
                        );

                        if map_zoom_level >= 9.0 {
                            painter.text(
                                pos + egui::vec2(0.0, -12.0),
                                egui::Align2::CENTER_BOTTOM,
                                &airport.icao,
                                egui::FontId::proportional(9.0),
                                egui::Color32::from_rgb(220, 220, 220),
                            );
                        }

                        // Check for hover
                        if let Some(hover_pos_val) = hover_pos {
                            let distance = hover_pos_val.distance(pos);
                            let hover_radius = airport.render_radius() + 8.0;
                            if distance <= hover_radius {
                                detected_hover = Some(HoveredMapItem::Airport(airport.clone()));
                            }
                        }

                        airports_drawn += 1;
                    }
                }
            }

            // Navaids
            if show_navaids && map_zoom_level >= 9.0 {
                let max_navaids = if map_zoom_level >= 10.0 { 1000 } else { 300 };
                let mut navaids_drawn = 0;

                for navaid in visible_navaids {
                    if navaids_drawn >= max_navaids {
                        break;
                    }

                    let pos = to_screen(navaid.latitude, navaid.longitude);

                    if rect.contains(pos) {
                        let (r, g, b) = navaid.get_color();
                        let navaid_color = egui::Color32::from_rgb(r, g, b);
                        let size = navaid.symbol_size();

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

                        if map_zoom_level >= 10.0 {
                            painter.text(
                                pos + egui::vec2(0.0, size + 8.0),
                                egui::Align2::CENTER_TOP,
                                &navaid.ident,
                                egui::FontId::proportional(8.0),
                                navaid_color,
                            );
                        }

                        // Check for hover
                        if let Some(hover_pos_val) = hover_pos {
                            let distance = hover_pos_val.distance(pos);
                            let hover_radius = size + 8.0;
                            if distance <= hover_radius {
                                detected_hover = Some(HoveredMapItem::Navaid(navaid.clone()));
                            }
                        }

                        navaids_drawn += 1;
                    }
                }
            }

            // Aircraft trails with LOD
            let trail_detail_level = if map_zoom_level >= 10.0 {
                1
            } else if map_zoom_level >= 9.0 {
                2
            } else {
                4
            };

            for aircraft in &aircraft_list {
                aircraft.with_data(|data| {
                    if data.position_history.is_empty() {
                        return;
                    }

                    if let (Some(lat), Some(lon)) = (data.latitude, data.longitude) {
                        let screen_pos = to_screen(lat, lon);
                        let margin = 100.0;
                        let expanded_rect = rect.expand(margin);

                        if !expanded_rect.contains(screen_pos) {
                            return;
                        }
                    }

                    let now = chrono::Utc::now();
                    let mut points_drawn = 0;

                    for i in (0..data.position_history.len()).step_by(trail_detail_level) {
                        let point = &data.position_history[i];
                        let age = (now - point.timestamp).num_milliseconds() as f32 / 1000.0;

                        if time_limited_trails && age > TRAIL_MAX_AGE_SECONDS {
                            continue;
                        }

                        let alpha = if time_limited_trails {
                            if age <= TRAIL_SOLID_DURATION_SECONDS {
                                255
                            } else {
                                let fade_age = age - TRAIL_SOLID_DURATION_SECONDS;
                                let opacity = (1.0 - (fade_age / TRAIL_FADE_DURATION_SECONDS)).clamp(0.0, 1.0);
                                (opacity * 255.0) as u8
                            }
                        } else {
                            255
                        };

                        let trail_pos = to_screen(point.lat, point.lon);
                        let next_idx = i + trail_detail_level;

                        if next_idx < data.position_history.len() {
                            let next_point = &data.position_history[next_idx];
                            let next_age = (now - next_point.timestamp).num_milliseconds() as f32 / 1000.0;

                            if !time_limited_trails || next_age <= TRAIL_MAX_AGE_SECONDS {
                                let next_pos = to_screen(next_point.lat, next_point.lon);
                                let (r, g, b) = Self::altitude_to_color(point.altitude);
                                let trail_color = egui::Color32::from_rgba_unmultiplied(r, g, b, alpha);
                                painter.line_segment(
                                    [trail_pos, next_pos],
                                    egui::Stroke::new(2.0, trail_color)
                                );
                                points_drawn += 1;
                            }
                        }

                        if points_drawn >= 100 && map_zoom_level < 9.0 {
                            break;
                        }
                    }

                    // Line from last history to current position
                    if let (Some(lat), Some(lon)) = (data.latitude, data.longitude) {
                        if let Some(last_point) = data.position_history.last() {
                            let last_pos = to_screen(last_point.lat, last_point.lon);
                            let current_pos = to_screen(lat, lon);
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

            // Aircraft icons and labels
            for aircraft in &aircraft_list {
                if let (Some(lat), Some(lon)) = (aircraft.latitude(), aircraft.longitude()) {
                    let pos = to_screen(lat, lon);

                    if rect.contains(pos) {
                        let icao = aircraft.icao();
                        let is_selected = selected_aircraft.as_ref() == Some(&icao);

                        let (color, size) = if is_selected {
                            (egui::Color32::from_rgb(255, 100, 100), 7.0)
                        } else {
                            (egui::Color32::from_rgb(120, 220, 120), 5.0)
                        };

                        let track = aircraft.track().unwrap_or(0.0) as f32;
                        Self::draw_aircraft_icon(&painter, pos, track, color, size);

                        if is_selected {
                            painter.circle_stroke(
                                pos,
                                size * 1.8,
                                egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 200, 50)),
                            );
                        }

                        // Callsign label with background
                        let mut label_offset_y = -10.0;
                        if let Some(ref callsign) = aircraft.callsign() {
                            let text = callsign.trim();
                            let text_pos = pos + egui::vec2(10.0, label_offset_y);
                            let galley = painter.layout_no_wrap(
                                text.to_string(),
                                egui::FontId::proportional(11.0),
                                egui::Color32::WHITE,
                            );
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
                            painter.text(
                                text_pos,
                                egui::Align2::LEFT_CENTER,
                                text,
                                egui::FontId::proportional(11.0),
                                egui::Color32::WHITE,
                            );
                            label_offset_y += 14.0;
                        }

                        // Altitude label
                        if let Some(alt) = aircraft.altitude() {
                            let alt_text = if alt >= 18000 {
                                format!("FL{:03}", alt / 100)
                            } else {
                                format!("{}ft", alt)
                            };
                            let text_pos = pos + egui::vec2(10.0, label_offset_y);
                            let galley = painter.layout_no_wrap(
                                alt_text.clone(),
                                egui::FontId::proportional(10.0),
                                egui::Color32::from_rgb(200, 200, 200),
                            );
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
                            painter.text(
                                text_pos,
                                egui::Align2::LEFT_CENTER,
                                &alt_text,
                                egui::FontId::proportional(10.0),
                                egui::Color32::from_rgb(200, 200, 200),
                            );
                        }

                        // Check for hover on aircraft
                        if let Some(hover_pos_val) = hover_pos {
                            let distance = hover_pos_val.distance(pos);
                            let hover_radius = size * 1.8 + 5.0;
                            if distance <= hover_radius {
                                detected_hover = Some(HoveredMapItem::Aircraft(aircraft.clone()));
                            }
                        }

                        // Check for click on aircraft
                        if let Some(click_pos_val) = click_pos {
                            let distance = click_pos_val.distance(pos);
                            let click_radius = size * 1.8 + 5.0;
                            if distance <= click_radius {
                                clicked_aircraft_icao = Some(icao.clone());
                            }
                        }
                    }
                }
            }

            (detected_hover, clicked_aircraft_icao)
        });

        // Update hover state and handle clicks from the map
        let (hover_result, click_result) = map_response.inner;
        self.hovered_map_item = hover_result;

        // Handle aircraft selection from map click
        if let Some(clicked_icao) = click_result {
            self.selected_aircraft = Some(clicked_icao);
        }

        // Render hover popup if hovering over a map item
        if let Some(ref hovered_item) = self.hovered_map_item {
            if let Some(hover_pos_val) = ui.input(|i| i.pointer.hover_pos()) {
                // Position popup with offset to avoid obscuring the item
                let popup_pos = hover_pos_val + egui::vec2(15.0, 10.0);

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

        // Instructions text at the top
        egui::Area::new("map_instructions".into())
            .fixed_pos(egui::pos2(10.0, 35.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                ui.label(
                    egui::RichText::new("Drag to pan | Scroll/pinch to zoom")
                        .size(12.0)
                        .color(egui::Color32::from_rgb(200, 200, 200))
                );
            });

        // Floating toolbar below the instructions
        egui::Area::new("map_toolbar".into())
            .fixed_pos(egui::pos2(10.0, 60.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::new()
                    .fill(egui::Color32::from_rgba_unmultiplied(25, 30, 35, 200))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::same(6))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;

                            // Settings button (cog icon)
                            let settings_button = egui::Button::new(
                                egui::RichText::new("⚙")
                                    .size(18.0)
                                    .color(egui::Color32::from_rgb(180, 180, 180))
                            )
                            .fill(egui::Color32::from_rgba_unmultiplied(45, 50, 55, 150));

                            if ui.add(settings_button)
                                .on_hover_text("Settings")
                                .clicked()
                            {
                                self.show_settings_window = !self.show_settings_window;
                            }

                            // Map overlays button (layers icon)
                            let overlays_button = egui::Button::new(
                                egui::RichText::new("☰")
                                    .size(18.0)
                                    .color(egui::Color32::from_rgb(180, 180, 180))
                            )
                            .fill(egui::Color32::from_rgba_unmultiplied(45, 50, 55, 150));

                            if ui.add(overlays_button)
                                .on_hover_text("Map Overlays")
                                .clicked()
                            {
                                self.show_map_overlays_window = !self.show_map_overlays_window;
                            }

                            // Filters button (funnel icon)
                            let filters_button = egui::Button::new(
                                egui::RichText::new("▼")
                                    .size(18.0)
                                    .color(egui::Color32::from_rgb(180, 180, 180))
                            )
                            .fill(egui::Color32::from_rgba_unmultiplied(45, 50, 55, 150));

                            if ui.add(filters_button)
                                .on_hover_text("Filters")
                                .clicked()
                            {
                                self.show_filters_window = !self.show_filters_window;
                            }
                        });
                    });
            });

        // Tile error/loading display at top-center
        if let Some(ref error_msg) = self.tile_error {
            let is_error = error_msg.contains("Failed");
            let bg_color = if is_error {
                egui::Color32::from_rgb(220, 50, 50)
            } else {
                egui::Color32::from_rgb(255, 200, 100)
            };

            egui::Area::new("tile_error".into())
                .fixed_pos(egui::pos2(
                    ui.max_rect().center().x,
                    ui.max_rect().top() + 20.0
                ))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    egui::Frame::new()
                        .fill(bg_color)
                        .corner_radius(5.0)
                        .inner_margin(egui::Margin::symmetric(12, 6))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(error_msg)
                                    .size(12.0)
                                    .color(egui::Color32::WHITE)
                            );
                        });
                });
        }

        // Handle smooth scroll-to-zoom with exponential smoothing and cursor-centered behavior
        if scroll_delta.y.abs() > 0.1 {
            // Apply exponential smoothing to scroll delta for smooth zoom
            // smoothing_factor: 0 = no smoothing (jittery), 1 = max smoothing (sluggish)
            let smoothing_factor = 0.7;

            // Convert scroll to zoom velocity (positive = zoom in, negative = zoom out)
            let target_velocity = scroll_delta.y / 300.0;

            // Smooth the velocity using exponential moving average
            self.scroll_zoom_velocity = self.scroll_zoom_velocity * smoothing_factor
                                       + target_velocity * (1.0 - smoothing_factor);
        } else {
            // Decay velocity when no scroll input (smooth stop)
            self.scroll_zoom_velocity *= 0.8;
        }

        // Apply smoothed scroll zoom with cursor-centered behavior using proper Web Mercator projection
        if self.scroll_zoom_velocity.abs() > 0.001 {
            // Get cursor position for zoom centering
            let cursor_pos = ui.input(|i| i.pointer.hover_pos());

            // Get current zoom and map center
            let old_zoom = self.map_memory.zoom();
            let map_position = self.map_memory.detached().unwrap_or_else(|| lat_lon(self.receiver_lat, self.receiver_lon));
            let map_center_lat = map_position.y();
            let map_center_lon = map_position.x();

            // Calculate new zoom level
            let new_zoom = (old_zoom + self.scroll_zoom_velocity as f64).clamp(6.0, 18.0);

            // If cursor is over the map, zoom centered on cursor using Web Mercator projection
            if let Some(cursor) = cursor_pos {
                // Get map widget bounds
                let map_rect = ui.max_rect();

                if map_rect.contains(cursor) {
                    // Use integer zoom levels for Web Mercator tile coordinate calculations
                    let old_zoom_int = old_zoom.round() as u8;
                    let new_zoom_int = new_zoom.round() as u8;
                    let tile_pixel_size = 256.0;

                    // Calculate cursor offset from map center in screen pixels
                    let map_center_screen = map_rect.center();
                    let cursor_offset_x = (cursor.x - map_center_screen.x) as f64;
                    let cursor_offset_y = (cursor.y - map_center_screen.y) as f64;

                    // Convert map center to tile coordinates at old zoom level
                    let old_center_tile_x = WebMercator::lon_to_x(map_center_lon, old_zoom_int);
                    let old_center_tile_y = WebMercator::lat_to_y(map_center_lat, old_zoom_int);

                    // Calculate fractional zoom for scale factor
                    let old_zoom_fraction = old_zoom - old_zoom_int as f64;
                    let old_scale_factor = 2.0_f64.powf(old_zoom_fraction);

                    // Convert cursor screen offset to tile offset at old zoom
                    let cursor_tile_offset_x = cursor_offset_x / (tile_pixel_size * old_scale_factor);
                    let cursor_tile_offset_y = cursor_offset_y / (tile_pixel_size * old_scale_factor);

                    // Get tile coordinates at cursor position (at old zoom level)
                    let cursor_tile_x = old_center_tile_x + cursor_tile_offset_x;
                    let cursor_tile_y = old_center_tile_y + cursor_tile_offset_y;

                    // Convert cursor tile coordinates back to lat/lon
                    let cursor_lat = WebMercator::tile_to_lat(cursor_tile_y, old_zoom_int);
                    let cursor_lon = WebMercator::tile_to_lon(cursor_tile_x, old_zoom_int);

                    // Apply zoom
                    if let Err(e) = self.map_memory.set_zoom(new_zoom) {
                        eprintln!("Failed to set zoom: {:?}", e);
                    }

                    // Convert cursor lat/lon to tile coordinates at NEW zoom level
                    let new_cursor_tile_x = WebMercator::lon_to_x(cursor_lon, new_zoom_int);
                    let new_cursor_tile_y = WebMercator::lat_to_y(cursor_lat, new_zoom_int);

                    // Calculate fractional zoom for new scale factor
                    let new_zoom_fraction = new_zoom - new_zoom_int as f64;
                    let new_scale_factor = 2.0_f64.powf(new_zoom_fraction);

                    // Calculate tile offset for cursor at new zoom (same screen pixels)
                    let new_cursor_tile_offset_x = cursor_offset_x / (tile_pixel_size * new_scale_factor);
                    let new_cursor_tile_offset_y = cursor_offset_y / (tile_pixel_size * new_scale_factor);

                    // Calculate new map center in tile coordinates
                    // We want cursor to stay at the same screen position, so:
                    // new_center + new_offset = cursor_tile
                    let new_center_tile_x = new_cursor_tile_x - new_cursor_tile_offset_x;
                    let new_center_tile_y = new_cursor_tile_y - new_cursor_tile_offset_y;

                    // Convert new center back to lat/lon
                    let new_center_lat = WebMercator::tile_to_lat(new_center_tile_y, new_zoom_int);
                    let new_center_lon = WebMercator::tile_to_lon(new_center_tile_x, new_zoom_int);

                    // Clamp latitude to valid range
                    let clamped_lat = new_center_lat.clamp(-85.0, 85.0);

                    // Update map center
                    self.map_memory.center_at(lat_lon(clamped_lat, new_center_lon));
                } else {
                    // Cursor not over map, just zoom normally
                    if let Err(e) = self.map_memory.set_zoom(new_zoom) {
                        eprintln!("Failed to set zoom: {:?}", e);
                    }
                }
            } else {
                // No cursor position, just zoom normally
                if let Err(e) = self.map_memory.set_zoom(new_zoom) {
                    eprintln!("Failed to set zoom: {:?}", e);
                }
            }
        }

        // Check if the map widget changed the zoom level (from pinch gesture)
        let zoom_after_map = self.map_memory.zoom();
        let map_zoom_change = zoom_after_map - zoom_before_map;

        // Handle pinch-to-zoom with reduced sensitivity
        // Only process if we didn't just apply scroll zoom (to avoid conflicts)
        if map_zoom_change.abs() > 0.001 && scroll_delta.y.abs() < 0.1 {
            // The map widget applied a zoom change from pinch gesture
            // Scale it down to 30% of original speed for more control
            let reduced_zoom_change = map_zoom_change * 0.3;
            let final_zoom = zoom_before_map + reduced_zoom_change;

            // Apply final zoom level (clamped to reasonable range)
            let clamped_zoom = final_zoom.clamp(6.0, 18.0);
            if let Err(e) = self.map_memory.set_zoom(clamped_zoom) {
                eprintln!("Failed to set zoom: {:?}", e);
            }
        }

        // Panning is now allowed - removed the blocking code that was preventing all map movement

        // Restore scroll input for the panel (if we saved it)
        if let (Some(smooth), Some(raw)) = (saved_smooth_scroll, saved_raw_scroll) {
            ui.ctx().input_mut(|i| {
                i.smooth_scroll_delta = smooth;
                i.raw_scroll_delta = raw;
            });
        }

        // Update state from MapMemory after gestures
        self.map_zoom_level = self.map_memory.zoom() as f32;
    }
}

impl eframe::App for AirjediApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let frame_start = std::time::Instant::now();

        // Define keyboard shortcuts as constants to avoid duplication
        const SETTINGS_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND,
            egui::Key::Comma,
        );
        const AIRCRAFT_LIST_SHORTCUT: egui::KeyboardShortcut = egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND,
            egui::Key::L,
        );

        // Handle keyboard shortcuts
        ctx.input_mut(|i| {
            // Cmd+, (Command+Comma) for Settings
            if i.consume_shortcut(&SETTINGS_SHORTCUT) {
                self.show_settings_window = true;
            }
            // Cmd+L (Command+L) for Aircraft List toggle
            if i.consume_shortcut(&AIRCRAFT_LIST_SHORTCUT) {
                self.aircraft_list_expanded = !self.aircraft_list_expanded;
            }
        });

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

                    // Set the center location in ConnectionManager for distance filtering
                    self.connection_manager.lock().unwrap().set_center(lat, lon);

                    self.system_status.lock().unwrap().add_diagnostic(
                        DiagnosticLevel::Info,
                        format!("Location set: {:.4}°, {:.4}°", lat, lon)
                    );

                    self.startup_state = StartupState::StartingTcpClient;
                }
                StartupState::StartingTcpClient => {
                    // Initialize all enabled servers from config
                    self.system_status.lock().unwrap().add_diagnostic(
                        DiagnosticLevel::Info,
                        "Initializing server connections...".to_string()
                    );

                    // Add all configured servers to the ConnectionManager
                    let mut connection_manager = self.connection_manager.lock().unwrap();
                    for server in &self.config.servers {
                        connection_manager.add_server(server.clone());
                    }

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
            let connection_manager = self.connection_manager.lock().unwrap();
            let aircraft_list = connection_manager.get_all_aircraft_merged();  // Cheap Arc clones from all servers
            let total = aircraft_list.len();
            let active = aircraft_list.iter().filter(|a| {
                (chrono::Utc::now() - a.last_seen()).num_seconds() < 60
            }).count();

            // Update per-server aircraft counts
            connection_manager.update_all_status_aircraft_counts();

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

        // Menu bar at the top
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    ui.add_enabled(false, egui::Button::new("New"));
                    ui.add_enabled(false, egui::Button::new("Open"));
                    ui.separator();
                    ui.add_enabled(false, egui::Button::new("Save"));
                    ui.add_enabled(false, egui::Button::new("Save As..."));
                    ui.separator();
                    if ui.add(egui::Button::new("Settings...")
                        .shortcut_text(ui.ctx().format_shortcut(&SETTINGS_SHORTCUT)))
                        .clicked()
                    {
                        self.show_settings_window = true;
                    }
                    ui.separator();
                    if ui.button("Exit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui.button("Map Overlays...").clicked() {
                        self.show_map_overlays_window = true;
                    }
                    if ui.button("Filters...").clicked() {
                        self.show_filters_window = true;
                    }
                    ui.separator();
                    // Aircraft List with checkmark and keyboard shortcut
                    let aircraft_list_text = if self.aircraft_list_expanded {
                        "✓ Aircraft List"
                    } else {
                        "  Aircraft List"
                    };
                    if ui.add(egui::Button::new(aircraft_list_text)
                        .shortcut_text(ui.ctx().format_shortcut(&AIRCRAFT_LIST_SHORTCUT)))
                        .clicked()
                    {
                        self.aircraft_list_expanded = !self.aircraft_list_expanded;
                    }
                    ui.separator();
                    ui.add_enabled(false, egui::Button::new("Zoom In"));
                    ui.add_enabled(false, egui::Button::new("Zoom Out"));
                    ui.separator();
                    ui.add_enabled(false, egui::Button::new("Reset View"));
                    ui.add_enabled(false, egui::Button::new("Fullscreen"));
                });
            });
        });

        // Map takes full width (or loading screen during startup)
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                // Show loading screen until startup is complete
                if self.startup_state != StartupState::Complete {
                    self.draw_loading_screen(ui, ctx);
                } else {
                    self.draw_map(ui);
                }
            });

        // Docked aircraft list panel on the right with smooth collapse animation
        // Calculate animated width for smooth expand/collapse
        let collapsed_width = 40.0;
        let target_width = if self.aircraft_list_expanded {
            self.aircraft_list_width
        } else {
            collapsed_width
        };

        let animated_width = ctx.animate_value_with_time(
            egui::Id::new("aircraft_list_width_anim"),
            target_width,
            0.2 // 200ms animation duration
        );

        // Configure panel differently based on expanded state
        // Use a proper frame to ensure the panel blocks input events from reaching the map
        let panel = egui::SidePanel::right("aircraft_list_panel")
            .frame(egui::Frame::NONE
                .fill(egui::Color32::from_rgba_unmultiplied(0, 0, 0, 1))); // Almost transparent but not TRANSPARENT

        // Apply width constraints based on state
        let panel = if self.aircraft_list_expanded {
            // Expanded: allow resizing within range
            panel.resizable(true)
                .min_width(200.0)
                .max_width(600.0)
                .default_width(self.aircraft_list_width)
        } else {
            // Collapsed: force exact width with animation
            panel.resizable(false)
                .exact_width(animated_width)
        };

        let panel_response = panel.show(ctx, |ui| {
                // Get the full panel area to create an input-blocking layer
                let panel_rect = ui.max_rect();

                // Use interact() instead of allocate_rect() - this blocks input without consuming layout space
                ui.interact(panel_rect, ui.id().with("panel_blocker"), egui::Sense::click_and_drag());

                // Draw gradient background for sheen effect
                let rect = ui.available_rect_before_wrap();

                if self.aircraft_list_expanded {
                    // Expanded state - show full gradient background
                    let painter = ui.painter();

                    // Layer 1: Solid background for clarity and separation from map
                    painter.rect_filled(
                        rect,
                        0.0,
                        egui::Color32::from_rgba_unmultiplied(25, 30, 35, 153)  // 60% opacity dark background
                    );

                    // Layer 2: Gradient overlay for sheen effect
                    let top_color = egui::Color32::from_rgba_unmultiplied(55, 64, 72, 179);
                    let bottom_color = egui::Color32::from_rgba_unmultiplied(15, 20, 25, 128);

                    // Draw gradient using mesh with vertices
                    let mut mesh = egui::epaint::Mesh::default();
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
                    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
                    painter.add(egui::Shape::mesh(mesh));

                    self.draw_aircraft_list(ui);
                } else {
                    // Collapsed state - show thin vertical tab
                    // Draw background and vertical text
                    {
                        let painter = ui.painter();
                        painter.rect_filled(
                            rect,
                            0.0,
                            egui::Color32::from_rgba_unmultiplied(35, 40, 45, 200)
                        );

                        // Vertical text "CONTACTS"
                        let center_x = rect.center().x;
                        let mut y_pos = rect.top() + 60.0;
                        let text = "CONTACTS";

                        for ch in text.chars() {
                            painter.text(
                                egui::pos2(center_x, y_pos),
                                egui::Align2::CENTER_TOP,
                                ch.to_string(),
                                egui::FontId::proportional(11.0),
                                egui::Color32::from_rgb(100, 200, 200),
                            );
                            y_pos += 14.0;
                        }
                    } // painter dropped here

                    // Expand button/icon at the top
                    ui.vertical_centered(|ui| {
                        ui.add_space(20.0);
                        let expand_button = egui::Button::new("◀")
                            .fill(egui::Color32::from_rgba_unmultiplied(45, 50, 55, 200))
                            .frame(false);

                        if ui.add(expand_button).clicked() {
                            self.aircraft_list_expanded = true;
                        }
                    });
                }
            });

        // Store panel rect for next frame's pointer detection
        self.aircraft_list_rect = Some(panel_response.response.rect);

        // Update panel width when user resizes (only when expanded)
        if self.aircraft_list_expanded {
            let current_width = panel_response.response.rect.width();
            if (current_width - self.aircraft_list_width).abs() > 1.0 {
                self.aircraft_list_width = current_width;
                // Save to config
                self.config.aircraft_list_width = current_width;
                let _ = self.config.save();
            }
        }

        // Attribution text (required by Carto/OSM license)
        // Position just to the left of the aircraft list panel
        let viewport = ctx.viewport_rect();

        // Calculate spacing (text width + padding, reduced by 23.5% to move closer to panel)
        let estimated_text_width = 260.0;
        let padding = 20.0;
        let total_spacing = (estimated_text_width + padding) * 0.765;  // 23.5% closer to the right

        egui::Area::new("map_attribution".into())
            .fixed_pos(egui::pos2(
                viewport.right() - animated_width - total_spacing,
                viewport.bottom() - 20.0  // 20px from bottom
            ))
            .order(egui::Order::Tooltip)  // Higher z-order to stay above panel
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("© OpenStreetMap contributors © CARTO")
                        .size(10.0)
                        .color(egui::Color32::from_rgba_unmultiplied(200, 200, 200, 180))  // Light gray, semi-transparent
                );
            });

        // Overlay controls window (only shown when opened from View menu)
        egui::Window::new("Map Overlays")
            .resizable(false)
            .collapsible(false)
            .open(&mut self.show_map_overlays_window)
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
                    // Track if any settings changed for auto-save
                    let mut settings_changed = false;

                    ui.horizontal(|ui| {
                        ui.label("Airports:");
                        if ui.checkbox(&mut self.show_airports, "").changed() {
                            settings_changed = true;
                        }
                    });

                    // Airport filter options (indented)
                    if self.show_airports {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new("Airport Filter:")
                            .size(10.0)
                            .color(egui::Color32::from_rgb(180, 180, 180)));

                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            if ui.radio_value(&mut self.airport_filter, AirportFilter::FrequentlyUsed, "Public/Frequent").changed() {
                                settings_changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            if ui.radio_value(&mut self.airport_filter, AirportFilter::All, "All Airports").changed() {
                                settings_changed = true;
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(10.0);
                            if ui.radio_value(&mut self.airport_filter, AirportFilter::MajorOnly, "Major Only").changed() {
                                settings_changed = true;
                            }
                        });
                        ui.add_space(4.0);
                    }

                    ui.horizontal(|ui| {
                        ui.label("Runways:");
                        if ui.checkbox(&mut self.show_runways, "").changed() {
                            settings_changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Navaids:");
                        if ui.checkbox(&mut self.show_navaids, "").changed() {
                            settings_changed = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Time-Limited Trails:");
                        if ui.checkbox(&mut self.time_limited_trails, "").changed() {
                            settings_changed = true;
                            // Sync checkbox state to all trackers
                            self.connection_manager.lock().unwrap().set_time_limited_trails(self.time_limited_trails);
                        }
                    });

                    // Auto-save settings if any changed
                    if settings_changed {
                        self.config.show_airports = self.show_airports;
                        self.config.show_runways = self.show_runways;
                        self.config.show_navaids = self.show_navaids;
                        self.config.time_limited_trails = self.time_limited_trails;
                        self.config.airport_filter = match self.airport_filter {
                            AirportFilter::All => "All".to_string(),
                            AirportFilter::FrequentlyUsed => "FrequentlyUsed".to_string(),
                            AirportFilter::MajorOnly => "MajorOnly".to_string(),
                        };

                        if let Err(e) = self.config.save() {
                            eprintln!("Failed to save config: {}", e);
                        }
                    }
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

        // Settings window (only shown when opened from File menu or Cmd+,)
        egui::Window::new("Settings")
            .resizable(false)
            .collapsible(false)
            .open(&mut self.show_settings_window)
            .show(ctx, |ui| {
                ui.heading(egui::RichText::new("Server Configuration")
                    .size(12.0)
                    .strong());

                ui.add_space(8.0);

                // Server list
                let mut servers_to_remove = Vec::new();
                let mut config_changed = false;

                // Get server statuses for display
                let server_statuses: std::collections::HashMap<String, status::ServerStatus> = {
                    let status = self.system_status.lock().unwrap();
                    status.servers.clone()
                };

                for server in &mut self.config.servers {
                    // Initialize edit state if not present
                    if !self.server_edit_state.contains_key(&server.id) {
                        self.server_edit_state.insert(
                            server.id.clone(),
                            (server.name.clone(), server.address.clone())
                        );
                    }

                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            // Connection status indicator
                            if let Some(server_status) = server_statuses.get(&server.id) {
                                let (icon, color) = match server_status.status {
                                    status::ConnectionStatus::Connected => ("●", egui::Color32::from_rgb(50, 255, 50)),
                                    status::ConnectionStatus::Connecting => ("○", egui::Color32::from_rgb(255, 200, 50)),
                                    status::ConnectionStatus::Disconnected => ("○", egui::Color32::from_rgb(150, 150, 150)),
                                    status::ConnectionStatus::Error => ("✗", egui::Color32::from_rgb(255, 100, 100)),
                                };
                                ui.label(egui::RichText::new(icon).color(color).size(16.0));
                            } else {
                                ui.label(egui::RichText::new("○").color(egui::Color32::from_rgb(150, 150, 150)).size(16.0));
                            }

                            ui.vertical(|ui| {
                                // Server name editor
                                ui.horizontal(|ui| {
                                    ui.label("Name:");
                                    let (name, _) = self.server_edit_state.get_mut(&server.id).unwrap();
                                    if ui.add(egui::TextEdit::singleline(name)
                                        .desired_width(120.0)).changed() {
                                        server.name = name.clone();
                                        config_changed = true;

                                        // Update SystemStatus immediately for live status pane update
                                        self.system_status.lock().unwrap().update_server_info(
                                            &server.id,
                                            server.name.clone(),
                                            server.address.clone()
                                        );
                                    }
                                });

                                // Server address editor
                                ui.horizontal(|ui| {
                                    ui.label("Address:");
                                    let (_, address) = self.server_edit_state.get_mut(&server.id).unwrap();
                                    if ui.add(egui::TextEdit::singleline(address)
                                        .hint_text("host:port")
                                        .desired_width(120.0)).changed() {
                                        server.address = address.clone();
                                        config_changed = true;

                                        // Update SystemStatus immediately for live status pane update
                                        self.system_status.lock().unwrap().update_server_info(
                                            &server.id,
                                            server.name.clone(),
                                            server.address.clone()
                                        );

                                        // Hot-reload address via ConnectionManager
                                        self.connection_manager.lock().unwrap()
                                            .update_server(&server.id, server.clone());
                                    }
                                });

                                // Show connection stats if available
                                if let Some(server_status) = server_statuses.get(&server.id) {
                                    ui.label(egui::RichText::new(
                                        format!("Messages: {} | Aircraft: {}",
                                            server_status.message_count,
                                            server_status.aircraft_count))
                                        .size(8.0)
                                        .color(egui::Color32::from_rgb(120, 120, 120)));

                                    if let Some(ref error) = server_status.last_error {
                                        ui.label(egui::RichText::new(format!("Error: {}", error))
                                            .size(8.0)
                                            .color(egui::Color32::from_rgb(255, 100, 100)));
                                    }
                                }
                            });

                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                // Remove button
                                if ui.button("🗑").on_hover_text("Remove server").clicked() {
                                    servers_to_remove.push(server.id.clone());
                                }

                                // Enabled checkbox
                                let mut enabled = server.enabled;
                                if ui.checkbox(&mut enabled, "Enabled").changed() {
                                    server.enabled = enabled;
                                    config_changed = true;

                                    // Enable/disable via ConnectionManager
                                    if enabled {
                                        self.connection_manager.lock().unwrap()
                                            .enable_server(&server.id);
                                    } else {
                                        self.connection_manager.lock().unwrap()
                                            .disable_server(&server.id);
                                    }
                                }
                            });
                        });
                    });

                    ui.add_space(4.0);
                }

                // Remove servers marked for deletion
                for server_id in &servers_to_remove {
                    self.config.remove_server(server_id);
                    self.server_edit_state.remove(server_id);
                    self.connection_manager.lock().unwrap().remove_server(server_id);
                    config_changed = true;
                }

                ui.add_space(8.0);

                // Add new server button
                if ui.button("➕ Add Server").clicked() {
                    let new_server = config::ServerConfig::new(
                        format!("Server {}", self.config.servers.len() + 1),
                        "localhost:30003".to_string(),
                        false  // Start disabled
                    );

                    self.server_edit_state.insert(
                        new_server.id.clone(),
                        (new_server.name.clone(), new_server.address.clone())
                    );

                    self.connection_manager.lock().unwrap().add_server(new_server.clone());
                    self.config.add_server(new_server);
                    config_changed = true;
                }

                // Auto-save configuration when changed
                if config_changed {
                    if let Err(e) = self.config.save() {
                        eprintln!("Failed to save config: {}", e);
                    }
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                // Show config file path
                if let Ok(config_path) = config::AppConfig::get_config_path() {
                    ui.label(egui::RichText::new("Config file:")
                        .size(9.0)
                        .color(egui::Color32::from_rgb(150, 150, 150)));
                    ui.label(egui::RichText::new(config_path.display().to_string())
                        .size(8.0)
                        .color(egui::Color32::from_rgb(120, 120, 120))
                        .monospace());
                }
            });

        // Filters window (only shown when opened from View menu)
        egui::Window::new("Filters")
            .resizable(false)
            .collapsible(false)
            .open(&mut self.show_filters_window)
            .show(ctx, |ui| {
                // Enable/Disable filters toggle
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.filters_enabled, "");
                    ui.label(egui::RichText::new("Enable Filters")
                        .color(egui::Color32::from_rgb(180, 180, 180))
                        .size(10.0));
                });

                ui.add_space(8.0);

                // Altitude filter
                ui.label(egui::RichText::new("Altitude (ft)")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(10.0)
                    .strong());
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

                ui.add_space(6.0);

                // Speed filter
                ui.label(egui::RichText::new("Speed (kts)")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(10.0)
                    .strong());
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

                ui.add_space(6.0);

                // Range filter
                ui.label(egui::RichText::new("Range (nm)")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(10.0)
                    .strong());
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

                ui.add_space(6.0);

                // ICAO filter
                ui.label(egui::RichText::new("ICAO")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(10.0)
                    .strong());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.filter_icao)
                        .hint_text("e.g., A1234")
                        .desired_width(200.0));
                    if !self.filter_icao.is_empty() {
                        if ui.small_button("✖").clicked() {
                            self.filter_icao.clear();
                        }
                    }
                });

                ui.add_space(6.0);

                // Registration filter
                ui.label(egui::RichText::new("Registration")
                    .color(egui::Color32::from_rgb(150, 200, 200))
                    .size(10.0)
                    .strong());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.filter_registration)
                        .hint_text("e.g., N12345")
                        .desired_width(200.0));
                    if !self.filter_registration.is_empty() {
                        if ui.small_button("✖").clicked() {
                            self.filter_registration.clear();
                        }
                    }
                });

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(6.0);

                // Reset filters button
                if ui.button("Reset All Filters").clicked() {
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
