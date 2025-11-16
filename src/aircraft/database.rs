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

//! Aircraft registration and metadata database.
//!
//! Provides lookups from ICAO hex codes to aircraft registration numbers
//! and types. Data is loaded from compressed CSV files bundled with the
//! application.

use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct AircraftInfo {
    pub icao: String,
    pub reg: Option<String>,
    #[serde(rename = "icaotype")]
    pub icao_type: Option<String>,
    #[allow(dead_code)]
    pub year: Option<String>,
    #[allow(dead_code)]
    pub manufacturer: Option<String>,
    pub model: Option<String>,
}

pub struct AircraftDatabase {
    aircraft_map: HashMap<String, AircraftInfo>,
}

impl AircraftDatabase {
    pub fn new() -> Self {
        Self {
            aircraft_map: HashMap::new(),
        }
    }

    /// Load aircraft database from ADS-B Exchange basic-ac-db.json.gz
    /// Returns the number of aircraft loaded
    pub fn load_or_download(&mut self) -> Result<usize, Box<dyn std::error::Error>> {
        let cache_dir = dirs::cache_dir()
            .ok_or("Could not determine cache directory")?
            .join("airjedi_egui")
            .join("aircraft_db");

        fs::create_dir_all(&cache_dir)?;

        let db_path = cache_dir.join("basic-ac-db.json");

        // Download if not cached
        if !db_path.exists() {
            println!("Downloading aircraft database from ADS-B Exchange...");
            self.download_database(&db_path)?;
        } else {
            println!("Loading aircraft database from cache...");
        }

        // Load database
        self.load_from_file(&db_path)?;

        let size = self.aircraft_map.len();
        println!("Aircraft database loaded: {} aircraft", size);

        Ok(size)
    }

    fn download_database(&self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        use flate2::read::GzDecoder;

        let url = "https://downloads.adsbexchange.com/downloads/basic-ac-db.json.gz";

        let response = reqwest::blocking::get(url)?;
        let bytes = response.bytes()?;

        // Decompress gzip
        let mut decoder = GzDecoder::new(&bytes[..]);
        let mut decompressed = String::new();
        decoder.read_to_string(&mut decompressed)?;

        // Write to cache
        fs::write(path, decompressed)?;

        Ok(())
    }

    fn load_from_file(&mut self, path: &PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        let contents = fs::read_to_string(path)?;

        // Parse JSON Lines format - one JSON object per line
        let mut aircraft_map = HashMap::new();

        for line in contents.lines() {
            if line.trim().is_empty() {
                continue;
            }

            let info: AircraftInfo = serde_json::from_str(line)?;
            aircraft_map.insert(info.icao.to_uppercase(), info);
        }

        self.aircraft_map = aircraft_map;

        Ok(())
    }

    /// Lookup aircraft by ICAO hex code (e.g., "A12F3C")
    pub fn lookup(&self, icao_hex: &str) -> Option<&AircraftInfo> {
        // Try both uppercase and lowercase
        self.aircraft_map.get(&icao_hex.to_uppercase())
            .or_else(|| self.aircraft_map.get(&icao_hex.to_lowercase()))
    }

    /// Get registration number for an ICAO hex code
    pub fn get_registration(&self, icao_hex: &str) -> Option<String> {
        self.lookup(icao_hex)
            .and_then(|info| info.reg.clone())
    }

    /// Get aircraft type for an ICAO hex code
    pub fn get_aircraft_type(&self, icao_hex: &str) -> Option<String> {
        self.lookup(icao_hex).and_then(|info| {
            // Try icao_type first, then fall back to model
            info.icao_type.clone()
                .or_else(|| info.model.clone())
        })
    }
}

impl Default for AircraftDatabase {
    fn default() -> Self {
        Self::new()
    }
}
