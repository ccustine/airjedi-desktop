mod basestation;
mod tcp_client;
mod tiles;

use basestation::{Aircraft, AircraftTracker};
use eframe::egui;
use std::sync::{Arc, Mutex};
use serde::Deserialize;
use tiles::{TileManager, WebMercator};

// Trail display constants
const TRAIL_MAX_AGE_SECONDS: f32 = 900.0;  // 15 minutes total
const TRAIL_SOLID_DURATION_SECONDS: f32 = 450.0;  // First 7.5 minutes solid
const TRAIL_FADE_DURATION_SECONDS: f32 = 450.0;  // Last 7.5 minutes fade

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct GeoLocation {
    latitude: Option<f64>,
    longitude: Option<f64>,
}

#[cfg(target_os = "macos")]
fn get_gps_location() -> Option<(f64, f64)> {
    use cocoa::base::{id, nil};
    use objc::runtime::Class;
    use objc::{msg_send, sel, sel_impl};
    use std::time::Duration;

    println!("Attempting to get GPS location from CoreLocation...");

    unsafe {
        // Get CLLocationManager class
        let cls = Class::get("CLLocationManager")?;
        let manager: id = msg_send![cls, new];

        if manager == nil {
            println!("Failed to create CLLocationManager");
            return None;
        }

        // Check authorization status
        let auth_status: i32 = msg_send![cls, authorizationStatus];

        // Request authorization if needed (0 = not determined)
        if auth_status == 0 {
            println!("Requesting location authorization...");
            let _: () = msg_send![manager, requestWhenInUseAuthorization];
            // Give it a moment to process
            std::thread::sleep(Duration::from_millis(500));
        }

        // Start updating location
        let _: () = msg_send![manager, startUpdatingLocation];

        // Wait a bit for location update
        std::thread::sleep(Duration::from_secs(2));

        // Get location
        let location: id = msg_send![manager, location];

        if location != nil {
            // CLLocationCoordinate2D is a struct with latitude and longitude
            #[repr(C)]
            struct CLLocationCoordinate2D {
                latitude: f64,
                longitude: f64,
            }

            let coord: CLLocationCoordinate2D = msg_send![location, coordinate];

            let _: () = msg_send![manager, stopUpdatingLocation];
            let _: () = msg_send![manager, release];

            println!("GPS location found: {}, {}", coord.latitude, coord.longitude);
            return Some((coord.latitude, coord.longitude));
        } else {
            println!("No location available from GPS");
        }

        let _: () = msg_send![manager, stopUpdatingLocation];
        let _: () = msg_send![manager, release];
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
    println!("Starting AirJedi Desktop...");

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
        Box::new(|_cc| {
            println!("Creating application...");
            Ok(Box::new(AdsbApp::new()))
        }),
    )
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
}

impl AdsbApp {
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

