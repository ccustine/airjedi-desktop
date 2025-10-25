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

//! Aviation data loading and spatial queries.
//!
//! This module provides access to airport, runway, and navaid data from
//! OurAirports dataset. It supports automatic downloading of CSV files,
//! spatial bounding box queries, and filtering by airport type and service.
//!
//! Data sources:
//! - Airports: Global airport database with ICAO codes and types
//! - Runways: Runway endpoints and surface information
//! - Navaids: VOR, NDB, DME navigation aids with frequencies

use log::info;
use serde::Deserialize;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use crate::video_protocol::VideoLink;

/// Airport data from OurAirports
#[derive(Debug, Clone, Deserialize)]
pub struct Airport {
    #[serde(rename = "ident")]
    pub icao: String,

    #[serde(rename = "type")]
    pub airport_type: String,

    #[serde(rename = "name")]
    pub name: String,

    #[serde(rename = "latitude_deg")]
    pub latitude: f64,

    #[serde(rename = "longitude_deg")]
    pub longitude: f64,

    #[serde(rename = "elevation_ft")]
    pub elevation: Option<i32>,

    #[serde(rename = "scheduled_service")]
    pub scheduled_service: String,

    /// Video stream links (not from CSV, populated at runtime)
    #[serde(skip, default)]
    pub video_links: Vec<VideoLink>,
}

impl Airport {
    /// Determine if this is a major airport (for rendering priority)
    pub fn is_major(&self) -> bool {
        self.airport_type == "large_airport"
    }

    /// Determine if this is a medium-sized airport
    pub fn is_medium(&self) -> bool {
        self.airport_type == "medium_airport"
    }

    /// Check if this is a small airport
    #[allow(dead_code)]
    pub fn is_small(&self) -> bool {
        self.airport_type == "small_airport"
    }

    /// Check if this airport has scheduled commercial airline service
    pub fn has_scheduled_service(&self) -> bool {
        self.scheduled_service == "yes"
    }

    /// Check if this is a public-use airport (airplane-accessible)
    /// Excludes heliports, seaplane bases, balloonports, and closed airports
    pub fn is_public_airplane_airport(&self) -> bool {
        matches!(
            self.airport_type.as_str(),
            "large_airport" | "medium_airport" | "small_airport"
        )
    }

    /// Check if this is a frequently-used public airport
    /// (has scheduled service OR is a large/medium airport)
    pub fn is_frequently_used(&self) -> bool {
        self.has_scheduled_service() || self.is_major() || self.is_medium()
    }

    /// Get rendering radius based on airport type
    pub fn render_radius(&self) -> f32 {
        match self.airport_type.as_str() {
            "large_airport" => 6.0,
            "medium_airport" => 4.0,
            "small_airport" => 3.0,
            _ => 2.0,
        }
    }
}

/// Runway data from OurAirports
#[derive(Debug, Clone, Deserialize)]
pub struct Runway {
    #[serde(rename = "airport_ident")]
    pub airport_icao: String,

    #[allow(dead_code)]
    #[serde(rename = "length_ft")]
    pub length_ft: Option<i32>,

    #[allow(dead_code)]
    #[serde(rename = "width_ft")]
    pub width_ft: Option<i32>,

    #[serde(rename = "surface")]
    pub surface: String,

    #[allow(dead_code)]
    #[serde(rename = "lighted")]
    pub lighted: Option<i32>,

    #[serde(rename = "closed")]
    pub closed: Option<i32>,

    #[allow(dead_code)]
    #[serde(rename = "le_ident")]
    pub le_ident: String,

    #[serde(rename = "le_latitude_deg")]
    pub le_latitude: Option<f64>,

    #[serde(rename = "le_longitude_deg")]
    pub le_longitude: Option<f64>,

    #[allow(dead_code)]
    #[serde(rename = "he_ident")]
    pub he_ident: String,

    #[serde(rename = "he_latitude_deg")]
    pub he_latitude: Option<f64>,

    #[serde(rename = "he_longitude_deg")]
    pub he_longitude: Option<f64>,
}

impl Runway {
    /// Check if this runway is active (not closed)
    pub fn is_active(&self) -> bool {
        self.closed.unwrap_or(0) == 0
    }

    /// Check if both endpoints have valid coordinates
    pub fn has_valid_endpoints(&self) -> bool {
        self.le_latitude.is_some()
            && self.le_longitude.is_some()
            && self.he_latitude.is_some()
            && self.he_longitude.is_some()
    }

