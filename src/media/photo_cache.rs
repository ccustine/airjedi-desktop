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

//! Aircraft photo texture cache and loading.
//!
//! Manages async loading of aircraft photos from URLs, conversion to egui
//! textures, and disk caching with SHA256-based filenames. Handles texture
//! lifecycle and prevents duplicate downloads.

use sha2::{Sha256, Digest};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::collections::{HashMap, HashSet};

/// Photo cache manager for aircraft thumbnails
#[derive(Clone)]
pub struct PhotoCache {
    cache_dir: PathBuf,
    pending_downloads: Arc<Mutex<HashSet<String>>>, // Track ongoing downloads
}

impl PhotoCache {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let cache_dir = dirs::cache_dir()
            .ok_or("Could not determine cache directory")?
            .join("airjedi_egui")
            .join("aircraft_photos");

        fs::create_dir_all(&cache_dir)?;

        Ok(Self {
            cache_dir,
            pending_downloads: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    /// Get cache file path for a given URL
    fn get_cache_path(&self, url: &str) -> PathBuf {
        // Use SHA256 hash of URL as filename to avoid filesystem issues
        let mut hasher = Sha256::new();
        hasher.update(url.as_bytes());
        let hash = format!("{:x}", hasher.finalize());

        // Extract file extension from URL
        let ext = url.rsplit('.').next().unwrap_or("jpg");

        self.cache_dir.join(format!("{}.{}", hash, ext))
    }

    /// Check if image is cached
    #[allow(dead_code)]
    pub fn is_cached(&self, url: &str) -> bool {
        self.get_cache_path(url).exists()
    }

    /// Get cached image bytes
    pub fn get_cached_bytes(&self, url: &str) -> Option<Vec<u8>> {
        let path = self.get_cache_path(url);
        fs::read(path).ok()
    }

    /// Download and cache an image
    pub async fn download_and_cache(&self, url: String) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Check if already downloading
        {
            let mut pending = self.pending_downloads.lock().unwrap();
            if pending.contains(&url) {
                return Err("Already downloading".into());
            }
            pending.insert(url.clone());
        }

        let result = self.download_image(&url).await;

        // Remove from pending
        self.pending_downloads.lock().unwrap().remove(&url);

        result
    }

    async fn download_image(&self, url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let response = reqwest::get(url).await?;

        if !response.status().is_success() {
            return Err(format!("HTTP error: {}", response.status()).into());
        }

        let bytes = response.bytes().await?;
        let bytes_vec = bytes.to_vec();

        // Cache to disk
        let cache_path = self.get_cache_path(url);
        fs::write(cache_path, &bytes_vec)?;

        Ok(bytes_vec)
    }

    /// Check if download is pending
    #[allow(dead_code)]
    pub fn is_pending(&self, url: &str) -> bool {
        self.pending_downloads.lock().unwrap().contains(url)
    }
}

impl Default for PhotoCache {
    fn default() -> Self {
        Self::new().expect("Failed to create photo cache")
    }
}

/// Manages loading aircraft photos into egui textures
pub struct PhotoTextureManager {
    cache: PhotoCache,
    textures: Arc<Mutex<HashMap<String, egui::TextureHandle>>>,
    loading: Arc<Mutex<HashSet<String>>>,
    placeholder: Option<egui::TextureHandle>,
}

impl PhotoTextureManager {
    pub fn new() -> Self {
        Self {
            cache: PhotoCache::new().expect("Failed to create photo cache"),
            textures: Arc::new(Mutex::new(HashMap::new())),
            loading: Arc::new(Mutex::new(HashSet::new())),
            placeholder: None,
        }
    }

