# AirJedi Desktop - Rust Improvements Plan

This document contains deferred improvements from the comprehensive Rust code review.
Priority 1 items have been completed. This plan covers Priority 2 (Performance) and Priority 3 (Code Quality) improvements.

---

## ✅ Completed (Priority 1: High-Impact, Low-Effort)

1. **Fixed `Utc::now()` re-evaluation** - `src/main.rs:2769`
   - Captured timestamp once before iterator to avoid repeated system calls
   - Performance improvement for aircraft statistics calculation

2. **Implemented `tokio::select!` for reactive address changes** - `src/tcp_client.rs:126-175`
   - Changed from message-based polling (every 100 messages) to event-driven reconnection
   - Immediate reaction to server address changes via watch channel
   - Time-based cleanup (30s) instead of message-based for predictable behavior
   - Better async hygiene with proper cancellation handling

3. **Added config version field** - `src/config.rs:83`
   - Prevents false migration attempts when users have zero servers configured
   - Version-based migration strategy (version < 2 triggers migration)
   - Future-proof for additional schema changes

---

## Priority 2: Performance Optimizations ⚡

### 1. Use `Arc<str>` for Immutable Strings
**Location**: `src/basestation.rs:81` (AircraftData struct)

**Current Code**:
```rust
pub struct AircraftData {
    pub icao: String,  // Cloned on every accessor call
    pub callsign: Option<String>,
    // ...
}
```

**Improvement**:
```rust
pub struct AircraftData {
    pub icao: Arc<str>,  // Cheap clone via reference counting
    pub callsign: Option<String>,  // Keep mutable fields as String
    pub source_server_id: Arc<str>,  // Also immutable
    pub source_server_name: Arc<str>,  // Also immutable
    // ...
}

// Update accessor methods:
pub fn icao(&self) -> Arc<str> {
    Arc::clone(&self.inner.read().expect(...).icao)
}
```

**Benefits**:
- Reduces string clones for ICAO codes (called every frame for rendering)
- Especially impactful for aircraft list with hundreds of aircraft
- No runtime overhead - just reference counting

**Impact**: Medium - Noticeable for large aircraft counts (100+ aircraft)

---

### 2. Use Async File I/O Consistently
**Location**: `src/aviation_data.rs:414-449` (download_data_files)

**Current Code**:
```rust
pub async fn download_data_files(data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    std::fs::create_dir_all(data_dir)?;  // Blocking call in async fn

    let response = reqwest::get(*url).await?;
    let bytes = response.bytes().await?;
    std::fs::write(&file_path, &bytes)?;  // Blocking call
}
```

**Improvement**:
```rust
use tokio::fs;

pub async fn download_data_files(data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(data_dir).await?;  // Async version

    for (filename, url) in &files {
        let file_path = data_dir.join(filename);

        // Check if file exists (async)
        if fs::metadata(&file_path).await.is_ok() {
            info!("{} already exists", filename);
            continue;
        }

        info!("Downloading {} from {}...", filename, url);
        let response = reqwest::get(*url).await?;
        let bytes = response.bytes().await?;

        // Write asynchronously
        fs::write(&file_path, &bytes).await?;
        info!("Downloaded {} ({} bytes)", filename, bytes.len());
    }
    Ok(())
}
```

**Benefits**:
- Prevents blocking tokio executor during startup
- Better async hygiene and consistency
- More responsive during downloads (though this is a one-time startup operation)

**Impact**: Low - Only affects initial download, but improves code quality

---

### 3. Add R-tree Spatial Index (Optional)
**Location**: `src/aviation_data.rs:391` (get_airports_in_bounds)

**Current Code**:
```rust
pub fn get_airports_in_bounds(&self, min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) -> Vec<&Airport> {
    self.airports.iter()
        .filter(|a| a.latitude >= min_lat && a.latitude <= max_lat
                 && a.longitude >= min_lon && a.longitude <= max_lon)
        .collect()
}
```

**When to Implement**: Only if you add more data sources beyond OurAirports (~60k airports)

**Add Dependency**:
```toml
[dependencies]
rstar = "0.12"  # R-tree spatial index
```