    /// Get stroke width based on runway surface
    pub fn stroke_width(&self) -> f32 {
        match self.surface.as_str() {
            "ASP" | "CON" | "ASPH-G" => 2.0, // Paved runways
            _ => 1.0, // Unpaved runways
        }
    }
}

/// Navaid data from OurAirports
#[derive(Debug, Clone, Deserialize)]
pub struct Navaid {
    #[serde(rename = "ident")]
    pub ident: String,

    #[serde(rename = "name")]
    pub name: String,

    #[serde(rename = "type")]
    pub navaid_type: String,

    #[serde(rename = "frequency_khz")]
    pub frequency_khz: Option<i32>,

    #[serde(rename = "latitude_deg")]
    pub latitude: f64,

    #[serde(rename = "longitude_deg")]
    pub longitude: f64,
}

impl Navaid {
    /// Get color based on navaid type
    pub fn get_color(&self) -> (u8, u8, u8) {
        match self.navaid_type.as_str() {
            "VOR" | "VORTAC" | "VOR-DME" => (100, 200, 255), // Blue for VORs
            "NDB" => (255, 200, 100), // Orange for NDBs
            "DME" => (200, 100, 255), // Purple for DME
            _ => (150, 150, 150), // Gray for others
        }
    }

    /// Get symbol size based on type
    pub fn symbol_size(&self) -> f32 {
        match self.navaid_type.as_str() {
            "VOR" | "VORTAC" => 5.0,
            "VOR-DME" => 4.5,
            "NDB" => 4.0,
            _ => 3.5,
        }
    }
}

/// Fixed camera location (not associated with airports or aircraft)
/// These are standalone cameras at specific geographic coordinates
#[derive(Debug, Clone)]
pub struct FixedCameraLocation {
    /// Unique identifier for this location
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Latitude in degrees
    pub latitude: f64,

    /// Longitude in degrees
    pub longitude: f64,

    /// Optional description of what this location monitors
    pub description: Option<String>,

    /// Video stream links for this location
    pub video_links: Vec<VideoLink>,
}

impl FixedCameraLocation {
    /// Create a new fixed camera location
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        latitude: f64,
        longitude: f64,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            latitude,
            longitude,
            description: None,
            video_links: Vec::new(),
        }
    }

    /// Builder method to add description
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Builder method to add a video link
    #[must_use]
    pub fn with_video_link(mut self, link: VideoLink) -> Self {
        self.video_links.push(link);
        self
    }

    /// Add a video link to this location
    pub fn add_video_link(&mut self, link: VideoLink) {
        self.video_links.push(link);
    }
}

/// Container for all aviation data
#[derive(Debug, Default)]
pub struct AviationData {
    pub airports: Vec<Airport>,
    pub runways: Vec<Runway>,
    pub navaids: Vec<Navaid>,
}

impl AviationData {
    /// Create a new empty AviationData
    pub fn new() -> Self {
        Self::default()
    }

    /// Load airports from CSV file
    pub fn load_airports<P: AsRef<Path>>(&mut self, path: P) -> Result<usize, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut csv_reader = csv::Reader::from_reader(reader);

        let mut count = 0;
        for result in csv_reader.deserialize() {
            let airport: Airport = result?;
            self.airports.push(airport);
            count += 1;
        }