    /// Initialize placeholder texture (call once during UI setup)
    pub fn init_placeholder(&mut self, ctx: &egui::Context) {
        // Create a simple gray placeholder image (48x32 pixels)
        let width = 48;
        let height = 32;
        let mut pixels = vec![egui::Color32::from_rgb(60, 60, 70); width * height];

        // Add a simple aircraft icon using ASCII art style
        // Draw a simple plane silhouette in the center
        for y in 12..20 {
            for x in 20..28 {
                pixels[y * width + x] = egui::Color32::from_rgb(100, 100, 110);
            }
        }
        // Wings
        for x in 10..38 {
            pixels[16 * width + x] = egui::Color32::from_rgb(100, 100, 110);
        }

        let image = egui::ColorImage {
            size: [width, height],
            pixels,
            source_size: egui::Vec2::new(width as f32, height as f32),
        };

        self.placeholder = Some(ctx.load_texture(
            "aircraft_placeholder",
            image,
            egui::TextureOptions::LINEAR,
        ));
    }

    /// Get or load texture for a photo URL
    pub fn get_or_load_texture(
        &self,
        ctx: &egui::Context,
        url: &str,
        icao: &str,
    ) -> Option<egui::TextureHandle> {
        // Check if already loaded
        {
            let textures = self.textures.lock().unwrap();
            if let Some(texture) = textures.get(url) {
                return Some(texture.clone());
            }
        }

        // Check if in cache
        if let Some(bytes) = self.cache.get_cached_bytes(url) {
            if let Some(texture) = self.load_texture_from_bytes(ctx, &bytes, icao) {
                self.textures.lock().unwrap().insert(url.to_string(), texture.clone());
                return Some(texture);
            }
        }

        // Check if already loading
        {
            let loading = self.loading.lock().unwrap();
            if loading.contains(url) {
                return None; // Still loading
            }
        }

        // Start download in background thread
        self.loading.lock().unwrap().insert(url.to_string());
        let cache = self.cache.clone();
        let url_clone = url.to_string();
        let textures = self.textures.clone();
        let loading = self.loading.clone();
        let ctx_clone = ctx.clone();
        let icao_clone = icao.to_string();

        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                if let Ok(bytes) = cache.download_and_cache(url_clone.clone()).await {
                    if let Some(texture) = Self::load_texture_from_bytes_static(&ctx_clone, &bytes, &icao_clone) {
                        textures.lock().unwrap().insert(url_clone.clone(), texture);
                        ctx_clone.request_repaint(); // Request UI update
                    }
                }
                loading.lock().unwrap().remove(&url_clone);
            });
        });

        None
    }

    fn load_texture_from_bytes(
        &self,
        ctx: &egui::Context,
        bytes: &[u8],
        icao: &str,
    ) -> Option<egui::TextureHandle> {
        Self::load_texture_from_bytes_static(ctx, bytes, icao)
    }

    fn load_texture_from_bytes_static(
        ctx: &egui::Context,
        bytes: &[u8],
        icao: &str,
    ) -> Option<egui::TextureHandle> {
        // Load image using the image crate
        let image = image::load_from_memory(bytes).ok()?;

        // Track original size
        let source_size = [image.width() as usize, image.height() as usize];

        // Resize to thumbnail size (48x32)
        let thumbnail = image.resize(48, 32, image::imageops::FilterType::Lanczos3);
        let rgba = thumbnail.to_rgba8();

        let size = [rgba.width() as usize, rgba.height() as usize];
        let pixels: Vec<egui::Color32> = rgba
            .pixels()
            .map(|p| egui::Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]))
            .collect();

        let color_image = egui::ColorImage {
            size,
            pixels,
            source_size: egui::Vec2::new(source_size[0] as f32, source_size[1] as f32),
        };

        Some(ctx.load_texture(
            format!("aircraft_photo_{}", icao),
            color_image,
            egui::TextureOptions::LINEAR,
        ))
    }

    /// Get placeholder texture
    pub fn get_placeholder(&self) -> Option<&egui::TextureHandle> {
        self.placeholder.as_ref()
    }

    /// Get texture if already loaded (non-blocking)
    #[allow(dead_code)]
    pub fn get_texture(&self, url: &str) -> Option<egui::TextureHandle> {
        self.textures.lock().unwrap().get(url).cloned()
    }
}

impl Default for PhotoTextureManager {
    fn default() -> Self {
        Self::new()
    }
}
