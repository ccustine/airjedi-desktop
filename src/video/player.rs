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

//! Video streaming and playback using GStreamer.
//!
//! This module provides the core video playback functionality for AirJedi Desktop.
//! It handles video stream decoding via GStreamer pipelines and provides frame
//! extraction for rendering in egui windows.
//!
//! Architecture:
//! - Background thread runs GStreamer pipeline and decodes video
//! - Latest decoded frame stored in Arc<Mutex<Option<Frame>>>
//! - Main thread reads frame for texture upload to GPU
//! - Supports RTSP, HLS, HTTP, and YouTube streams

use super::protocol::{VideoLink, VideoProtocol};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app as gst_app;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Initialize GStreamer library (must be called once at application startup)
///
/// # Errors
/// Returns error if GStreamer initialization fails
pub fn init_gstreamer() -> Result<(), String> {
    gst::init().map_err(|e| format!("Failed to initialize GStreamer: {}", e))?;
    Ok(())
}

/// Current playback state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackState {
    /// Stream is stopped/not playing
    Stopped,
    /// Stream is buffering
    Buffering,
    /// Stream is playing
    Playing,
    /// Stream is paused
    Paused,
    /// An error occurred
    Error,
}

/// A decoded video frame ready for rendering
#[derive(Clone)]
pub struct VideoFrame {
    /// Raw RGBA pixel data
    pub data: Vec<u8>,
    /// Frame width in pixels
    pub width: u32,
    /// Frame height in pixels
    pub height: u32,
    /// Timestamp when this frame was captured
    pub timestamp: Instant,
}

impl VideoFrame {
    /// Convert this frame to an egui ColorImage for texture upload
    #[must_use]
    pub fn to_color_image(&self) -> egui::ColorImage {
        let pixels: Vec<egui::Color32> = self
            .data
            .chunks_exact(4)
            .map(|rgba| egui::Color32::from_rgba_premultiplied(rgba[0], rgba[1], rgba[2], rgba[3]))
            .collect();

        egui::ColorImage {
            size: [self.width as usize, self.height as usize],
            source_size: egui::vec2(self.width as f32, self.height as f32),
            pixels,
        }
    }
}

/// Video stream decoder using GStreamer
pub struct VideoStream {
    /// The video link being played
    link: VideoLink,

    /// GStreamer pipeline
    pipeline: gst::Pipeline,

    /// Latest decoded frame (shared with rendering thread)
    current_frame: Arc<Mutex<Option<VideoFrame>>>,

    /// Current playback state
    state: Arc<Mutex<PlaybackState>>,

    /// Error message if in error state
    error_message: Arc<Mutex<Option<String>>>,

    /// Background thread handle
    _thread_handle: Option<std::thread::JoinHandle<()>>,
}

impl VideoStream {
    /// Create a new video stream from a VideoLink
    ///
    /// # Errors
    /// Returns error if pipeline creation or initialization fails
    pub fn new(link: VideoLink) -> Result<Self, String> {
        let pipeline_desc = Self::build_pipeline_string(&link)?;

        let pipeline = gst::parse::launch(&pipeline_desc)
            .map_err(|e| format!("Failed to create GStreamer pipeline: {}", e))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| "Created element is not a pipeline".to_string())?;

        let current_frame = Arc::new(Mutex::new(None));
        let state = Arc::new(Mutex::new(PlaybackState::Stopped));
        let error_message = Arc::new(Mutex::new(None));