**Implementation Sketch**:
```rust
use rstar::{RTree, AABB};

#[derive(Debug, Clone)]
struct AirportPoint {
    position: [f64; 2],  // [lon, lat]
    index: usize,        // Index into airports Vec
}

impl rstar::RTreeObject for AirportPoint {
    type Envelope = AABB<[f64; 2]>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_point(self.position)
    }
}

pub struct AviationData {
    airports: Vec<Airport>,
    airport_rtree: RTree<AirportPoint>,  // Spatial index
    // ...
}

impl AviationData {
    pub fn get_airports_in_bounds(&self, min_lat: f64, max_lat: f64, min_lon: f64, max_lon: f64) -> Vec<&Airport> {
        let bbox = AABB::from_corners([min_lon, min_lat], [max_lon, max_lat]);

        self.airport_rtree
            .locate_in_envelope(&bbox)
            .map(|point| &self.airports[point.index])
            .collect()
    }
}
```

**Benefits**:
- O(log n + k) queries instead of O(n) where k is result count
- Significant improvement with 100k+ points

**Impact**: Low for current dataset, High if you add more data sources

---

## Priority 3: Code Quality & Safety 🛡️

### 4. Extract Exponential Smoothing into Reusable Struct
**Location**: `src/main.rs:2447-2461` (scroll zoom smoothing)

**Current Code**:
```rust
// Manual smoothing implementation
let smoothing_factor = 0.7;
let target_velocity = scroll_delta.y / 300.0;
self.scroll_zoom_velocity = self.scroll_zoom_velocity * smoothing_factor
                           + target_velocity * (1.0 - smoothing_factor);
```

**Improvement**:
Create a reusable smoothing utility:

```rust
// Add new file: src/smoothing.rs

/// Exponential moving average for smooth animations
pub struct ExponentialSmoothing {
    value: f32,
    alpha: f32,  // Smoothing coefficient (0.0 = no change, 1.0 = instant)
}

impl ExponentialSmoothing {
    /// Create a new smoother
    ///
    /// # Arguments
    /// * `initial_value` - Starting value
    /// * `smoothing_factor` - How much to smooth (0.0-1.0, higher = smoother but slower)
    pub fn new(initial_value: f32, smoothing_factor: f32) -> Self {
        Self {
            value: initial_value,
            alpha: 1.0 - smoothing_factor.clamp(0.0, 1.0),
        }
    }

    /// Update with a new target value and return the smoothed result
    pub fn update(&mut self, target: f32) -> f32 {
        self.value = self.value * (1.0 - self.alpha) + target * self.alpha;
        self.value
    }

    /// Get current value without updating
    pub fn get(&self) -> f32 {
        self.value
    }

    /// Reset to a new value instantly
    pub fn reset(&mut self, value: f32) {
        self.value = value;
    }

    /// Decay towards zero (useful for velocity damping)
    pub fn decay(&mut self, decay_factor: f32) -> f32 {
        self.value *= decay_factor;
        self.value
    }
}

// Usage in main.rs:
struct AirjediApp {
    // Replace:
    // scroll_zoom_velocity: f32,

    // With:
    scroll_zoom_smoother: ExponentialSmoothing,
    // ...
}

impl AirjediApp {
    fn new(...) -> Self {
        Self {
            scroll_zoom_smoother: ExponentialSmoothing::new(0.0, 0.7),
            // ...
        }
    }
}

// In scroll handling:
if scroll_delta.y.abs() > 0.1 {
    let target_velocity = scroll_delta.y / 300.0;
    let smoothed_velocity = self.scroll_zoom_smoother.update(target_velocity);
} else {
    let smoothed_velocity = self.scroll_zoom_smoother.decay(0.8);
}
```

**Benefits**:
- Reusable for other animations (camera movement, UI transitions)
- More testable (can unit test smoothing behavior)
- Self-documenting code with clear intent
- Could be used for aircraft label animations, panel animations, etc.

**Impact**: Medium - Improves code maintainability and enables future animations

---

### 5. Add Builder Pattern for ServerConfig with Validation
**Location**: `src/config.rs:45` (ServerConfig::new)

**Current Code**:
```rust
impl ServerConfig {
    pub fn new(name: String, address: String, enabled: bool) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            address,
            enabled,
        }
    }
}
```

