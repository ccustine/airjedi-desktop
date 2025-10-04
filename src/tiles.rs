use egui::{ColorImage, TextureHandle};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

const TILE_SIZE: u32 = 256;
const CACHE_DURATION_DAYS: u64 = 7;

/// Web Mercator projection utilities
pub struct WebMercator;

impl WebMercator {
    /// Convert latitude to Web Mercator Y coordinate (0.0 to 1.0)
    pub fn lat_to_y(lat: f64, zoom: u8) -> f64 {
        let lat_rad = lat.to_radians();
        let n = 2_f64.powi(zoom as i32);
        let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0;
        y * n
    }

    /// Convert longitude to Web Mercator X coordinate (0.0 to 1.0)
    pub fn lon_to_x(lon: f64, zoom: u8) -> f64 {
        let n = 2_f64.powi(zoom as i32);
        ((lon + 180.0) / 360.0) * n
    }

    /// Convert tile coordinates back to latitude
    #[allow(dead_code)]
    pub fn tile_to_lat(y: f64, zoom: u8) -> f64 {
        let n = 2_f64.powi(zoom as i32);
        let lat_rad = ((std::f64::consts::PI * (1.0 - 2.0 * y / n)).sinh()).atan();
        lat_rad.to_degrees()
    }

    /// Convert tile coordinates back to longitude
    #[allow(dead_code)]
    pub fn tile_to_lon(x: f64, zoom: u8) -> f64 {
        let n = 2_f64.powi(zoom as i32);
        x / n * 360.0 - 180.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TileCoord {
    pub x: u32,
    pub y: u32,
    pub zoom: u8,
}

impl TileCoord {
    pub fn new(x: u32, y: u32, zoom: u8) -> Self {
        Self { x, y, zoom }
    }

    /// Get the tile URL from Carto CDN
    pub fn url(&self) -> String {
        let subdomain = ['a', 'b', 'c', 'd'][((self.x + self.y) % 4) as usize];
        format!(
            "https://{}.basemaps.cartocdn.com/dark_all/{}/{}/{}.png",
            subdomain, self.zoom, self.x, self.y
        )
    }

    /// Get cache filename based on hash of URL
    fn cache_filename(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.url().as_bytes());
        let hash = hasher.finalize();
        format!("{:x}", hash)
    }
}

pub enum TileState {
    Loading,
    Loaded(TextureHandle),
    Failed,
}

pub struct TileManager {
    cache_dir: PathBuf,
    tiles: Arc<Mutex<HashMap<TileCoord, TileState>>>,
    download_queue: Arc<Mutex<Vec<TileCoord>>>,
}

impl Default for TileManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TileManager {
    pub fn new() -> Self {
        let cache_dir = Self::get_cache_dir();

        // Create cache directory if it doesn't exist
        if let Err(e) = fs::create_dir_all(&cache_dir) {
            eprintln!("Failed to create cache directory: {}", e);
        }

        // Clean up old tiles
        Self::cleanup_old_tiles(&cache_dir);

        Self {
            cache_dir,
            tiles: Arc::new(Mutex::new(HashMap::new())),
            download_queue: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn get_cache_dir() -> PathBuf {
        let mut path = dirs::cache_dir().unwrap_or_else(|| PathBuf::from(".cache"));
        path.push("airjedi-desktop");
        path.push("tiles");
        path
    }

    fn cleanup_old_tiles(cache_dir: &PathBuf) {
        let now = SystemTime::now();
        let max_age = Duration::from_secs(CACHE_DURATION_DAYS * 24 * 60 * 60);

        if let Ok(entries) = fs::read_dir(cache_dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(modified) = metadata.modified() {
                        if let Ok(age) = now.duration_since(modified) {
                            if age > max_age {
                                let _ = fs::remove_file(entry.path());
                                println!("Removed old tile cache: {:?}", entry.path());
                            }
                        }
                    }
                }
            }
        }
    }

    /// Get tile from cache or queue for download
    pub fn get_tile(&self, coord: TileCoord, ctx: &egui::Context) -> Option<TextureHandle> {
        let mut tiles = self.tiles.lock().unwrap();

        match tiles.get(&coord) {
            Some(TileState::Loaded(texture)) => Some(texture.clone()),
            Some(TileState::Loading) => None,
            Some(TileState::Failed) => None,
            None => {
                // Check if we have it in disk cache
                let cache_path = self.cache_dir.join(format!("{}.png", coord.cache_filename()));

                if cache_path.exists() {
                    // Load from cache
                    match self.load_tile_from_disk(&cache_path, ctx, coord) {
                        Ok(texture) => {
                            tiles.insert(coord, TileState::Loaded(texture.clone()));
                            Some(texture)
                        }
                        Err(e) => {
                            eprintln!("Failed to load cached tile: {}", e);
                            tiles.insert(coord, TileState::Loading);
                            self.queue_download(coord, ctx.clone());
                            None
                        }
                    }
                } else {
                    // Need to download
                    tiles.insert(coord, TileState::Loading);
                    self.queue_download(coord, ctx.clone());
                    None
                }
            }
        }
    }

