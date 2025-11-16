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

//! Video stream manager for coordinating multiple video player windows.
//!
//! This module manages the lifecycle of video player windows, enforces
//! resource limits (max concurrent streams), and provides a centralized
//! API for opening and closing video streams.
//!
//! Key responsibilities:
//! - Track active video player windows
//! - Enforce maximum concurrent stream limit
//! - Generate unique window IDs
//! - Cleanup closed windows
//! - Coordinate with egui for rendering

use super::player::VideoPlayerWindow;
use super::protocol::VideoLink;
use std::collections::HashMap;
use uuid::Uuid;

/// Default maximum number of concurrent video streams
const DEFAULT_MAX_STREAMS: usize = 4;

/// Manages multiple video player windows
pub struct VideoManager {
    /// Active video player windows (window_id -> player)
    players: HashMap<String, VideoPlayerWindow>,

    /// Maximum number of concurrent streams allowed
    max_streams: usize,
}

impl Default for VideoManager {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoManager {
    /// Create a new video manager with default settings
    #[must_use]
    pub fn new() -> Self {
        Self {
            players: HashMap::new(),
            max_streams: DEFAULT_MAX_STREAMS,
        }
    }

    /// Create a new video manager with custom max streams
    #[must_use]
    pub fn with_max_streams(max_streams: usize) -> Self {
        Self {
            players: HashMap::new(),
            max_streams,
        }
    }

    /// Get the number of active streams
    #[must_use]
    pub fn active_stream_count(&self) -> usize {
        self.players.len()
    }

    /// Check if we can open another stream
    #[must_use]
    pub fn can_open_stream(&self) -> bool {
        self.players.len() < self.max_streams
    }

    /// Open a new video stream window
    ///
    /// # Errors
    /// Returns error if:
    /// - Max streams limit reached
    /// - Video player creation fails (e.g., invalid URL, GStreamer error)
    pub fn open_stream(&mut self, link: VideoLink) -> Result<String, String> {
        // Check if we've hit the limit
        if !self.can_open_stream() {
            return Err(format!(
                "Maximum concurrent streams ({}) reached. Close a video window to open another.",
                self.max_streams
            ));
        }

        // Generate unique window ID
        let window_id = Uuid::new_v4().to_string();

        // Create video player window
        let player = VideoPlayerWindow::new(window_id.clone(), link)?;

        // Store the player
        self.players.insert(window_id.clone(), player);

        Ok(window_id)
    }

    /// Close a specific video stream by window ID
    pub fn close_stream(&mut self, window_id: &str) {
        self.players.remove(window_id);
    }

    /// Render all active video windows
    pub fn render(&mut self, ctx: &egui::Context) {
        // Collect IDs of windows that should be closed
        let mut to_remove = Vec::new();

        // Render each player
        for (id, player) in &mut self.players {
            player.render(ctx);

            // Check if window was closed
            if !player.is_open() {
                to_remove.push(id.clone());
            }
        }

        // Remove closed windows
        for id in to_remove {
            self.players.remove(&id);
        }
    }

    /// Close all video streams
    pub fn close_all(&mut self) {
        self.players.clear();
    }

    /// Get maximum allowed streams
    #[must_use]
    pub const fn max_streams(&self) -> usize {
        self.max_streams
    }

    /// Set maximum allowed streams
    /// Note: This does not close existing streams if the new limit is lower
    pub fn set_max_streams(&mut self, max: usize) {
        self.max_streams = max;
    }

    /// Check if a specific window is still open
    #[must_use]
    pub fn is_window_open(&self, window_id: &str) -> bool {
        self.players.get(window_id).map_or(false, VideoPlayerWindow::is_open)
    }

    /// Get status summary for debugging/UI
    #[must_use]
    pub fn status_summary(&self) -> String {
        format!(
            "{}/{} streams active",
            self.players.len(),
            self.max_streams
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_streams_limit() {
        let mut manager = VideoManager::with_max_streams(2);
        assert_eq!(manager.max_streams(), 2);
        assert!(manager.can_open_stream());
        assert_eq!(manager.active_stream_count(), 0);
    }

    #[test]
    fn test_status_summary() {
        let manager = VideoManager::with_max_streams(4);
        assert_eq!(manager.status_summary(), "0/4 streams active");
    }
}
