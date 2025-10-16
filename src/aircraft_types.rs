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

use std::collections::HashMap;
use std::fs;
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
        let contents = fs::read_to_string(path)?;

        let mut type_map = HashMap::new();
        let mut processed = 0;

        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Split by semicolon
            let parts: Vec<&str> = line.split(';').collect();

            // Need at least 6 parts (indices 0-5)
            if parts.len() < 6 {
                continue;
            }

            let type_code = parts[2].trim();
            let full_name = parts[4].trim();

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

        println!("Aircraft type database loaded: {} unique types from {} entries", unique_types, processed);
        Ok(unique_types)
    }

    /// Lookup full aircraft type name by ICAO type code
    /// Returns the full descriptive name if found, None otherwise
    pub fn lookup(&self, type_code: &str) -> Option<&String> {
        self.type_map.get(type_code)
    }

    /// Get the number of type codes in the database
    pub fn len(&self) -> usize {
        self.type_map.len()
    }

    /// Check if the database is empty
    pub fn is_empty(&self) -> bool {
        self.type_map.is_empty()
    }
}

impl Default for AircraftTypeDatabase {
    fn default() -> Self {
        Self::new()
    }
}