    fn load_tile_from_disk(
        &self,
        path: &PathBuf,
        ctx: &egui::Context,
        coord: TileCoord,
    ) -> Result<TextureHandle, String> {
        let img_data = fs::read(path).map_err(|e| e.to_string())?;
        let img = image::load_from_memory(&img_data).map_err(|e| e.to_string())?;
        let rgba = img.to_rgba8();

        let color_image = ColorImage::from_rgba_unmultiplied(
            [TILE_SIZE as usize, TILE_SIZE as usize],
            &rgba.into_raw(),
        );

        Ok(ctx.load_texture(
            format!("tile_{}_{}/{}", coord.zoom, coord.x, coord.y),
            color_image,
            Default::default(),
        ))
    }

    fn queue_download(&self, coord: TileCoord, ctx: egui::Context) {
        let mut queue = self.download_queue.lock().unwrap();
        if !queue.contains(&coord) {
            queue.push(coord);

            // Spawn download task
            let tiles = self.tiles.clone();
            let cache_dir = self.cache_dir.clone();

            std::thread::spawn(move || {
                Self::download_tile(coord, tiles, cache_dir, ctx);
            });
        }
    }

    fn download_tile(
        coord: TileCoord,
        tiles: Arc<Mutex<HashMap<TileCoord, TileState>>>,
        cache_dir: PathBuf,
        ctx: egui::Context,
    ) {
        let url = coord.url();
        println!("Downloading tile: {}", url);

        match reqwest::blocking::get(&url) {
            Ok(response) => {
                if response.status().is_success() {
                    match response.bytes() {
                        Ok(bytes) => {
                            // Save to cache
                            let cache_path = cache_dir.join(format!("{}.png", coord.cache_filename()));
                            if let Err(e) = fs::write(&cache_path, &bytes) {
                                eprintln!("Failed to save tile to cache: {}", e);
                            }

                            // Load into texture
                            match image::load_from_memory(&bytes) {
                                Ok(img) => {
                                    let rgba = img.to_rgba8();
                                    let color_image = ColorImage::from_rgba_unmultiplied(
                                        [TILE_SIZE as usize, TILE_SIZE as usize],
                                        &rgba.into_raw(),
                                    );

                                    let texture = ctx.load_texture(
                                        format!("tile_{}_{}/{}", coord.zoom, coord.x, coord.y),
                                        color_image,
                                        Default::default(),
                                    );

                                    let mut tiles_lock = tiles.lock().unwrap();
                                    tiles_lock.insert(coord, TileState::Loaded(texture));
                                    ctx.request_repaint();
                                }
                                Err(e) => {
                                    eprintln!("Failed to decode tile image: {}", e);
                                    let mut tiles_lock = tiles.lock().unwrap();
                                    tiles_lock.insert(coord, TileState::Failed);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Failed to read tile bytes: {}", e);
                            let mut tiles_lock = tiles.lock().unwrap();
                            tiles_lock.insert(coord, TileState::Failed);
                        }
                    }
                } else {
                    eprintln!("Failed to download tile: HTTP {}", response.status());
                    let mut tiles_lock = tiles.lock().unwrap();
                    tiles_lock.insert(coord, TileState::Failed);
                }
            }
            Err(e) => {
                eprintln!("Failed to fetch tile: {}", e);
                let mut tiles_lock = tiles.lock().unwrap();
                tiles_lock.insert(coord, TileState::Failed);
            }
        }
    }

    /// Get all tiles needed for a viewport
    pub fn get_visible_tiles(
        &self,
        center_lat: f64,
        center_lon: f64,
        zoom: u8,
        viewport_width: f32,
        viewport_height: f32,
    ) -> Vec<(TileCoord, f32, f32)> {
        let mut tiles = Vec::new();

        // Calculate center tile
        let center_tile_x = WebMercator::lon_to_x(center_lon, zoom);
        let center_tile_y = WebMercator::lat_to_y(center_lat, zoom);

        // Calculate how many tiles we need in each direction
        let tiles_wide = (viewport_width / TILE_SIZE as f32).ceil() as i32 + 2;
        let tiles_high = (viewport_height / TILE_SIZE as f32).ceil() as i32 + 2;

        let start_x = center_tile_x.floor() as i32 - tiles_wide / 2;
        let start_y = center_tile_y.floor() as i32 - tiles_high / 2;

        let max_tile = 2_i32.pow(zoom as u32);

        for dy in 0..tiles_high {
            for dx in 0..tiles_wide {
                let tile_x = start_x + dx;
                let tile_y = start_y + dy;

                // Wrap X coordinate (longitude wraps around)
                let wrapped_x = ((tile_x % max_tile) + max_tile) % max_tile;

                // Clamp Y coordinate (latitude doesn't wrap)
                if tile_y >= 0 && tile_y < max_tile {
                    let coord = TileCoord::new(wrapped_x as u32, tile_y as u32, zoom);

                    // Calculate screen position offset from center
                    let offset_x = (tile_x as f64 - center_tile_x) * TILE_SIZE as f64;
                    let offset_y = (tile_y as f64 - center_tile_y) * TILE_SIZE as f64;

                    tiles.push((coord, offset_x as f32, offset_y as f32));
                }
            }
        }

        tiles
    }

    pub fn has_loading_tiles(&self) -> bool {
        let tiles = self.tiles.lock().unwrap();
        tiles.values().any(|state| matches!(state, TileState::Loading))
    }

    pub fn get_error_count(&self) -> usize {
        let tiles = self.tiles.lock().unwrap();
        tiles.values().filter(|state| matches!(state, TileState::Failed)).count()
    }
}
