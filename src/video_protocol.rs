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

//! Video streaming protocol types and configuration.
//!
//! This module defines the video streaming protocols supported by AirJedi Desktop
//! and provides the `VideoLink` abstraction for associating video streams with
//! aircraft, airports, and fixed camera locations.
//!
//! Supported protocols:
//! - RTSP (Real Time Streaming Protocol)
//! - HLS (HTTP Live Streaming)
//! - HTTP (Direct HTTP video streams)
//! - YouTube (YouTube live streams and videos)
//! - RTMP (Real Time Messaging Protocol)

use serde::{Deserialize, Serialize};

/// Video streaming protocol identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoProtocol {
    /// Real Time Streaming Protocol (rtsp://)
    /// Common for IP cameras and security systems
    RTSP,

    /// HTTP Live Streaming (https://.../*.m3u8)
    /// Apple's adaptive bitrate streaming protocol
    HLS,

    /// Direct HTTP stream (http:// or https://)
    /// For MJPEG, MP4, or other direct HTTP video streams
    HTTP,

    /// YouTube video or live stream
    /// Requires URL resolution via youtube-dl or similar
    YouTube,

    /// Real Time Messaging Protocol (rtmp://)
    /// Common for live broadcast streaming
    RTMP,
}

impl VideoProtocol {
    /// Automatically detect protocol from URL
    /// Falls back to HTTP if unable to determine
    #[must_use]
    pub fn from_url(url: &str) -> Self {
        let lower = url.to_lowercase();

        if lower.starts_with("rtsp://") {
            Self::RTSP
        } else if lower.starts_with("rtmp://") {
            Self::RTMP
        } else if lower.contains("youtube.com") || lower.contains("youtu.be") {
            Self::YouTube
        } else if lower.ends_with(".m3u8") || lower.contains("/hls/") {
            Self::HLS
        } else {
            Self::HTTP
        }
    }

    /// Get human-readable name for this protocol
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::RTSP => "RTSP",
            Self::HLS => "HLS",
            Self::HTTP => "HTTP",
            Self::YouTube => "YouTube",
            Self::RTMP => "RTMP",
        }
    }

    /// Get typical latency characteristics for this protocol
    /// Returns (min_ms, max_ms)
    #[must_use]
    pub const fn typical_latency_ms(&self) -> (u32, u32) {
        match self {
            Self::RTSP => (500, 3000),      // 0.5-3 seconds typical
            Self::HLS => (6000, 30000),     // 6-30 seconds (due to segmentation)
            Self::HTTP => (100, 1000),      // 0.1-1 second
            Self::YouTube => (5000, 15000), // 5-15 seconds
            Self::RTMP => (1000, 5000),     // 1-5 seconds
        }
    }
}

/// A video stream link that can be associated with entities
/// (aircraft, airports, fixed locations)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoLink {
    /// The URL of the video stream
    pub url: String,

    /// The streaming protocol used
    pub protocol: VideoProtocol,

    /// Human-readable title/name for this stream
    pub title: Option<String>,

    /// Optional description of what this stream shows
    pub description: Option<String>,
}

impl VideoLink {
    /// Create a new video link with automatic protocol detection
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        let url = url.into();
        let protocol = VideoProtocol::from_url(&url);

        Self {
            url,
            protocol,
            title: None,
            description: None,
        }
    }

    /// Create a new video link with explicit protocol
    #[must_use]
    pub fn with_protocol(url: impl Into<String>, protocol: VideoProtocol) -> Self {
        Self {
            url: url.into(),
            protocol,
            title: None,
            description: None,
        }
    }

    /// Builder method to add a title
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Builder method to add a description
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Get the display name for this video link
    /// Returns title if available, otherwise a generated name from URL
    #[must_use]
    pub fn display_name(&self) -> String {
        if let Some(ref title) = self.title {
            title.clone()
        } else {
            // Extract a reasonable name from URL
            self.url
                .split('/')
                .last()
                .unwrap_or("Video Stream")
                .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_detection() {
        assert_eq!(
            VideoProtocol::from_url("rtsp://192.168.1.100:554/stream"),
            VideoProtocol::RTSP
        );
        assert_eq!(
            VideoProtocol::from_url("https://example.com/stream.m3u8"),
            VideoProtocol::HLS
        );
        assert_eq!(
            VideoProtocol::from_url("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            VideoProtocol::YouTube
        );
        assert_eq!(
            VideoProtocol::from_url("rtmp://live.example.com/stream"),
            VideoProtocol::RTMP
        );
        assert_eq!(
            VideoProtocol::from_url("https://example.com/video.mp4"),
            VideoProtocol::HTTP
        );
    }

    #[test]
    fn test_video_link_builder() {
        let link = VideoLink::new("rtsp://camera.local/stream")
            .with_title("Cockpit Camera")
            .with_description("Live feed from aircraft cockpit");

        assert_eq!(link.protocol, VideoProtocol::RTSP);
        assert_eq!(link.title, Some("Cockpit Camera".to_string()));
        assert_eq!(link.display_name(), "Cockpit Camera");
    }

    #[test]
    fn test_display_name_fallback() {
        let link = VideoLink::new("https://example.com/cameras/tower_cam.m3u8");
        assert_eq!(link.display_name(), "tower_cam.m3u8");
    }
}
