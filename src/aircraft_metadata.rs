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

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Deserialize)]
pub struct PhotoInfo {
    pub id: String,
    pub thumbnail: ThumbnailInfo,
    pub thumbnail_large: ThumbnailInfo,
    pub link: String,
    pub photographer: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThumbnailInfo {
    pub src: String,
    pub size: ThumbnailSize,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ThumbnailSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct PlanespottersResponse {
    photos: Vec<PhotoInfo>,
}

#[derive(Debug, Clone)]
pub struct AircraftMetadata {
    pub registration: Option<String>,
    pub aircraft_type: Option<String>,
    pub photo_url: Option<String>,
    pub photo_thumbnail_url: Option<String>,
    pub photographer: Option<String>,
}

struct CacheEntry {
    metadata: Option<AircraftMetadata>,
    timestamp: Instant,
}

pub struct MetadataService {
    cache: Arc<Mutex<HashMap<String, CacheEntry>>>,
    cache_ttl: Duration,
}

impl MetadataService {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
            cache_ttl: Duration::from_secs(3600 * 24), // Cache for 24 hours
        }
    }

    /// Fetch aircraft photo from planespotters.net by ICAO hex code
    pub async fn fetch_photo_by_icao(&self, icao_hex: &str) -> Option<AircraftMetadata> {
        // Check cache first
        if let Some(cached) = self.get_from_cache(icao_hex) {
            return cached;
        }

        let url = format!("https://api.planespotters.net/pub/photos/hex/{}", icao_hex.to_lowercase());

        match self.fetch_from_api(&url).await {
            Ok(metadata) => {
                self.store_in_cache(icao_hex, Some(metadata.clone()));
                Some(metadata)
            }
            Err(e) => {
                println!("Failed to fetch photo for {}: {}", icao_hex, e);
                // Cache the failure to avoid repeated requests
                self.store_in_cache(icao_hex, None);
                None
            }
        }
    }

    /// Fetch aircraft photo from planespotters.net by registration number
    pub async fn fetch_photo_by_registration(&self, registration: &str) -> Option<AircraftMetadata> {
        // Check cache first
        let cache_key = format!("reg_{}", registration);
        if let Some(cached) = self.get_from_cache(&cache_key) {
            return cached;
        }

        let url = format!("https://api.planespotters.net/pub/photos/reg/{}", registration);

        match self.fetch_from_api(&url).await {
            Ok(metadata) => {
                self.store_in_cache(&cache_key, Some(metadata.clone()));
                Some(metadata)
            }
            Err(e) => {
                println!("Failed to fetch photo for {}: {}", registration, e);
                // Cache the failure to avoid repeated requests
                self.store_in_cache(&cache_key, None);
                None
            }
        }
    }

    async fn fetch_from_api(&self, url: &str) -> Result<AircraftMetadata, Box<dyn std::error::Error + Send + Sync>> {
        let response = reqwest::get(url).await?;

        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()).into());
        }

        let data: PlanespottersResponse = response.json().await?;

        if let Some(photo) = data.photos.first() {
            Ok(AircraftMetadata {
                registration: None, // Will be filled in by caller
                aircraft_type: None, // Will be filled in by caller
                photo_url: Some(photo.thumbnail_large.src.clone()),
                photo_thumbnail_url: Some(photo.thumbnail.src.clone()),
                photographer: Some(photo.photographer.clone()),
            })
        } else {
            Err("No photos available".into())
        }
    }

    fn get_from_cache(&self, key: &str) -> Option<Option<AircraftMetadata>> {
        let cache = self.cache.lock().ok()?;

        if let Some(entry) = cache.get(key) {
            // Check if cache entry is still valid
            if entry.timestamp.elapsed() < self.cache_ttl {
                return Some(entry.metadata.clone());
            }
        }

        None
    }

    fn store_in_cache(&self, key: &str, metadata: Option<AircraftMetadata>) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(
                key.to_string(),
                CacheEntry {
                    metadata,
                    timestamp: Instant::now(),
                },
            );
        }
    }

    /// Clear old cache entries
    pub fn cleanup_cache(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            cache.retain(|_, entry| entry.timestamp.elapsed() < self.cache_ttl);
        }
    }
}

impl Default for MetadataService {
    fn default() -> Self {
        Self::new()
    }
}
