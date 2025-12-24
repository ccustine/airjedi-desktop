# Weather Layer Design

## Overview

Add optional weather overlay tiles to the AirJedi Desktop map using OpenWeatherMap's free tile API. Weather layers are configurable in settings and render as semi-transparent overlays between the base map and aviation data.

## Requirements

- **Weather types**: Precipitation radar, cloud coverage, wind speed
- **Display method**: Tile overlays (PNG tiles at map zoom levels)
- **Provider**: OpenWeatherMap (free tier: 1,000 API calls/day)
- **Controls**: Individual toggle for each layer type
- **API key**: Environment variable first, then config field fallback

## Architecture

### Module Structure

```
src/weather/
├── mod.rs              # Module exports
├── openweathermap.rs   # OWM tile source implementation
└── config.rs           # Weather-specific config types (if needed)
```

### Layer Rendering Order

```
┌─────────────────────────────────┐
│  Aircraft icons + trails    (top)
│  Airports, Runways, Navaids     │
│  Wind tiles (if enabled)        │
│  Cloud tiles (if enabled)       │
│  Precipitation tiles (if enabled)
│  Carto dark basemap       (bottom)
└─────────────────────────────────┘
```

### OpenWeatherMap Tile URLs

```
https://tile.openweathermap.org/map/{layer}/{z}/{x}/{y}.png?appid={API_KEY}
```

Layers:
- `precipitation_new` - Rain/snow radar
- `clouds_new` - Cloud coverage
- `wind_new` - Wind speed visualization

## Configuration Changes

### New AppConfig Fields

```rust
// Weather overlay settings
pub show_weather_precipitation: bool,  // default: false
pub show_weather_clouds: bool,         // default: false
pub show_weather_wind: bool,           // default: false
pub weather_opacity: f32,              // default: 0.6 (60%)
pub openweathermap_api_key: Option<String>,  // default: None
```

### API Key Resolution Order

1. Environment variable `OPENWEATHERMAP_API_KEY`
2. Config field `openweathermap_api_key`
3. If neither exists, weather toggles are disabled

## Implementation Details

### WeatherTileSource

```rust
pub struct OpenWeatherMapSource {
    layer: WeatherLayer,
    api_key: String,
}

impl walkers::TileSource for OpenWeatherMapSource {
    fn tile_url(&self, tile_id: TileId) -> String {
        format!(
            "https://tile.openweathermap.org/map/{}/{}/{}/{}.png?appid={}",
            self.layer.as_str(),
            tile_id.zoom,
            tile_id.x,
            tile_id.y,
            self.api_key
        )
    }
}
```

### Weather Tiles Manager

```rust
struct WeatherTiles {
    precipitation: Option<HttpTiles>,
    clouds: Option<HttpTiles>,
    wind: Option<HttpTiles>,
}
```

Layers created lazily - only when enabled and API key available.

### Caching Strategy

- Path: `~/.cache/airjedi-desktop/weather/{layer}/`
- TTL: 10 minutes (weather updates frequently)
- Stale fallback: 1 hour (if fresh fetch fails)

### Opacity Rendering

```rust
fn draw_weather_layer(
    ui: &mut egui::Ui,
    projector: &Projector,
    tiles: &mut HttpTiles,
    opacity: f32,
) {
    let tint = egui::Color32::from_rgba_unmultiplied(
        255, 255, 255, (opacity * 255.0) as u8
    );
    // Draw tiles with tint applied
}
```

## UI Changes

### Settings Window - New Section

```
┌─ Weather Layers ─────────────────────────┐
│ API Key: [________________________] [?]  │
│          Using: Environment Variable ✓   │
│                                          │
│ Opacity: [=======|----] 60%              │
│                                          │
│ ☐ Precipitation (rain/snow radar)        │
│ ☐ Cloud Coverage                         │
│ ☐ Wind Speed                             │
└──────────────────────────────────────────┘
```

The `[?]` button opens OpenWeatherMap signup page.

### State Indicators

| State | Display |
|-------|---------|
| No API key | "⚠ API key required" (toggles disabled) |
| Key from env var | "✓ Using environment variable" |
| Key from config | "✓ API key configured" |
| Tiles loading | Spinner next to layer name |
| Tile fetch error | "⚠ Weather unavailable" |

## Error Handling

### API Key Validation

Test with lightweight endpoint before saving:
```
GET https://api.openweathermap.org/data/2.5/weather?lat=0&lon=0&appid={key}
```

### Graceful Degradation

- Failed tile fetches: Skip silently, don't block map
- Rate limits: Use cached tiles as fallback
- Network errors: Log to diagnostics, continue with stale cache

### Rate Limit Awareness

Free tier: 1,000 calls/day
- ~20 tiles per view × 3 layers = 60 tiles per full reload
- With caching, supports ~16 full reloads/day
- Personal use should stay well within limits

## Files to Modify

1. `src/config.rs` - Add weather config fields
2. `src/main.rs` - Add weather tile rendering, UI controls
3. `src/weather/mod.rs` - New module (create)
4. `src/weather/openweathermap.rs` - Tile source (create)

## Testing Plan

1. Unit test: API key resolution (env var vs config)
2. Unit test: Tile URL generation
3. Manual test: Enable each layer, verify tiles appear
4. Manual test: Adjust opacity slider
5. Manual test: Invalid API key shows error
6. Manual test: Tiles cache correctly (check disk)
7. Manual test: Layers render below aircraft, above basemap
