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

//! Aircraft type code database.
//!
//! Provides mapping from ICAO aircraft type codes (e.g., "B738") to
//! human-readable descriptions (e.g., "Boeing 737-800").

use csv::ReaderBuilder;
use log::info;
use std::collections::HashMap;
use std::path::Path;

pub struct AircraftTypeDatabase {
    type_map: HashMap<String, String>,
}

impl AircraftTypeDatabase {
    pub fn new() -> Self {
        Self {
            type_map: HashMap::new(),
        }
    }

    /// Load aircraft type mappings from aircraft.csv file
    /// CSV format: ICAO_hex;registration;type_code;category;full_name;year;owner
    /// We extract columns 3 (type_code) and 5 (full_name)
    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> Result<usize, Box<dyn std::error::Error>> {
        let mut rdr = ReaderBuilder::new()
            .delimiter(b';')
            .has_headers(false)
            .from_path(path)?;

        let mut type_map = HashMap::new();
        let mut processed = 0;

        for result in rdr.records() {
            let record = result?;

            // Need at least 6 fields (indices 0-5)
            if record.len() < 6 {
                continue;
            }

            let type_code = record[2].trim();
            let full_name = record[4].trim();

            // Only add if both type_code and full_name are non-empty
            // and this type_code hasn't been seen before (use first occurrence)
            if !type_code.is_empty() && !full_name.is_empty() {
                type_map.entry(type_code.to_string())
                    .or_insert_with(|| full_name.to_string());
                processed += 1;
            }
        }

        let unique_types = type_map.len();
        self.type_map = type_map;

        info!("Aircraft type database loaded: {} unique types from {} entries", unique_types, processed);
        Ok(unique_types)
    }

    /// Lookup full aircraft type name by ICAO type code
    /// Returns the full descriptive name if found, None otherwise
    pub fn lookup(&self, type_code: &str) -> Option<&str> {
        self.type_map.get(type_code).map(|s| s.as_str())
    }

    /// Get the number of type codes in the database
    #[must_use]
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.type_map.len()
    }

    /// Check if the database is empty
    #[must_use]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.type_map.is_empty()
    }
}

impl Default for AircraftTypeDatabase {
    fn default() -> Self {
        Self::new()
    }
}
