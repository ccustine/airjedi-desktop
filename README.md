# ADSB Aircraft Tracker

A real-time aircraft tracking application built with Rust, egui, and ADSB BaseStation protocol data from a TCP stream. Features live map tiles, altitude-based flight path visualization, and intelligent position filtering.

## Features

### Real-time ADSB Data
- Connects to `localhost:30003` via TCP to receive BaseStation format aircraft data
- Automatic reconnection on connection loss
- Parses aircraft identification, position, altitude, velocity, and heading data

### Interactive Map with Carto Tiles
- **Live Map Tiles**: Displays Carto Voyager basemap tiles with Web Mercator projection
- **Tile Caching**: 7-day disk cache at `~/.cache/airjedi_egui/tiles/` for faster loading
- **Auto-centering**: Map automatically centers on your current GPS location on startup
- **Pan & Zoom Controls**:
  - Drag to pan the map
  - Two-finger pinch to zoom (trackpad)
  - Zoom slider (levels 6-12, default 8 ≈ 150 mile range)
- **Proper Attribution**: Displays "© OpenStreetMap contributors © CARTO" as required

### Altitude-Based Flight Path Trails
Aircraft trails show the last 7.5 minutes of flight history with color-coded altitude visualization:

- **0-10,000 ft**: Cyan/Teal - Low altitude (general aviation)
- **10,000-20,000 ft**: Green/Yellow - Medium-low
- **20,000-30,000 ft**: Yellow/Orange - Medium altitude
- **30,000-40,000 ft**: Orange/Red/Magenta - High altitude (cruise)
- **40,000+ ft**: Purple/Magenta - Very high altitude

Trails fade from solid color (current) to transparent (7.5 minutes old), providing both altitude and time information at a glance.

### Intelligent Position Filtering
- **400-mile radius filter**: Only shows aircraft within 400 miles of your GPS location
- **10-mile jump filter**: Rejects erroneous position updates that would move aircraft more than 10 miles instantly
- **3-minute timeout**: Automatically removes aircraft that haven't sent updates in 3 minutes

### Aircraft Information Display
Right panel shows detailed information for all tracked aircraft:
- ICAO address
- Callsign
- Position (lat/lon)
- Altitude (feet)
- Ground speed (knots)
- Track/heading (degrees)
- Last seen timestamp

### Visual Aircraft Markers
- **Red circles**: Current aircraft position
- **Directional arrows**: Heading indicator showing aircraft track
- **Labels**: Callsign and altitude displayed next to each aircraft

## Building and Running

### Prerequisites

- Rust (latest stable version)
- Cargo

### Build

```bash
cargo build --release
```

### Run

```bash
cargo run --release
```

The application will:
1. Fetch your current GPS location via IP geolocation
2. Center the map on your location
3. Connect to the ADSB feed at `localhost:30003`
4. Begin tracking aircraft within 400 miles

## Configuration

### ADSB Data Source

The TCP address is configured in `src/tcp_client.rs`. To change the data source:

```rust
let address = "localhost:30003";  // Change to your BaseStation feed
```

### Distance Filter

To change the maximum distance from center (default: 400 miles), edit `src/basestation.rs`:

```rust
pub fn new() -> Self {
    Self {
        aircraft: HashMap::new(),
        center_lat: 0.0,
        center_lon: 0.0,
        max_distance_miles: 400.0,  // Change this value
    }
}
```

### Trail Duration

To change the trail display duration (default: 7.5 minutes), edit `src/main.rs`:

```rust
let gradient_duration = 450.0;  // seconds (450 = 7.5 minutes)
```

## Project Structure

```
src/
├── main.rs        # UI implementation with egui and map rendering
├── basestation.rs # BaseStation protocol decoder and aircraft tracking
├── tcp_client.rs  # TCP connection handler for BaseStation data
└── tiles.rs       # Map tile management and Web Mercator projection
```

## How It Works

1. **GPS Location**: Fetches current location via IP geolocation (ipapi.co or ip-api.com)
2. **TCP Connection**: Connects to BaseStation feed and maintains connection with automatic reconnection
3. **BaseStation Protocol Decoding**: Parses MSG format messages containing aircraft data
4. **Position Filtering**: Validates positions are within 400 miles of center and haven't "jumped" >10 miles
5. **Trail Recording**: Stores position history with altitude for each aircraft (15 minutes of data)
6. **Map Rendering**: Downloads and caches Carto map tiles, renders with Web Mercator projection
7. **Trail Visualization**: Draws altitude-colored, time-faded flight paths
8. **Real-time UI**: Updates every 500ms to show latest aircraft positions

## BaseStation Protocol

The application decodes BaseStation MSG format messages:
- **MSG,1**: Aircraft identification (callsign)
- **MSG,3**: Airborne position (lat/lon/altitude)
- **MSG,4**: Airborne velocity (speed/heading)
- **MSG,5**: Surveillance altitude
- **MSG,6**: Surveillance position
- **MSG,7**: Air-to-air message
- **MSG,8**: All call reply

## UI Controls

### Map
- **Drag**: Pan the map
- **Two-finger pinch**: Zoom in/out (trackpad)
- **Zoom slider**: Adjust zoom level (6-12)
- **Blue circle**: Indicates map center point (your GPS location)

### Aircraft List
- **Scrollable list**: All tracked aircraft within range
- **Resizable panel**: Drag the divider to adjust panel width

## Technical Details

### Map Tiles
- Provider: Carto Voyager basemap
- Projection: Web Mercator (EPSG:3857)
- Tile size: 256×256 pixels
- Cache location: `~/.cache/airjedi_egui/tiles/`
- Cache duration: 7 days
- Load balancing: Subdomains a-d for parallel downloads

### Position Validation
- Haversine formula for accurate distance calculation
- Center radius check: 400 miles
- Jump detection: 10 miles between consecutive positions
- Position resolution: ~100 meters (minimum change to record)

### Performance
- Trail history: 15 minutes stored, 7.5 minutes displayed
- Update frequency: 500ms (2 Hz)
- Cleanup interval: Every 100 received messages
- Aircraft timeout: 3 minutes of no updates

## Future Enhancements

Potential improvements:
- Click on aircraft for detailed popup
- Configurable filters (altitude, speed, etc.)
- Export data to KML/GeoJSON
- Multiple data source support
- Weather layer overlay
- Airport/waypoint markers
- Custom color schemes
