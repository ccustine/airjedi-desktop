# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

AirJedi Desktop is a real-time ADS-B aircraft tracking application built with Rust and egui. It connects to a BaseStation protocol TCP feed (localhost:30003), displays aircraft on an interactive map with Carto tiles, and visualizes flight paths with altitude-based color coding.

## Build & Run Commands

```bash
# Build the project
cargo build

# Build optimized release version
cargo build --release

# Run the application
cargo run

# Run with release optimizations
cargo run --release
```

The application will automatically:
1. Attempt GPS location via CoreLocation (macOS) or fall back to IP geolocation
2. Connect to `localhost:30003` for BaseStation ADS-B data
3. Display aircraft within 400 miles on the map

## Architecture Overview

### Module Structure

**src/main.rs** - UI layer and application entry point
- egui-based GUI with map rendering
- GPS location detection (CoreLocation on macOS, IP fallback)
- Receiver location marker display
- Aircraft visualization with altitude-colored trails
- Map interaction (pan/pinch-zoom)
- Aviation overlay rendering (airports, runways, navaids)
- Airport filtering UI with 3 modes (FrequentlyUsed, All, MajorOnly)
- Background aviation data loading with progress indicator
- Spatial bounding box calculation for viewport-based filtering
- Constants: `TRAIL_MAX_AGE_SECONDS` (300s), `TRAIL_SOLID_DURATION_SECONDS` (225s), `TRAIL_FADE_DURATION_SECONDS` (75s)

**src/basestation.rs** - Core aircraft tracking logic
- `Aircraft` struct: Stores position, velocity, altitude, track, callsign, and position history
- `AircraftTracker`: HashMap-based aircraft registry with spatial filtering
- BaseStation MSG protocol parser (MSG,1/3/4/5/6/7/8 types)
- Position validation: 400-mile radius filter, 10-mile jump detection
- Haversine distance calculations for accuracy
- Position history management (5 minutes stored)

**src/tcp_client.rs** - Async TCP connection handler
- Connects to BaseStation feed at `localhost:30003`
- Automatic reconnection on disconnect (5-second retry)
- Message parsing loop with periodic cleanup (every 100 messages)
- Aircraft timeout: 3 minutes of inactivity

**src/tiles.rs** - Map tile management
- Web Mercator projection utilities
- Carto basemap tile fetching (dark_all theme)
- Disk cache at `~/.cache/airjedi_egui/tiles/` (7-day TTL)
- SHA256-based cache filenames
- Subdomain load balancing (a-d.basemaps.cartocdn.com)
- Async tile loading with texture management

**src/aviation_data.rs** - Aviation overlay data
- Airport, Runway, and Navaid struct definitions
- CSV parser for OurAirports data format (serde-based)
- Async download functionality (`load_or_download()`)
- Automatic download from davidmegginson.github.io on first run
- Spatial filtering with bounding box queries
- Airport filtering methods: `is_frequently_used()`, `is_public_airplane_airport()`, `has_scheduled_service()`
- Methods for zoom-based visibility control
- Color-coding by airport size and navaid type

### Key Data Flow

1. **TCP Client** receives BaseStation messages → parses → updates `AircraftTracker`
2. **AircraftTracker** validates positions (distance/jump filters) → stores in `Aircraft.position_history`
3. **Main UI** reads tracker state (via Arc<Mutex>) → renders map + aircraft + trails
4. **Tile Manager** fetches tiles on-demand → caches → provides textures to UI

### Threading Model

- **Main thread**: UI rendering (egui), map interaction, aircraft display
- **Background tokio runtime**: TCP connection, message parsing (spawned in `AdsbApp::new()`)
- **Shared state**: `Arc<Mutex<AircraftTracker>>` synchronized between threads

## Important Implementation Details

### Position Validation
Aircraft positions are rejected if they:
- Exceed 400 miles from the receiver location
- "Jump" more than 10 miles from the last known position
- Are stored in history only if they change by >100 meters

This prevents displaying erroneous data while maintaining smooth trails.

### Trail Rendering
- Trails use `position_history` Vec with timestamps
- Color coding based on altitude at each point (0-10k ft = cyan, 40k+ ft = purple)
- Opacity fades linearly over the last 7.5 minutes (solid → transparent)
- Trail segments connect consecutive points with altitude-colored lines

### Map Projection
Uses Web Mercator (EPSG:3857) for compatibility with standard web map tiles:
- `WebMercator::lat_to_y()` / `lon_to_x()` convert lat/lon to tile coordinates
- Supports zoom levels 6-12 (configured via `map_zoom_level` float)
- Center point determines which tiles are visible

### GPS Location (macOS-specific)
- On macOS: Uses CoreLocation via objc/cocoa bindings
- Requests location authorization and waits 2 seconds for GPS fix
- Falls back to IP geolocation (ipapi.co, then ip-api.com)
- Receiver position stored separately from map center to allow panning

## Configuration Points

### Aviation Data Overlays
Aviation data is automatically downloaded from OurAirports on first startup to `./data/`:
- `airports.csv` - Airport locations and metadata
- `runways.csv` - Runway endpoints and surface types
- `navaids.csv` - VOR, NDB, DME navigation aids

Files are cached and reused on subsequent runs. To force re-download, delete the `./data/` directory.

Airport filter modes:
- `AirportFilter::FrequentlyUsed` - Default, shows airports with scheduled service
- `AirportFilter::All` - All public airplane airports (excludes heliports)
- `AirportFilter::MajorOnly` - Only large international airports

### TCP Connection
Edit `src/tcp_client.rs:9`:
```rust
let address = "localhost:30003";  // Change to your BaseStation feed
```

### Distance Filters
Edit `src/basestation.rs` in `AircraftTracker::new()`:
```rust
max_distance_miles: 400.0,  // Radius filter
```

Edit `src/basestation.rs` in `Aircraft::update_position()`:
```rust
if distance_from_last > 10.0 {  // Jump detection threshold
```

### Trail Display Duration
Edit `src/main.rs` constants:
```rust
const TRAIL_MAX_AGE_SECONDS: f32 = 300.0;  // Total history stored (5 minutes)
const TRAIL_SOLID_DURATION_SECONDS: f32 = 225.0;  // Solid trail duration (75% - 3.75 minutes)
const TRAIL_FADE_DURATION_SECONDS: f32 = 75.0;  // Fade-out period (25% - 1.25 minutes)
```

### Cleanup Intervals
Edit `src/tcp_client.rs`:
```rust
const CLEANUP_INTERVAL_MESSAGES: u32 = 100;  // Cleanup frequency
const AIRCRAFT_TIMEOUT_SECONDS: i64 = 180;   // 3 minutes
```

## Platform-Specific Code

**macOS GPS support** requires `core-foundation`, `objc`, and `cocoa` dependencies (see `Cargo.toml` target-specific deps). The implementation uses unsafe Objective-C bindings to access CLLocationManager.

On other platforms, GPS location detection is skipped and IP geolocation is used directly.