        // Set up app sink to extract frames
        let appsink = pipeline
            .by_name("sink")
            .ok_or("Failed to get appsink from pipeline")?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| "Sink element is not an AppSink".to_string())?;

        let frame_clone = current_frame.clone();

        // Configure appsink to emit signals and pull samples
        appsink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |appsink| {
                    let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Error)?;

                    if let Some(buffer) = sample.buffer() {
                        if let Some(caps) = sample.caps() {
                            // Extract video dimensions from caps
                            let s = caps.structure(0).ok_or(gst::FlowError::Error)?;
                            let width = s.get::<i32>("width").ok().ok_or(gst::FlowError::Error)? as u32;
                            let height = s.get::<i32>("height").ok().ok_or(gst::FlowError::Error)? as u32;

                            // Map buffer for reading
                            let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                            let data = map.as_slice().to_vec();

                            // Store frame
                            let frame = VideoFrame {
                                data,
                                width,
                                height,
                                timestamp: Instant::now(),
                            };

                            if let Ok(mut current) = frame_clone.lock() {
                                *current = Some(frame);
                            }
                        }
                    }

                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );

        Ok(Self {
            link,
            pipeline,
            current_frame,
            state,
            error_message,
            _thread_handle: None,
        })
    }

    /// Build the GStreamer pipeline string for a given protocol
    fn build_pipeline_string(link: &VideoLink) -> Result<String, String> {
        let pipeline = match link.protocol {
            VideoProtocol::RTSP => {
                format!(
                    "rtspsrc location={} latency=200 protocols=tcp ! decodebin name=dec \
                     dec. ! queue ! videoconvert ! video/x-raw,format=RGBA ! appsink name=sink max-buffers=1 drop=true \
                     dec. ! queue ! audioconvert ! audioresample ! autoaudiosink",
                    link.url
                )
            }
            VideoProtocol::HLS => {
                format!(
                    "souphttpsrc location={} ! hlsdemux ! tsdemux ! h264parse ! \
                     avdec_h264 ! videoconvert ! video/x-raw,format=RGBA ! \
                     appsink name=sink max-buffers=1 drop=true",
                    link.url
                )
            }
            VideoProtocol::HTTP => {
                format!(
                    "souphttpsrc location={} ! decodebin ! videoconvert ! \
                     video/x-raw,format=RGBA ! appsink name=sink max-buffers=1 drop=true",
                    link.url
                )
            }
            VideoProtocol::YouTube => {
                // YouTube requires URL resolution via youtube-dl
                // For now, return error - can be implemented later
                return Err("YouTube streams require youtube-dl integration (not yet implemented)".to_string());
            }
            VideoProtocol::RTMP => {
                format!(
                    "rtmpsrc location={} ! flvdemux ! h264parse ! avdec_h264 ! \
                     videoconvert ! video/x-raw,format=RGBA ! \
                     appsink name=sink max-buffers=1 drop=true",
                    link.url
                )
            }
        };

        Ok(pipeline)
    }

    /// Start playing the stream
    ///
    /// # Errors
    /// Returns error if playback cannot be started
    pub fn play(&mut self) -> Result<(), String> {
        println!("[VIDEO] Starting playback for: {}", self.link.url);

        self.pipeline
            .set_state(gst::State::Playing)
            .map_err(|e| format!("Failed to start playback: {}", e))?;

        *self.state.lock()
            .map_err(|_| "Failed to lock state mutex".to_string())? = PlaybackState::Playing;

        println!("[VIDEO] Playback state set to Playing");
        Ok(())
    }

    /// Pause the stream
    ///
    /// # Errors
    /// Returns error if pause fails
    pub fn pause(&mut self) -> Result<(), String> {
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|e| format!("Failed to pause: {}", e))?;

        *self.state.lock()
            .map_err(|_| "Failed to lock state mutex".to_string())? = PlaybackState::Paused;

        Ok(())
    }

    /// Stop the stream
    ///
    /// # Errors
    /// Returns error if stop fails
    pub fn stop(&mut self) -> Result<(), String> {
        // Use Paused state instead of Null so pipeline can be resumed
        self.pipeline
            .set_state(gst::State::Paused)
            .map_err(|e| format!("Failed to stop: {}", e))?;

        *self.state.lock()
            .map_err(|_| "Failed to lock state mutex".to_string())? = PlaybackState::Stopped;

        // Clear current frame
        if let Ok(mut frame) = self.current_frame.lock() {
            *frame = None;
        }

        Ok(())
    }

    /// Get the current playback state
    #[must_use]
    pub fn get_state(&self) -> PlaybackState {
        *self.state.lock().unwrap_or_else(|_| {
            panic!("State mutex poisoned")
        })
    }

    /// Get the latest decoded frame
    #[must_use]
    pub fn get_frame(&self) -> Option<VideoFrame> {
        self.current_frame.lock().ok()?.clone()
    }

    /// Get the video link being played
    #[must_use]
    pub fn link(&self) -> &VideoLink {
        &self.link
    }

    /// Get current error message if in error state
    #[must_use]
    pub fn get_error(&self) -> Option<String> {
        self.error_message.lock().ok()?.clone()
    }

    /// Check for pipeline errors and update state accordingly
    pub fn update_state(&mut self) {
        if let Some(bus) = self.pipeline.bus() {
            // Process all pending messages
            while let Some(msg) = bus.pop() {
                use gst::MessageView;

                match msg.view() {
                    MessageView::Error(err) => {
                        let error_msg = format!(
                            "GStreamer Error: {} ({})",
                            err.error(),
                            err.debug().unwrap_or_else(|| "No debug info".into())
                        );

                        // Log error to console for debugging
                        eprintln!("[VIDEO ERROR] {}", error_msg);

                        if let Ok(mut state) = self.state.lock() {
                            *state = PlaybackState::Error;
                        }
                        if let Ok(mut error) = self.error_message.lock() {
                            *error = Some(error_msg);
                        }
                    }
                    MessageView::Eos(_) => {
                        // End of stream
                        if let Ok(mut state) = self.state.lock() {
                            *state = PlaybackState::Stopped;
                        }
                    }
                    MessageView::Buffering(buffering) => {
                        let percent = buffering.percent();
                        if percent < 100 {
                            if let Ok(mut state) = self.state.lock() {
                                *state = PlaybackState::Buffering;
                            }
                        } else if let Ok(mut state) = self.state.lock() {
                            *state = PlaybackState::Playing;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

impl Drop for VideoStream {
    fn drop(&mut self) {
        // Stop pipeline when stream is dropped
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

/// Video player window for egui
pub struct VideoPlayerWindow {
    /// Unique window ID
    id: String,

    /// The video stream
    stream: VideoStream,

    /// Current video texture
    texture: Option<egui::TextureHandle>,

    /// Window is open
    is_open: bool,

    /// Volume (0.0 to 1.0)
    volume: f32,

    /// Last frame update time
    last_frame_update: Instant,
}

impl VideoPlayerWindow {
    /// Create a new video player window
    ///
    /// # Errors
    /// Returns error if stream creation fails
    pub fn new(id: String, link: VideoLink) -> Result<Self, String> {
        let mut stream = VideoStream::new(link)?;

        // Auto-start playback
        stream.play()?;

        Ok(Self {
            id,
            stream,
            texture: None,
            is_open: true,
            volume: 0.5,
            last_frame_update: Instant::now(),
        })
    }

    /// Get the window ID
    #[must_use]
    pub const fn id(&self) -> &String {
        &self.id
    }

    /// Check if window is open
    #[must_use]
    pub const fn is_open(&self) -> bool {
        self.is_open
    }

    /// Start playback
    ///
    /// # Errors
    /// Returns error if playback start fails
    pub fn play(&mut self) -> Result<(), String> {
        self.stream.play()
    }

    /// Pause playback
    ///
    /// # Errors
    /// Returns error if pause fails
    pub fn pause(&mut self) -> Result<(), String> {
        self.stream.pause()
    }

    /// Stop playback
    ///
    /// # Errors
    /// Returns error if stop fails
    pub fn stop(&mut self) -> Result<(), String> {
        self.stream.stop()
    }

    /// Render the video player window
    pub fn render(&mut self, ctx: &egui::Context) {
        // Update stream state
        self.stream.update_state();

        let window_title = self.stream.link().display_name();
        let mut is_open = self.is_open;

        // Position window at safe default location
        let safe_x = 50.0;
        let safe_y = 80.0; // Below menu bar

        egui::Window::new(&window_title)
            .id(egui::Id::new(&self.id))
            .default_pos(egui::pos2(safe_x, safe_y))
            .default_width(480.0)
            .default_height(320.0)
            .min_width(320.0)
            .min_height(240.0)
            .resizable(true)
            .collapsible(false)
            .open(&mut is_open)
            .show(ctx, |ui| {
                self.render_content(ui);
            });

        self.is_open = is_open;
    }

    /// Render window content
    fn render_content(&mut self, ui: &mut egui::Ui) {
        // Video display area
        let available_size = ui.available_size();

        // Update texture if we have a new frame
        if let Some(frame) = self.stream.get_frame() {
            // Only update texture if frame is recent (within last second)
            if frame.timestamp.elapsed() < Duration::from_secs(1) {
                let color_image = frame.to_color_image();

                if let Some(ref mut texture) = self.texture {
                    texture.set(color_image, egui::TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ui.ctx().load_texture(
                        &format!("video_frame_{}", self.id),
                        color_image,
                        egui::TextureOptions::LINEAR,
                    ));
                }

                self.last_frame_update = Instant::now();
            }
        }

        // Render video or placeholder
        let video_rect = egui::Rect::from_min_size(
            ui.cursor().min,
            egui::vec2(available_size.x, available_size.y - 50.0), // Reserve 50px for controls
        );

        if let Some(ref texture) = self.texture {
            ui.add(
                egui::Image::new(texture)
                    .fit_to_exact_size(video_rect.size())
                    .sense(egui::Sense::hover()),
            );
        } else {
            // Placeholder
            ui.allocate_rect(video_rect, egui::Sense::hover());
            ui.painter().rect_filled(
                video_rect,
                0.0,
                egui::Color32::from_rgb(30, 30, 30),
            );

            let state = self.stream.get_state();
            let status_text = match state {
                PlaybackState::Stopped => "Stopped",
                PlaybackState::Buffering => "Buffering...",
                PlaybackState::Playing => "Loading...",
                PlaybackState::Paused => "Paused",
                PlaybackState::Error => "Error",
            };

            ui.painter().text(
                video_rect.center(),
                egui::Align2::CENTER_CENTER,
                status_text,
                egui::FontId::proportional(16.0),
                egui::Color32::from_rgb(200, 200, 200),
            );
        }

        // Control bar
        ui.separator();
        ui.horizontal(|ui| {
            self.render_controls(ui);
        });
    }

    /// Render playback controls
    fn render_controls(&mut self, ui: &mut egui::Ui) {
        let state = self.stream.get_state();

        // Play/Pause button
        if state == PlaybackState::Playing {
            if ui.button("⏸ Pause").clicked() {
                let _ = self.pause();
            }
        } else if ui.button("▶ Play").clicked() {
            let _ = self.play();
        }

        // Stop button
        if ui.button("⏹ Stop").clicked() {
            let _ = self.stop();
        }

        ui.separator();

        // Volume control
        ui.label("Volume:");
        ui.add(egui::Slider::new(&mut self.volume, 0.0..=1.0).show_value(false));

        ui.separator();

        // Stream info
        if let Some(error) = self.stream.get_error() {
            // Use a horizontal scroll area for long error messages
            egui::ScrollArea::horizontal()
                .max_height(20.0)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(format!("⚠ {}", error))
                        .color(egui::Color32::from_rgb(255, 100, 100))
                        .size(9.0));
                });
        } else {
            let protocol_name = self.stream.link().protocol.name();
            ui.label(egui::RichText::new(format!("Protocol: {}", protocol_name))
                .color(egui::Color32::from_rgb(150, 150, 150))
                .size(9.0));

            // Frame info
            if let Some(frame) = self.stream.get_frame() {
                ui.label(egui::RichText::new(format!("{}×{}", frame.width, frame.height))
                    .color(egui::Color32::from_rgb(150, 150, 150))
                    .size(9.0));
            }
        }
    }
}
