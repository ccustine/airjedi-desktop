//! Waterfall display window for FFT visualization.
//!
//! This window displays a scrolling waterfall (time-frequency) plot of
//! spectrum data from the IQ processor.

use crate::sdr::{list_devices, DeviceInfo, GainMode, IqProcessor, ProcessorConfig, SourceType};
use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use std::collections::VecDeque;
use std::path::PathBuf;

/// Waterfall visualization window.
///
/// Displays real-time spectrum data as a scrolling waterfall plot
/// with frequency on the horizontal axis and time on the vertical axis.
pub struct WaterfallWindow {
    /// Unique window ID
    id: String,
    /// Window open state
    open: bool,
    /// IQ processor (flowgraph)
    processor: Option<IqProcessor>,
    /// Spectrum history buffer (newest at back)
    waterfall_buffer: VecDeque<Vec<f32>>,
    /// Maximum number of spectrum lines to keep
    max_lines: usize,
    /// Waterfall texture
    waterfall_texture: Option<TextureHandle>,
    /// Configuration
    config: ProcessorConfig,
    /// UI state
    ui_state: UiState,
    /// Frame averaging buffer (accumulates values for averaging)
    averaging_buffer: Vec<f32>,
    /// Number of frames accumulated in averaging buffer
    frames_accumulated: usize,
}

/// UI state for controls
#[derive(Debug)]
struct UiState {
    // Source selection
    /// Selected source type (Demo, File, RTL-SDR)
    source_type: SourceType,

    // File mode
    /// File path input buffer
    file_path_input: String,

    // RTL-SDR mode
    /// Available RTL-SDR devices
    available_devices: Vec<DeviceInfo>,
    /// Selected device index
    selected_device_index: usize,
    /// Center frequency in MHz
    center_frequency_mhz: f64,
    /// Sample rate in MHz
    sample_rate_mhz: f64,
    /// Gain mode (Auto or Manual)
    gain_mode: GainMode,
    /// Manual gain value in dB
    manual_gain_db: f32,
    /// PPM frequency correction
    ppm_correction: i32,

    // Common parameters
    /// FFT size selection index
    fft_size_index: usize,
    /// Is processor running?
    is_running: bool,
    /// Min dB for display
    min_db: f32,
    /// Max dB for display
    max_db: f32,
    /// Auto-scale intensity
    auto_scale: bool,
    /// Number of frames to average (1-10, higher = slower/smoother)
    frames_to_average: usize,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            // Source selection
            source_type: SourceType::Demo,

            // File mode
            file_path_input: String::from("data/iq_samples.wav"),

            // RTL-SDR mode
            available_devices: Vec::new(),
            selected_device_index: 0,
            center_frequency_mhz: 1090.0, // 1090 MHz (ADS-B)
            sample_rate_mhz: 2.4,          // 2.4 MHz
            gain_mode: GainMode::Auto,
            manual_gain_db: 20.0,
            ppm_correction: 0,

            // Common parameters
            fft_size_index: 1, // 1024 by default
            is_running: false,
            min_db: -100.0,
            max_db: 0.0,
            auto_scale: true,
            frames_to_average: 4, // Default to 4 frames for smoother display
        }
    }
}

