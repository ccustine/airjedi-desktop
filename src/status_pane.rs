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

use egui;
use crate::status::{SystemStatus, ConnectionStatus, DiagnosticLevel};
use std::time::Instant;

pub struct StatusPane {
    pub visible: bool,
    pub collapsed: bool,
    // Sparkline cache to avoid recalculating on every frame
    last_sparkline_update: Instant,
    cached_sparkline_points: Vec<egui::Pos2>,
    cached_sparkline_max: f32,
}

impl StatusPane {
    pub fn new() -> Self {
        Self {
            visible: true,
            collapsed: false,
            last_sparkline_update: Instant::now(),
            cached_sparkline_points: Vec::new(),
            cached_sparkline_max: 1.0,
        }
    }

    /// Render the status pane as a floating window
    pub fn render(&mut self, ctx: &egui::Context, status: &SystemStatus) {
        if !self.visible {
            // Show a small button to re-open the status pane when hidden
            egui::Window::new("show_status")
                .title_bar(false)
                .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(10.0, -10.0))
                .fixed_size(egui::vec2(140.0, 35.0))
                .resizable(false)
                .frame(egui::Frame::window(&ctx.style())
                    .fill(egui::Color32::from_rgba_unmultiplied(25, 30, 35, 200))
                    .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 80, 100)))
                    .corner_radius(6.0))
                .show(ctx, |ui| {
                    if ui.button(egui::RichText::new("📊 Show Status")
                        .color(egui::Color32::from_rgb(150, 200, 220))
                        .size(11.0))
                        .clicked() {
                        self.visible = true;
                    }
                });
            return;
        }

        let screen_height = ctx.screen_rect().height();

        egui::Window::new("System Status")
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(10.0, -10.0))
            .fixed_size(egui::vec2(304.0, if self.collapsed { 40.0 } else { screen_height.min(500.0) }))
            .resizable(false)
            .collapsible(false)
            .frame(egui::Frame::window(&ctx.style())
                .fill(egui::Color32::from_rgba_unmultiplied(25, 30, 35, 230))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_rgb(60, 80, 100)))
                .corner_radius(6.0))
            .show(ctx, |ui| {
                // Header with collapse and close buttons
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("◈ STATUS")
                        .color(egui::Color32::from_rgb(100, 180, 220))
                        .size(12.0)
                        .strong());

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Close/hide button
                        if ui.button(egui::RichText::new("✕")
                            .size(12.0)
                            .color(egui::Color32::from_rgb(200, 100, 100)))
                            .on_hover_text("Hide status pane")
                            .clicked() {
                            self.visible = false;
                        }

                        ui.add_space(4.0);

                        // Collapse/expand button
                        let collapse_icon = if self.collapsed { "▼" } else { "▲" };
                        if ui.button(egui::RichText::new(collapse_icon).size(10.0))
                            .on_hover_text(if self.collapsed { "Expand" } else { "Collapse" })
                            .clicked() {
                            self.collapsed = !self.collapsed;
                        }
                    });
                });

                if self.collapsed {
                    return;
                }

                ui.separator();

                egui::ScrollArea::vertical()
                    .max_height(screen_height.min(450.0))
                    .show(ui, |ui| {
                        // Connection Section
                        self.render_connection_section(ui, status);

                        ui.add_space(6.0);

                        // Metrics Section
                        self.render_metrics_section(ui, status);

                        ui.add_space(6.0);

                        // Data Status Section
                        self.render_data_status_section(ui, status);

                        ui.add_space(6.0);

                        // Performance Section
                        self.render_performance_section(ui, status);

                        ui.add_space(6.0);

                        // Diagnostics Section
                        self.render_diagnostics_section(ui, status);
                    });
            });
    }

    fn render_connection_section(&self, ui: &mut egui::Ui, status: &SystemStatus) {
        ui.label(egui::RichText::new("CONN")
            .color(egui::Color32::from_rgb(150, 150, 150))
            .size(9.0)
            .strong());

        ui.add_space(2.0);

        // Connection status with colored indicator
        ui.horizontal(|ui| {
            let (status_color, status_text, status_icon) = match status.connection_status {
                ConnectionStatus::Connected => (
                    egui::Color32::from_rgb(100, 255, 100),
                    "CONNECTED",
                    "●"
                ),
                ConnectionStatus::Connecting => (
                    egui::Color32::from_rgb(255, 200, 100),
                    "CONNECTING",
                    "◐"
                ),
                ConnectionStatus::Disconnected => (
                    egui::Color32::from_rgb(150, 150, 150),
                    "DISCONNECTED",
                    "○"
                ),
                ConnectionStatus::Error => (
                    egui::Color32::from_rgb(255, 100, 100),
                    "ERROR",
                    "✕"
                ),
            };

            ui.label(egui::RichText::new(status_icon)
                .color(status_color)
                .size(10.0));

            ui.label(egui::RichText::new(status_text)
                .color(status_color)
                .size(10.0)
                .monospace()
                .strong());
        });

        // Connection address (compact)
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&status.connection_address)
                .color(egui::Color32::from_rgb(180, 180, 180))
                .size(8.0)
                .monospace());
        });

        // Uptime (only if connected)
        if status.connection_status == ConnectionStatus::Connected && status.connection_uptime_seconds > 0 {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Uptime:")
                    .color(egui::Color32::from_rgb(130, 130, 130))
                    .size(9.0));
                let uptime_str = format_duration(status.connection_uptime_seconds);
                ui.label(egui::RichText::new(uptime_str)
                    .color(egui::Color32::from_rgb(200, 200, 200))
                    .size(9.0)
                    .monospace());
            });
        }
    }

    fn render_metrics_section(&mut self, ui: &mut egui::Ui, status: &SystemStatus) {
        ui.label(egui::RichText::new("METRICS")
            .color(egui::Color32::from_rgb(150, 150, 150))
            .size(10.0)
            .strong());

        ui.add_space(3.0);

        // Total messages
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Total:")
                .color(egui::Color32::from_rgb(130, 130, 130))
                .size(9.0));
            ui.label(egui::RichText::new(format!("{}", status.total_messages_received))
                .color(egui::Color32::from_rgb(200, 200, 200))
                .size(9.0)
                .monospace());
        });

        // Position updates with sparkline
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Pos:")
                .color(egui::Color32::from_rgb(130, 130, 130))
                .size(9.0));
            ui.label(egui::RichText::new(format!("{:.1}/s", status.position_updates_per_second))
                .color(egui::Color32::from_rgb(100, 200, 200))
                .size(9.0)
                .monospace());
        });

        // Sparkline visualization (outside of horizontal to allow mutable access)
        ui.horizontal(|ui| {
            self.render_sparkline(ui, status);
        });

        // Aircraft statistics
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Aircraft:")
                .color(egui::Color32::from_rgb(130, 130, 130))
                .size(9.0));
            ui.label(egui::RichText::new(format!("{} active / {} total",
                status.active_aircraft, status.total_aircraft_tracked))
                .color(egui::Color32::from_rgb(200, 200, 200))
                .size(9.0)
                .monospace());
        });
    }

    fn render_sparkline(&mut self, ui: &mut egui::Ui, status: &SystemStatus) {
        // Sparkline dimensions
        let width = 120.0;
        let height = 18.0;

        // Allocate space for the sparkline
        let (rect, _response) = ui.allocate_exact_size(
            egui::vec2(width, height),
            egui::Sense::hover()
        );

        let painter = ui.painter();

        // Get position update history
        let history = &status.position_updates_history;

        // Need at least 3 points: 2 stable points to draw a line, plus 1 current point we'll exclude
        if history.len() < 3 {
            // Not enough data to draw
            return;
        }

        // Check if 1 second has elapsed since last update
        let now = Instant::now();
        let should_update = now.duration_since(self.last_sparkline_update).as_secs_f32() >= 1.0;

        if should_update {
            // Recalculate sparkline points (only once per second)

            // Find max value for scaling (use all points for consistent scale)
            let max_count = history.iter()
                .map(|(_, count)| *count)
                .max()
                .unwrap_or(1) as f32;

            // Avoid division by zero
            self.cached_sparkline_max = max_count.max(1.0);

            // Calculate points, EXCLUDING the last point (current second still accumulating)
            // This prevents the graph from constantly repainting as the current second's count changes
            let stable_count = history.len() - 1;

            self.cached_sparkline_points = history
                .iter()
                .take(stable_count)  // Exclude the last point
                .enumerate()
                .map(|(i, (_, count))| {
                    let x = rect.min.x + (i as f32 / (stable_count - 1).max(1) as f32) * width;
                    let normalized = (*count as f32) / self.cached_sparkline_max;
                    let y = rect.max.y - (normalized * height);
                    egui::pos2(x, y)
                })
                .collect();

            self.last_sparkline_update = now;
        } else {
            // Use cached points but adjust for current rect position
            // (in case the window was moved or resized)
            if !self.cached_sparkline_points.is_empty() {
                self.cached_sparkline_points = self.cached_sparkline_points
                    .iter()
                    .enumerate()
                    .map(|(i, old_point)| {
                        // Recalculate with current rect, but use cached normalized values
                        let x = rect.min.x + (i as f32 / (self.cached_sparkline_points.len() - 1).max(1) as f32) * width;
                        let normalized = (rect.max.y - old_point.y) / height;
                        let y = rect.max.y - (normalized * height);
                        egui::pos2(x, y)
                    })
                    .collect();
            }
        }

        // Draw the line using cached points
        if self.cached_sparkline_points.len() >= 2 {
            painter.add(egui::Shape::line(
                self.cached_sparkline_points.clone(),
                egui::Stroke::new(1.5, egui::Color32::from_rgb(100, 220, 220)) // Cyan line
            ));
        }
    }

    fn render_data_status_section(&self, ui: &mut egui::Ui, status: &SystemStatus) {
        ui.label(egui::RichText::new("DATA STATUS")
            .color(egui::Color32::from_rgb(150, 150, 150))
            .size(10.0)
            .strong());

        ui.add_space(3.0);

        // Aviation data
        ui.horizontal(|ui| {
            let (icon, color) = if status.aviation_data_loaded {
                ("✓", egui::Color32::from_rgb(100, 255, 100))
            } else {
                ("⏳", egui::Color32::from_rgb(255, 200, 100))
            };

            ui.label(egui::RichText::new(icon)
                .color(color)
                .size(10.0));

            ui.label(egui::RichText::new("Aviation DB:")
                .color(egui::Color32::from_rgb(130, 130, 130))
                .size(9.0));

            if status.aviation_data_loaded {
                ui.label(egui::RichText::new(format!("{} apt, {} rwy, {} nav",
                    status.airports_loaded, status.runways_loaded, status.navaids_loaded))
                    .color(egui::Color32::from_rgb(200, 200, 200))
                    .size(8.0)
                    .monospace());
            } else {
                ui.label(egui::RichText::new("Loading...")
                    .color(egui::Color32::from_rgb(200, 200, 200))
                    .size(8.0));
            }
        });

        // Aircraft database
        ui.horizontal(|ui| {
            let (icon, color) = if status.aircraft_db_loaded {
                ("✓", egui::Color32::from_rgb(100, 255, 100))
            } else {
                ("⏳", egui::Color32::from_rgb(255, 200, 100))
            };

            ui.label(egui::RichText::new(icon)
                .color(color)
                .size(10.0));

            ui.label(egui::RichText::new("Aircraft DB:")
                .color(egui::Color32::from_rgb(130, 130, 130))
                .size(9.0));

            if status.aircraft_db_loaded {
                ui.label(egui::RichText::new(format!("{} aircraft", status.aircraft_db_size))
                    .color(egui::Color32::from_rgb(200, 200, 200))
                    .size(8.0)
                    .monospace());
            } else {
                ui.label(egui::RichText::new("Loading...")
                    .color(egui::Color32::from_rgb(200, 200, 200))
                    .size(8.0));
            }
        });
    }

    fn render_performance_section(&self, ui: &mut egui::Ui, status: &SystemStatus) {
        ui.label(egui::RichText::new("PERFORMANCE")
            .color(egui::Color32::from_rgb(150, 150, 150))
            .size(10.0)
            .strong());

        ui.add_space(3.0);

        // Frame time
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Frame:")
                .color(egui::Color32::from_rgb(130, 130, 130))
                .size(9.0));

            let frame_color = if status.average_update_duration_ms < 16.0 {
                egui::Color32::from_rgb(100, 255, 100) // Green for <60 FPS
            } else if status.average_update_duration_ms < 33.0 {
                egui::Color32::from_rgb(255, 200, 100) // Yellow for <30 FPS
            } else {
                egui::Color32::from_rgb(255, 100, 100) // Red for slow
            };

            ui.label(egui::RichText::new(format!("{:.1}ms ({:.0} FPS)",
                status.average_update_duration_ms,
                1000.0 / status.average_update_duration_ms.max(0.1)))
                .color(frame_color)
                .size(9.0)
                .monospace());
        });
    }

    fn render_diagnostics_section(&self, ui: &mut egui::Ui, status: &SystemStatus) {
        ui.label(egui::RichText::new("DIAGNOSTICS")
            .color(egui::Color32::from_rgb(150, 150, 150))
            .size(10.0)
            .strong());

        ui.add_space(3.0);

        let total_diagnostics = status.diagnostics.len();

        if total_diagnostics == 0 {
            ui.label(egui::RichText::new("No messages")
                .color(egui::Color32::from_rgb(100, 100, 100))
                .size(8.0)
                .italics());
        } else {
            // Show truncation message if more than 8 entries
            if total_diagnostics > 8 {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("⋮")
                        .color(egui::Color32::from_rgb(150, 150, 150))
                        .size(10.0));
                    ui.label(egui::RichText::new(format!("Log truncated ({} older)", total_diagnostics - 6))
                        .color(egui::Color32::from_rgb(120, 120, 120))
                        .size(7.5)
                        .italics());
                });
            }

            // Scrollable area for diagnostics (max 6 lines visible)
            // Each line is approximately 14 pixels tall (icon + text + spacing)
            let line_height = 14.0;
            let max_visible_lines = 6;

            egui::ScrollArea::vertical()
                .max_height(line_height * max_visible_lines as f32)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    // Show all diagnostics in reverse order (newest first)
                    for diagnostic in status.diagnostics.iter().rev() {
                        ui.horizontal(|ui| {
                            // Level indicator
                            let (icon, color) = match diagnostic.level {
                                DiagnosticLevel::Info => ("ℹ", egui::Color32::from_rgb(100, 180, 255)),
                                DiagnosticLevel::Warning => ("⚠", egui::Color32::from_rgb(255, 200, 100)),
                                DiagnosticLevel::Error => ("✕", egui::Color32::from_rgb(255, 100, 100)),
                            };

                            ui.label(egui::RichText::new(icon)
                                .color(color)
                                .size(9.0));

                            // Timestamp
                            let time_str = diagnostic.timestamp.format("%H:%M:%S").to_string();
                            ui.label(egui::RichText::new(time_str)
                                .color(egui::Color32::from_rgb(100, 100, 100))
                                .size(8.0)
                                .monospace());

                            // Message (truncate if too long)
                            let max_len = 26;
                            let msg = if diagnostic.message.len() > max_len {
                                format!("{}...", &diagnostic.message[..max_len])
                            } else {
                                diagnostic.message.clone()
                            };

                            ui.label(egui::RichText::new(msg)
                                .color(egui::Color32::from_rgb(180, 180, 180))
                                .size(8.0));
                        });
                    }
                });
        }
    }
}

fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

    if hours > 0 {
        format!("{}h {}m {}s", hours, minutes, secs)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}