    fn new() -> Self {
        println!("Initializing ADSB app...");
        let tracker = Arc::new(Mutex::new(AircraftTracker::new()));

        // Get current GPS location
        let (lat, lon) = get_current_location()
            .unwrap_or_else(|| {
                println!("Using default location (San Francisco)");
                (37.7749, -122.4194)
            });

        // Set the center location in the tracker for distance filtering
        tracker.lock().unwrap().set_center(lat, lon);

        // Spawn TCP connection in background
        println!("Starting TCP client thread...");
        let tracker_clone = tracker.clone();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(tcp_client::connect_adsb_feed(tracker_clone));
        });

        println!("App initialized successfully");
        Self {
            tracker,
            map_center_lat: lat,
            map_center_lon: lon,
            receiver_lat: lat,
            receiver_lon: lon,
            map_zoom_level: 8.0, // Zoom level 8 ≈ 150 mile range
            tile_manager: TileManager::new(),
            tile_error: None,
            selected_aircraft: None,
        }
    }

    fn draw_aircraft_list(&mut self, ui: &mut egui::Ui) {
        // Clone aircraft data with single lock to avoid holding lock during rendering
        let (count, aircraft_data): (usize, Vec<Aircraft>) = {
            let tracker = self.tracker.lock()
                .expect("Aircraft tracker mutex poisoned");
            let aircraft = tracker.get_aircraft();
            (aircraft.len(), aircraft.into_iter().cloned().collect())
        };

        // Military-style header
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("◈ CONTACT LIST")
                    .color(egui::Color32::from_rgb(100, 200, 100))
                    .size(14.0)
                    .strong());
            });

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("TOTAL: {}", count))
                    .color(egui::Color32::from_rgb(150, 150, 150))
                    .size(10.0)
                    .monospace());
            });
        });

        ui.add_space(4.0);

        let mut aircraft_list: Vec<&Aircraft> = aircraft_data.iter().collect();
        aircraft_list.sort_unstable_by(|a, b| {
            // Sort by altitude descending (highest threat first)
            b.altitude.unwrap_or(0).cmp(&a.altitude.unwrap_or(0))
        });

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.push_id("aircraft_list", |ui| {
                for aircraft in aircraft_list {
                    // Determine status color based on altitude and recency
                    let seconds_ago = (chrono::Utc::now() - aircraft.last_seen).num_seconds();
                    let (status_color, status_symbol) = if seconds_ago < 10 {
                        (egui::Color32::from_rgb(100, 255, 100), "●") // Active - green
                    } else if seconds_ago < 60 {
                        (egui::Color32::from_rgb(255, 200, 50), "●") // Recent - amber
                    } else {
                        (egui::Color32::from_rgb(150, 150, 150), "○") // Stale - grey
                    };

                    // Altitude-based threat level
                    let (alt_color, alt_indicator) = match aircraft.altitude {
                        Some(alt) if alt >= 30000 => (egui::Color32::from_rgb(200, 100, 255), "▲"), // High - purple
                        Some(alt) if alt >= 20000 => (egui::Color32::from_rgb(255, 150, 50), "▲"),  // Medium-high - orange
                        Some(alt) if alt >= 10000 => (egui::Color32::from_rgb(200, 200, 100), "▲"), // Medium - yellow
                        Some(_) => (egui::Color32::from_rgb(100, 200, 200), "▼"),                    // Low - cyan
                        None => (egui::Color32::from_rgb(100, 100, 100), "─"),                       // Unknown - grey
                    };

                    // Check if this aircraft is selected
                    let is_selected = self.selected_aircraft.as_ref() == Some(&aircraft.icao);

                    // Create a frame with background color if selected
                    let frame = if is_selected {
                        egui::Frame::group(ui.style())
                            .fill(egui::Color32::from_rgba_unmultiplied(100, 140, 180, 220))
                    } else {
                        egui::Frame::group(ui.style())
                    };

                    let response = frame.show(ui, |ui| {
                        // Status line with ICAO and callsign
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(status_symbol)
                                .color(status_color)
                                .size(12.0));

                            ui.label(egui::RichText::new(&aircraft.icao)
                                .color(egui::Color32::from_rgb(200, 220, 255))
                                .size(11.0)
                                .monospace()
                                .strong());

                            if let Some(ref callsign) = aircraft.callsign {
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
                                if let Some(alt) = aircraft.altitude {
                                    ui.label(egui::RichText::new(format!("{} FL{:03}", alt_indicator, alt / 100))
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
                            if let Some(vel) = aircraft.velocity {
                                ui.label(egui::RichText::new(format!("SPD {:03}", vel as i32))
                                    .color(egui::Color32::from_rgb(180, 180, 180))
                                    .size(9.0)
                                    .monospace());
                            }

                            // Track/Heading
                            if let Some(track) = aircraft.track {
                                ui.label(egui::RichText::new(format!("HDG {:03}°", track as i32))
                                    .color(egui::Color32::from_rgb(180, 180, 180))
                                    .size(9.0)
                                    .monospace());
                            }
                        });

                        // Position coordinates - dim
                        if let (Some(lat), Some(lon)) = (aircraft.latitude, aircraft.longitude) {
                            ui.label(egui::RichText::new(format!("{:>7.3}° {:>8.3}°", lat, lon))
                                .color(egui::Color32::from_rgb(120, 120, 120))
                                .size(8.5)
                                .monospace());
                        }

                        // Last seen timestamp
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(format!("T-{:03}s", seconds_ago))
                                .color(egui::Color32::from_rgb(100, 100, 100))
                                .size(8.0)
                                .monospace());
                        });
                    });

                    // Handle click to select this aircraft
                    if response.response.clicked() {
                        self.selected_aircraft = Some(aircraft.icao.clone());
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

        // Draw background
        painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(200, 220, 240));

        // Handle pinch-zoom gesture
        let zoom_delta = ui.ctx().input(|i| i.zoom_delta());
        if (zoom_delta - 1.0).abs() > 0.001 {
            // Apply zoom delta (zoom_delta > 1.0 means zoom in, < 1.0 means zoom out)
            let zoom_change = zoom_delta.log2();
            self.map_zoom_level += zoom_change;
            self.map_zoom_level = self.map_zoom_level.clamp(6.0, 12.0);
        }

        // Calculate tile size in pixels at current zoom level
        let tile_pixel_size = 256.0;

        // Round zoom level for tile fetching
        let tile_zoom_level = self.map_zoom_level.round() as u8;

        // Render map tiles
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
                let tile_pos = egui::pos2(
                    center.x + offset_x,
                    center.y + offset_y,
                );

                let tile_rect = egui::Rect::from_min_size(
                    tile_pos,
                    egui::vec2(tile_pixel_size, tile_pixel_size),
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

            let pixel_x = (tile_x - center_tile_x) * tile_pixel_size as f64;
            let pixel_y = (tile_y - center_tile_y) * tile_pixel_size as f64;

            egui::pos2(
                center.x + pixel_x as f32,
                center.y + pixel_y as f32,
            )
        };

        // Draw aircraft - clone data to release lock quickly
        let aircraft_list: Vec<Aircraft> = {
            let tracker = self.tracker.lock()
                .expect("Aircraft tracker mutex poisoned");
            tracker.get_aircraft().into_iter().cloned().collect()
        };

        for aircraft in &aircraft_list {
            // Draw trail first (so aircraft appears on top)
            if !aircraft.position_history.is_empty() {
                let now = chrono::Utc::now();

                // Draw trail segments
                for i in 0..aircraft.position_history.len() {
                    let point = &aircraft.position_history[i];
                    let age = (now - point.timestamp).num_milliseconds() as f32 / 1000.0;

                    // Only draw if age is within trail duration
                    if age > TRAIL_MAX_AGE_SECONDS {
                        continue;
                    }

                    // Calculate opacity based on age
                    let alpha = if age <= TRAIL_SOLID_DURATION_SECONDS {
                        255 // Solid for first half
                    } else {
                        let fade_age = age - TRAIL_SOLID_DURATION_SECONDS;
                        let opacity = (1.0 - (fade_age / TRAIL_FADE_DURATION_SECONDS)).clamp(0.0, 1.0);
                        (opacity * 255.0) as u8
                    };

                    let trail_pos = to_screen(point.lat, point.lon);

                    // Draw line to next point if there is one
                    if i + 1 < aircraft.position_history.len() {
                        let next_point = &aircraft.position_history[i + 1];
                        let next_age = (now - next_point.timestamp).num_milliseconds() as f32 / 1000.0;

                        // Only draw if next point is also within trail duration
                        if next_age <= TRAIL_MAX_AGE_SECONDS {
                            let next_pos = to_screen(next_point.lat, next_point.lon);

                            // Get altitude-based color
                            let (r, g, b) = Self::altitude_to_color(point.altitude);

                            // Apply time-based transparency to the altitude color
                            let trail_color = egui::Color32::from_rgba_unmultiplied(r, g, b, alpha);
                            painter.line_segment(
                                [trail_pos, next_pos],
                                egui::Stroke::new(2.0, trail_color)
                            );
                        }
                    }
                }

                // Draw line from last history point to current position if available
                if let (Some(lat), Some(lon)) = (aircraft.latitude, aircraft.longitude) {
                    if let Some(last_point) = aircraft.position_history.last() {
                        let last_pos = to_screen(last_point.lat, last_point.lon);
                        let current_pos = to_screen(lat, lon);

                        // Most recent segment is fully opaque with altitude-based color
                        let (r, g, b) = Self::altitude_to_color(aircraft.altitude);
                        let trail_color = egui::Color32::from_rgb(r, g, b);
                        painter.line_segment(
                            [last_pos, current_pos],
                            egui::Stroke::new(2.5, trail_color)
                        );
                    }
                }
            }

            if let (Some(lat), Some(lon)) = (aircraft.latitude, aircraft.longitude) {
                let pos = to_screen(lat, lon);

                // Only draw if within visible area
                if rect.contains(pos) {
                    // Draw aircraft as a circle
                    let color = egui::Color32::from_rgb(120, 220, 120); // Light green shade
                    painter.circle_filled(pos, 5.0, color);

                    // Draw heading indicator
                    if let Some(track) = aircraft.track {
                        let angle = track.to_radians();
                        let dx = angle.sin() as f32 * 15.0;
                        let dy = -angle.cos() as f32 * 15.0;
                        let end_pos = pos + egui::vec2(dx, dy);
                        painter.line_segment([pos, end_pos], egui::Stroke::new(2.0, color));
                    }

                    // Draw callsign and altitude labels to the right of the aircraft icon
                    let mut label_offset_y = -10.0; // Start slightly above the icon

                    // Draw callsign first (top)
                    if let Some(ref callsign) = aircraft.callsign {
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
                    if let Some(alt) = aircraft.altitude {
                        let alt_text = format!("{}ft", alt);
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

                    // Check if this aircraft icon was clicked
                    if response.clicked() {
                        if let Some(click_pos) = response.interact_pointer_pos() {
                            let distance = ((click_pos.x - pos.x).powi(2) + (click_pos.y - pos.y).powi(2)).sqrt();
                            if distance <= 10.0 { // Click radius slightly larger than icon
                                self.selected_aircraft = Some(aircraft.icao.clone());
                            }
                        }
                    }
                }
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
            "Drag to pan | Pinch to zoom",
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
    }
}

impl eframe::App for AdsbApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Request repaint periodically for real-time updates
        ctx.request_repaint_after(std::time::Duration::from_millis(500));

        // Map takes full width
        egui::CentralPanel::default()
            .frame(egui::Frame::none())
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
            .frame(egui::Frame::none()
                .fill(egui::Color32::TRANSPARENT)
                .rounding(egui::Rounding::same(8.0))
            )
            .show(ctx, |ui| {
                // Draw gradient background for sheen effect
                let rect = ui.available_rect_before_wrap();
                let painter = ui.painter();

                // Create gradient from top (lighter) to bottom (darker)
                let top_color = egui::Color32::from_rgba_unmultiplied(45, 50, 55, 61);    // Lighter top (24% opacity)
                let bottom_color = egui::Color32::from_rgba_unmultiplied(20, 25, 30, 41); // Darker bottom (16% opacity)

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
    }
}