impl WaterfallWindow {
    /// Create a new waterfall window.
    ///
    /// # Arguments
    /// * `id` - Unique window identifier
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            open: true,
            processor: None,
            waterfall_buffer: VecDeque::with_capacity(500),
            max_lines: 500,
            waterfall_texture: None,
            config: ProcessorConfig::default(),
            ui_state: UiState::default(),
            averaging_buffer: Vec::new(),
            frames_accumulated: 0,
        }
    }

    /// Check if window is open.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Close the window.
    pub fn close(&mut self) {
        self.open = false;
        if let Some(mut processor) = self.processor.take() {
            processor.stop();
        }
    }

    /// Refresh the list of available RTL-SDR devices.
    fn refresh_devices(&mut self) {
        self.ui_state.available_devices = list_devices();
        if self.ui_state.available_devices.is_empty() {
            log::warn!("No RTL-SDR devices found");
        } else {
            log::info!("Found {} RTL-SDR device(s)", self.ui_state.available_devices.len());
        }
    }

    /// Start the IQ processor with current configuration.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn start_processor(&mut self) {
        log::info!("▶ User clicked START - Initializing waterfall processor...");

        // If there's an existing processor, signal it to stop
        // The background thread will clean up asynchronously
        if let Some(mut old_processor) = self.processor.take() {
            log::info!("⏸️ Stopping previous processor...");
            let stop_time = std::time::Instant::now();
            old_processor.stop();
            log::info!("⏱️ Stop signal sent after {:?}", stop_time.elapsed());

            // Add a tiny delay to let the stop signal propagate
            // This is barely noticeable (~50ms) but helps avoid device claim conflicts
            // when restarting RTL-SDR hardware
            log::info!("⏳ Waiting 50ms for background thread to see stop signal...");
            std::thread::sleep(std::time::Duration::from_millis(50));
            log::info!("⏱️ Total time since stop: {:?}", stop_time.elapsed());

            // Drop old_processor here - thread will exit in background
            log::info!("📦 Dropping old processor handle (thread will exit in background)...");
        }

        // Build config from UI state
        // Sync RTL-SDR parameters from UI state to source_type before using it
        // This ensures gain_mode and ppm_correction changes made after selecting RTL-SDR are applied
        if let SourceType::RtlSdr { device_index, .. } = &self.ui_state.source_type {
            self.ui_state.source_type = SourceType::RtlSdr {
                device_index: *device_index,
                gain_mode: self.ui_state.gain_mode,
                ppm_correction: self.ui_state.ppm_correction,
            };
        }

        self.config.source = self.ui_state.source_type.clone();
        self.config.fft_size = Self::fft_size_options()[self.ui_state.fft_size_index];
        self.config.sample_rate = self.ui_state.sample_rate_mhz * 1_000_000.0; // MHz to Hz
        self.config.center_frequency = self.ui_state.center_frequency_mhz * 1_000_000.0; // MHz to Hz

        // Create processor
        log::info!("🔧 Creating new IQ processor...");
        match IqProcessor::new(self.config.clone()) {
            Ok(processor) => {
                self.processor = Some(processor);
                self.ui_state.is_running = true;
                self.waterfall_buffer.clear();
                log::info!(
                    "✅ IQ processor started successfully (FFT size: {})",
                    self.config.fft_size
                );
            }
            Err(e) => {
                let error_msg = e.to_string();

                // Provide helpful message if device is still claimed from previous session
                if error_msg.contains("usb_claim_interface") {
                    log::error!("❌ RTL-SDR device still claimed (previous session closing)");
                    log::error!("   Please wait a moment and click Start again");
                } else {
                    log::error!("❌ Failed to start IQ processor: {}", e);
                }

                self.ui_state.is_running = false;
            }
        }
    }

    /// Stop the IQ processor (non-blocking).
    ///
    /// This signals the background thread to stop but doesn't wait for it.
    /// The processor handle is kept so we can properly clean it up when starting a new one.
    fn stop_processor(&mut self) {
        if let Some(processor) = &mut self.processor {
            processor.stop();
        }
        self.ui_state.is_running = false;
        log::info!("Stopped IQ processor (non-blocking)");
    }

    /// FFT size options for dropdown.
    const fn fft_size_options() -> [usize; 6] {
        [512, 1024, 2048, 4096, 8192, 16384]
    }

    /// Update waterfall buffer with new spectrum data.
    fn update_buffer(&mut self) {
        if let Some(processor) = &mut self.processor {
            // Only pull data if we're in running state
            if self.ui_state.is_running {
                // Pull all available spectrums and apply averaging
                while let Some(spectrum) = processor.try_recv_spectrum() {
                    // Initialize averaging buffer on first frame or size change
                    if self.averaging_buffer.is_empty() || self.averaging_buffer.len() != spectrum.len() {
                        self.averaging_buffer = vec![0.0; spectrum.len()];
                        self.frames_accumulated = 0;
                    }

                    // Accumulate this frame
                    for (i, &value) in spectrum.iter().enumerate() {
                        self.averaging_buffer[i] += value;
                    }
                    self.frames_accumulated += 1;

                    // If we've accumulated enough frames, compute average and add to display
                    if self.frames_accumulated >= self.ui_state.frames_to_average {
                        let averaged_spectrum: Vec<f32> = self.averaging_buffer
                            .iter()
                            .map(|&sum| sum / self.frames_accumulated as f32)
                            .collect();

                        self.waterfall_buffer.push_back(averaged_spectrum);

                        // Maintain max buffer size
                        while self.waterfall_buffer.len() > self.max_lines {
                            self.waterfall_buffer.pop_front();
                        }

                        // Reset averaging buffer for next batch
                        self.averaging_buffer.fill(0.0);
                        self.frames_accumulated = 0;
                    }
                }
            }

            // Check if processor is still running
            if !processor.is_running() && self.ui_state.is_running {
                self.ui_state.is_running = false;
                log::info!("IQ processor finished");
            }
        }
    }

    /// Render waterfall as texture.
    fn render_waterfall_texture(&mut self, ctx: &egui::Context) {
        if self.waterfall_buffer.is_empty() {
            return;
        }

        let width = self.waterfall_buffer[0].len();

        // Limit display to most recent lines that fit in window
        // With fixed window size of 700px height and ~150px for controls,
        // we have ~550px for waterfall. Limit to 500 lines for clean display.
        const MAX_VISIBLE_LINES: usize = 500;
        let total_lines = self.waterfall_buffer.len();
        let visible_lines = total_lines.min(MAX_VISIBLE_LINES);
        let skip_lines = total_lines.saturating_sub(MAX_VISIBLE_LINES);

        let height = visible_lines;

        // Auto-scale if enabled
        let (min_db, max_db) = if self.ui_state.auto_scale {
            let mut min = f32::INFINITY;
            let mut max = f32::NEG_INFINITY;
            for spectrum in &self.waterfall_buffer {
                for &value in spectrum {
                    min = min.min(value);
                    max = max.max(value);
                }
            }
            (min, max)
        } else {
            (self.ui_state.min_db, self.ui_state.max_db)
        };

        // Convert waterfall buffer to color image
        let mut pixels = Vec::with_capacity(width * height);

        // Render only the most recent visible lines (skip old ones)
        // This creates the "scrolling off top" effect
        for spectrum in self.waterfall_buffer.iter().skip(skip_lines) {
            for &db_value in spectrum {
                let color = db_to_color(db_value, min_db, max_db);
                pixels.push(color);
            }
        }

        let image = ColorImage {
            size: [width, height],
            source_size: egui::vec2(width as f32, height as f32),
            pixels,
        };

        // Update or create texture
        if let Some(tex) = &mut self.waterfall_texture {
            tex.set(image, TextureOptions::NEAREST);
        } else {
            self.waterfall_texture = Some(ctx.load_texture(
                format!("waterfall_{}", self.id),
                image,
                TextureOptions::NEAREST,
            ));
        }
    }

    /// Render the window.
    pub fn render(&mut self, ctx: &egui::Context) {
        if !self.open {
            return;
        }

        // Update buffer with new data
        self.update_buffer();

        // Render waterfall texture
        self.render_waterfall_texture(ctx);

        // Show window
        let mut open = self.open;
        egui::Window::new(format!("Waterfall - {}", self.id))
            .id(egui::Id::new(&self.id))
            .open(&mut open)
            .fixed_size([900.0, 700.0])
            .show(ctx, |ui| {
                self.render_content(ui);
            });

        self.open = open;

        // Cleanup if closed
        if !self.open {
            self.close();
        }
    }

    /// Render window content.
    #[allow(clippy::too_many_lines)]
    fn render_content(&mut self, ui: &mut egui::Ui) {
        // Source selection dropdown
        ui.horizontal(|ui| {
            ui.label("Source:");
            egui::ComboBox::from_id_salt("source_type")
                .selected_text(match &self.ui_state.source_type {
                    SourceType::Demo => "Demo Mode",
                    SourceType::File { .. } => "IQ File",
                    SourceType::RtlSdr { .. } => "RTL-SDR Hardware",
                })
                .show_ui(ui, |ui| {
                    if ui.selectable_label(matches!(self.ui_state.source_type, SourceType::Demo), "Demo Mode").clicked() {
                        self.ui_state.source_type = SourceType::Demo;
                    }
                    if ui.selectable_label(matches!(self.ui_state.source_type, SourceType::File { .. }), "IQ File").clicked() {
                        self.ui_state.source_type = SourceType::File {
                            path: PathBuf::from(&self.ui_state.file_path_input),
                        };
                    }
                    if ui.selectable_label(matches!(self.ui_state.source_type, SourceType::RtlSdr { .. }), "RTL-SDR Hardware").clicked() {
                        self.refresh_devices();
                        self.ui_state.source_type = SourceType::RtlSdr {
                            device_index: self.ui_state.selected_device_index as u32,
                            gain_mode: self.ui_state.gain_mode,
                            ppm_correction: self.ui_state.ppm_correction,
                        };
                    }
                });
        });

        ui.separator();

        // Source-specific controls
        match &self.ui_state.source_type {
            SourceType::Demo => {
                ui.label("🎨 Demo Mode - Generates synthetic spectrum data");
                ui.label("This mode creates 3 moving signals for testing the waterfall display.");
            }
            SourceType::File { .. } => {
                ui.horizontal(|ui| {
                    ui.label("IQ File:");
                    let response = ui.text_edit_singleline(&mut self.ui_state.file_path_input);

                    // Update source_type when user types in the text box
                    if response.changed() {
                        self.ui_state.source_type = SourceType::File {
                            path: PathBuf::from(&self.ui_state.file_path_input),
                        };
                    }

                    if ui.button("Browse...").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("All IQ Files", &["cf32", "iq", "cfile", "wav"])
                            .add_filter("WAV Files (16-bit stereo)", &["wav"])
                            .add_filter("Complex Float32 IQ", &["cf32", "iq", "cfile"])
                            .add_filter("All Files", &["*"])
                            .pick_file()
                        {
                            self.ui_state.file_path_input = path.display().to_string();
                            self.ui_state.source_type = SourceType::File { path };
                        }
                    }
                });
            }
            SourceType::RtlSdr { .. } => {
                // Device selection
                ui.horizontal(|ui| {
                    ui.label("Device:");
                    if self.ui_state.available_devices.is_empty() {
                        ui.label("No devices found");
                    } else {
                        egui::ComboBox::from_id_salt("rtlsdr_device")
                            .selected_text(if self.ui_state.selected_device_index < self.ui_state.available_devices.len() {
                                &self.ui_state.available_devices[self.ui_state.selected_device_index].name
                            } else {
                                "Select device"
                            })
                            .show_ui(ui, |ui| {
                                for (i, dev) in self.ui_state.available_devices.iter().enumerate() {
                                    if ui.selectable_value(&mut self.ui_state.selected_device_index, i, &dev.name).clicked() {
                                        self.ui_state.source_type = SourceType::RtlSdr {
                                            device_index: dev.index,
                                            gain_mode: self.ui_state.gain_mode,
                                            ppm_correction: self.ui_state.ppm_correction,
                                        };
                                    }
                                }
                            });
                    }
                    if ui.button("🔄 Refresh").clicked() {
                        self.refresh_devices();
                    }
                });

                // Frequency input with presets
                ui.horizontal(|ui| {
                    ui.label("Frequency:");
                    ui.add(egui::DragValue::new(&mut self.ui_state.center_frequency_mhz)
                        .speed(0.1)
                        .suffix(" MHz")
                        .range(24.0..=1766.0));

                    if ui.button("1090 MHz").clicked() {
                        self.ui_state.center_frequency_mhz = 1090.0;
                    }
                    if ui.button("978 MHz").clicked() {
                        self.ui_state.center_frequency_mhz = 978.0;
                    }
                    if ui.button("FM").clicked() {
                        self.ui_state.center_frequency_mhz = 100.0;
                    }
                });

                // Sample rate
                ui.horizontal(|ui| {
                    ui.label("Sample Rate:");
                    egui::ComboBox::from_id_salt("sample_rate")
                        .selected_text(format!("{} MHz", self.ui_state.sample_rate_mhz))
                        .show_ui(ui, |ui| {
                            for &rate in &[0.25, 0.5, 1.0, 1.024, 1.92, 2.048, 2.4, 2.56, 3.2] {
                                ui.selectable_value(&mut self.ui_state.sample_rate_mhz, rate, format!("{rate} MHz"));
                            }
                        });
                });

                // Gain controls
                ui.horizontal(|ui| {
                    ui.label("Gain:");
                    let mut gain_is_auto = matches!(self.ui_state.gain_mode, GainMode::Auto);
                    ui.radio_value(&mut gain_is_auto, true, "Auto");
                    ui.radio_value(&mut gain_is_auto, false, "Manual");

                    if gain_is_auto {
                        self.ui_state.gain_mode = GainMode::Auto;
                    } else {
                        ui.add(egui::Slider::new(&mut self.ui_state.manual_gain_db, 0.0..=50.0).suffix(" dB"));
                        self.ui_state.gain_mode = GainMode::Manual((self.ui_state.manual_gain_db * 10.0) as i32);
                    }
                });

                // PPM correction
                ui.horizontal(|ui| {
                    ui.label("PPM Correction:");
                    ui.add(egui::DragValue::new(&mut self.ui_state.ppm_correction)
                        .speed(1.0)
                        .range(-100..=100)
                        .suffix(" ppm"));
                });
            }
        }

        ui.separator();

        ui.horizontal(|ui| {
            ui.label("FFT Size:");
            egui::ComboBox::from_id_salt("fft_size")
                .selected_text(format!("{}", Self::fft_size_options()[self.ui_state.fft_size_index]))
                .show_ui(ui, |ui| {
                    for (i, &size) in Self::fft_size_options().iter().enumerate() {
                        ui.selectable_value(&mut self.ui_state.fft_size_index, i, format!("{size}"));
                    }
                });

            ui.separator();

            ui.label("Averaging:");
            ui.add(egui::Slider::new(&mut self.ui_state.frames_to_average, 1..=10)
                .suffix(" frames")
                .text(""));

            ui.separator();

            if self.ui_state.is_running {
                if ui.button("⏸ Stop").clicked() {
                    self.stop_processor();
                }
                ui.label("🟢 Running");
            } else if ui.button("▶ Start").clicked() {
                self.start_processor();
            }
        });

        ui.horizontal(|ui| {
            ui.checkbox(&mut self.ui_state.auto_scale, "Auto-scale");

            if !self.ui_state.auto_scale {
                ui.label("Min dB:");
                ui.add(egui::DragValue::new(&mut self.ui_state.min_db).speed(1.0));
                ui.label("Max dB:");
                ui.add(egui::DragValue::new(&mut self.ui_state.max_db).speed(1.0));
            }
        });

        ui.separator();

        // Info panel
        if let Some(processor) = &self.processor {
            let config = processor.config();
            ui.horizontal(|ui| {
                ui.label(format!("Center Frequency: {:.3} MHz", config.center_frequency / 1e6));
                ui.separator();
                ui.label(format!("Sample Rate: {:.3} MHz", config.sample_rate / 1e6));
                ui.separator();
                ui.label(format!("Lines: {}", self.waterfall_buffer.len()));
            });
        }

        ui.separator();

        // Waterfall display - allocate fixed space to prevent window growth
        let available_size = ui.available_size();
        let display_width = available_size.x;
        let display_height = available_size.y.max(500.0); // Reserve at least 500px for waterfall

        if let Some(texture) = &self.waterfall_texture {
            // Render texture at full width, natural height
            let texture_height = texture.size()[1] as f32;

            // Allocate the full available space to prevent window resizing
            let (rect, _response) = ui.allocate_exact_size(
                egui::vec2(display_width, display_height),
                egui::Sense::hover()
            );

            // Render image bottom-aligned within the allocated space
            // This makes new data appear at bottom, old data at top
            let image_rect = if texture_height < display_height {
                // Texture smaller than space - align to bottom
                egui::Rect::from_min_size(
                    egui::pos2(rect.min.x, rect.max.y - texture_height),
                    egui::vec2(display_width, texture_height),
                )
            } else {
                // Texture fills or exceeds space
                rect
            };

            ui.painter().image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            // No texture yet - show demo message in allocated space
            let (rect, _response) = ui.allocate_exact_size(
                egui::vec2(display_width, display_height),
                egui::Sense::hover()
            );

            let mut child_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rect)
                    .layout(egui::Layout::centered_and_justified(egui::Direction::TopDown))
            );

            child_ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new("🎨 DEMO MODE READY")
                        .size(18.0)
                        .color(egui::Color32::from_rgb(100, 200, 255))
                );
                ui.add_space(10.0);
                ui.label("Click the ▶ Start button above to generate");
                ui.label("synthetic waterfall data and see the visualization.");
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new("This demo shows 3 moving signals on a noise floor")
                        .italics()
                        .color(egui::Color32::GRAY)
                );
            });
        }
    }
}