        info!("Loaded {} airports", count);
        Ok(count)
    }

    /// Load runways from CSV file
    pub fn load_runways<P: AsRef<Path>>(&mut self, path: P) -> Result<usize, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut csv_reader = csv::Reader::from_reader(reader);

        let mut count = 0;
        for result in csv_reader.deserialize() {
            let runway: Runway = result?;
            // Only load active runways with valid endpoints
            if runway.is_active() && runway.has_valid_endpoints() {
                self.runways.push(runway);
                count += 1;
            }
        }

        info!("Loaded {} runways", count);
        Ok(count)
    }

    /// Load navaids from CSV file
    pub fn load_navaids<P: AsRef<Path>>(&mut self, path: P) -> Result<usize, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut csv_reader = csv::Reader::from_reader(reader);

        let mut count = 0;
        for result in csv_reader.deserialize() {
            let navaid: Navaid = result?;
            self.navaids.push(navaid);
            count += 1;
        }

        info!("Loaded {} navaids", count);
        Ok(count)
    }

    /// Load all aviation data from a directory containing the CSV files
    #[allow(dead_code)]
    pub fn load_from_directory<P: AsRef<Path>>(directory: P) -> Result<Self, Box<dyn std::error::Error>> {
        let mut data = Self::new();

        let dir = directory.as_ref();

        // Try to load each file, but don't fail if some are missing
        let airports_path = dir.join("airports.csv");
        if airports_path.exists() {
            if let Err(e) = data.load_airports(&airports_path) {
                eprintln!("Failed to load airports: {}", e);
            }
        } else {
            eprintln!("airports.csv not found at {:?}", airports_path);
        }

        let runways_path = dir.join("runways.csv");
        if runways_path.exists() {
            if let Err(e) = data.load_runways(&runways_path) {
                eprintln!("Failed to load runways: {}", e);
            }
        } else {
            eprintln!("runways.csv not found at {:?}", runways_path);
        }

        let navaids_path = dir.join("navaids.csv");
        if navaids_path.exists() {
            if let Err(e) = data.load_navaids(&navaids_path) {
                eprintln!("Failed to load navaids: {}", e);
            }
        } else {
            eprintln!("navaids.csv not found at {:?}", navaids_path);
        }

        Ok(data)
    }

    /// Get airports within a geographic bounding box
    pub fn get_airports_in_bounds(&self, min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) -> Vec<&Airport> {
        self.airports.iter()
            .filter(|a| a.latitude >= min_lat && a.latitude <= max_lat
                     && a.longitude >= min_lon && a.longitude <= max_lon)
            .collect()
    }

    /// Get runways for a specific airport
    pub fn get_runways_for_airport(&self, airport_icao: &str) -> Vec<&Runway> {
        self.runways.iter()
            .filter(|r| r.airport_icao == airport_icao)
            .collect()
    }

    /// Get navaids within a geographic bounding box
    pub fn get_navaids_in_bounds(&self, min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) -> Vec<&Navaid> {
        self.navaids.iter()
            .filter(|n| n.latitude >= min_lat && n.latitude <= max_lat
                     && n.longitude >= min_lon && n.longitude <= max_lon)
            .collect()
    }

    /// Download aviation data files if they don't exist
    pub async fn download_data_files(data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
        const AIRPORTS_URL: &str = "https://davidmegginson.github.io/ourairports-data/airports.csv";
        const RUNWAYS_URL: &str = "https://davidmegginson.github.io/ourairports-data/runways.csv";
        const NAVAIDS_URL: &str = "https://davidmegginson.github.io/ourairports-data/navaids.csv";

        // Create data directory if it doesn't exist
        std::fs::create_dir_all(data_dir)?;

        let files = [
            ("airports.csv", AIRPORTS_URL),
            ("runways.csv", RUNWAYS_URL),
            ("navaids.csv", NAVAIDS_URL),
        ];

        for (filename, url) in &files {
            let file_path = data_dir.join(filename);

            // Skip if file already exists
            if file_path.exists() {
                info!("{} already exists, skipping download", filename);
                continue;
            }

            info!("Downloading {} from {}...", filename, url);

            // Download the file
            let response = reqwest::get(*url).await?;
            let bytes = response.bytes().await?;

            // Write to file
            std::fs::write(&file_path, &bytes)?;
            info!("Downloaded {} ({} bytes)", filename, bytes.len());
        }

        Ok(())
    }

    /// Load aviation data from directory, downloading files if needed
    pub async fn load_or_download(data_dir: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        // Download files if they don't exist
        Self::download_data_files(&data_dir).await?;

        // Load the data
        let mut data = Self::new();

        let airports_path = data_dir.join("airports.csv");
        if airports_path.exists() {
            if let Err(e) = data.load_airports(&airports_path) {
                eprintln!("Failed to load airports: {}", e);
            }
        }

        let runways_path = data_dir.join("runways.csv");
        if runways_path.exists() {
            if let Err(e) = data.load_runways(&runways_path) {
                eprintln!("Failed to load runways: {}", e);
            }
        }

        let navaids_path = data_dir.join("navaids.csv");
        if navaids_path.exists() {
            if let Err(e) = data.load_navaids(&navaids_path) {
                eprintln!("Failed to load navaids: {}", e);
            }
        }

        Ok(data)
    }
}