**Improvement**:
```rust
use std::net::ToSocketAddrs;

impl ServerConfig {
    /// Create a builder for ServerConfig
    pub fn builder() -> ServerConfigBuilder {
        ServerConfigBuilder::default()
    }

    // Keep the simple constructor for backwards compatibility
    pub fn new(name: String, address: String, enabled: bool) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            address,
            enabled,
        }
    }
}

#[derive(Default)]
pub struct ServerConfigBuilder {
    name: Option<String>,
    address: Option<String>,
    enabled: bool,
}

impl ServerConfigBuilder {
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn address(mut self, address: impl Into<String>) -> Self {
        self.address = Some(address.into());
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Build the ServerConfig, validating the address format
    pub fn build(self) -> Result<ServerConfig, String> {
        let address = self.address.ok_or("Server address is required")?;

        // Validate address format (host:port)
        if !address.contains(':') {
            return Err(format!("Invalid address format: '{}' (expected host:port)", address));
        }

        // Try to resolve the address to validate format
        // Note: This doesn't check if the server is reachable, just if the format is valid
        let validation_address = address.clone();
        if validation_address.to_socket_addrs().is_err() {
            // For hostnames that might not resolve immediately, just check format
            let parts: Vec<&str> = address.split(':').collect();
            if parts.len() != 2 {
                return Err(format!("Invalid address format: '{}'", address));
            }

            // Validate port is a number
            if parts[1].parse::<u16>().is_err() {
                return Err(format!("Invalid port number: '{}'", parts[1]));
            }
        }

        Ok(ServerConfig {
            id: Uuid::new_v4().to_string(),
            name: self.name.unwrap_or_else(|| "Unnamed Server".to_string()),
            address,
            enabled: self.enabled,
        })
    }
}

// Usage example:
let server = ServerConfig::builder()
    .name("My ADS-B Server")
    .address("192.168.1.100:30003")
    .enabled(true)
    .build()?;  // Returns Result for error handling
```

**Benefits**:
- Validation at construction time prevents invalid states
- Clear error messages for users
- Fluent API is more ergonomic
- Easy to extend with additional validation

**Usage in UI**:
```rust
// In settings window when adding new server:
match ServerConfig::builder()
    .name(server_name.clone())
    .address(server_address.clone())
    .enabled(true)
    .build()
{
    Ok(server) => {
        self.config.add_server(server);
        self.connection_manager.lock().unwrap().add_server(server);
    }
    Err(e) => {
        // Show error to user in UI
        self.system_status.lock().unwrap().add_diagnostic(
            DiagnosticLevel::Error,
            format!("Invalid server configuration: {}", e)
        );
    }
}
```

**Impact**: Medium - Prevents invalid server configurations, better UX

---

## Implementation Priority Recommendation

Based on effort vs impact:

1. **Do First**: Exponential Smoothing struct (#4)
   - Low effort, enables future animations
   - Immediate code quality improvement

2. **Do Second**: ServerConfig builder (#5)
   - Medium effort, prevents user errors
   - Better validation UX

3. **Do Third**: Arc<str> optimization (#1)
   - Low effort, measurable performance improvement
   - Profile first to confirm impact

4. **Do Fourth**: Async file I/O (#2)
   - Low effort, code hygiene improvement
   - One-time startup operation, low priority

5. **Skip for Now**: R-tree spatial index (#3)
   - High effort, not needed for current dataset
   - Implement only if adding more data sources

---

## Testing Recommendations

After implementing each improvement:

1. **Functional Testing**:
   - Run the application with typical workloads
   - Test edge cases (empty server list, invalid addresses, etc.)

2. **Performance Testing**:
   - Profile with `cargo flamegraph` before and after
   - Measure frame times with large aircraft counts (100+ aircraft)
   - Test scroll zoom smoothness

3. **Regression Testing**:
   - Verify config migration still works
   - Test server hot-reload with address changes
   - Verify aircraft statistics update correctly

4. **Platform Testing**:
   - macOS (your primary platform)
   - Raspberry Pi (ARM Linux with Glow renderer)
   - Consider Windows if supporting that platform

---

## Notes

- All improvements maintain backwards compatibility
- No breaking changes to existing APIs
- Focus on incremental improvements without major refactoring
- Can be implemented independently in any order

---

Generated from comprehensive Rust code review on 2025-10-25