/// Convert dB value to color using a blue→green→yellow→red gradient.
///
/// This creates a "hot" colormap similar to those used in spectrum analyzers.
///
/// # Arguments
/// * `db` - dB value
/// * `min_db` - Minimum dB (maps to blue/black)
/// * `max_db` - Maximum dB (maps to red)
///
/// # Returns
/// Color32 for the given dB value
fn db_to_color(db: f32, min_db: f32, max_db: f32) -> Color32 {
    // Normalize to 0.0-1.0 range
    let normalized = ((db - min_db) / (max_db - min_db)).clamp(0.0, 1.0);

    // Blue → Cyan → Green → Yellow → Red gradient (5 stops)
    // Similar to the altitude gradient in main.rs:944
    let stops = [
        (0.0, (0, 0, 128)),     // Dark blue (noise floor)
        (0.25, (0, 128, 255)),  // Cyan
        (0.5, (0, 255, 0)),     // Green
        (0.75, (255, 255, 0)),  // Yellow
        (1.0, (255, 0, 0)),     // Red (strong signal)
    ];

    // Find the two stops to interpolate between
    for i in 0..stops.len() - 1 {
        let (t1, (r1, g1, b1)) = stops[i];
        let (t2, (r2, g2, b2)) = stops[i + 1];

        if normalized >= t1 && normalized <= t2 {
            // Linear interpolation
            let t = (normalized - t1) / (t2 - t1);
            let r = (r1 as f32 + t * (r2 - r1) as f32) as u8;
            let g = (g1 as f32 + t * (g2 - g1) as f32) as u8;
            let b = (b1 as f32 + t * (b2 - b1) as f32) as u8;
            return Color32::from_rgb(r, g, b);
        }
    }

    // Fallback (shouldn't reach here)
    Color32::from_rgb(255, 0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_db_to_color() {
        // Test gradient endpoints
        let color_min = db_to_color(-100.0, -100.0, 0.0);
        assert_eq!(color_min, Color32::from_rgb(0, 0, 128));

        let color_max = db_to_color(0.0, -100.0, 0.0);
        assert_eq!(color_max, Color32::from_rgb(255, 0, 0));

        // Test middle (should be greenish)
        let color_mid = db_to_color(-50.0, -100.0, 0.0);
        let Color32 { r: _r, g, b: _b, a: _ } = color_mid;
        assert!(g > 128); // Should have significant green component
    }
}
