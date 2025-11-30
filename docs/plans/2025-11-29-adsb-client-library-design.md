# ADS-B Client Library Design

Modular, reusable library for connecting to and parsing ADS-B data feeds.

## Goals

- Extract ADS-B connection and parsing logic from airjedi-desktop into a standalone library
- Support use in different UI applications
- Extensible protocol support (BaseStation/SBS-1 now, BEAST/AVR/others later)
- Clean separation between parsing, state tracking, and network layers
- Tokio-based async runtime

## Architecture

Three independent, composable layers:

```
┌─────────────────────────────────────────────────┐
│            Connection Layer (tcp)               │
│  Async TCP with reconnection, address hot-reload│
│         Produces: raw bytes/lines               │
└─────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────┐
│            Protocol Layer (protocol)            │
│  BaseStation parser, extensible for BEAST, etc. │
│         Produces: typed Message enums           │
└─────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────┐
│            Tracker Layer (tracker)              │
│  Aircraft state, position history, validation   │
│  Produces: Aircraft structs + change events     │
└─────────────────────────────────────────────────┘
```

Each layer can be used independently. Consumers tap in at any level.

## Module Structure

```
src/
  lib.rs          # Re-exports, top-level Client type
  protocol/       # Message parsing (BaseStation, future: BEAST, AVR)
  tracker/        # Aircraft state management
  tcp/            # Async connection handling
```

## Protocol Layer

Trait-based abstraction for extensible protocol support.

### Protocol Trait

```rust
pub trait Protocol {
    type Message;
    type Error;

    fn parse(&mut self, input: &[u8]) -> Result<Option<Self::Message>, Self::Error>;
}
```

### Unified Message Type

```rust
pub enum AircraftMessage {
    Identification {
        icao: String,
        callsign: String,
    },
    Position {
        icao: String,
        latitude: f64,
        longitude: f64,
        altitude: Option<i32>,
    },
    Velocity {
        icao: String,
        speed: f64,
        track: f64,
        vertical_rate: Option<i32>,
    },
}
```

### BaseStation Implementation

```rust
pub struct BaseStationParser;

impl Protocol for BaseStationParser {
    type Message = AircraftMessage;
    type Error = ParseError;

    fn parse(&mut self, line: &[u8]) -> Result<Option<AircraftMessage>, ParseError> {
        // Parse CSV, return appropriate variant
    }
}
```

### Standalone Usage

```rust
let mut parser = BaseStationParser::new();
for line in lines {
    if let Some(msg) = parser.parse(line.as_bytes())? {
        println!("Got: {:?}", msg);
    }
}
```

## Tracker Layer

Maintains aircraft state and emits change events. Decoupled from protocols.

### Core Types

```rust
pub struct Aircraft {
    pub icao: String,
    pub callsign: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<i32>,
    pub track: Option<f64>,
    pub velocity: Option<f64>,
    pub vertical_rate: Option<i32>,
    pub last_seen: DateTime<Utc>,
    pub position_history: Vec<PositionPoint>,
}

pub struct PositionPoint {
    pub lat: f64,
    pub lon: f64,
    pub altitude: Option<i32>,
    pub timestamp: DateTime<Utc>,
}
```

### Change Events

```rust
pub enum TrackerEvent {
    AircraftAdded(String),
    PositionUpdated(String),
    AircraftRemoved(String),
}
```

### Configuration

```rust
pub struct TrackerConfig {
    pub center: Option<(f64, f64)>,
    pub max_distance_miles: f64,          // Default 400
    pub jump_threshold_miles: f64,        // Default 10
    pub aircraft_timeout_secs: i64,       // Default 180
    pub position_history_secs: i64,       // Default 300
}
```

### API

```rust
impl AircraftTracker {
    pub fn new(config: TrackerConfig) -> Self;
    pub fn process_message(&mut self, msg: AircraftMessage);
    pub fn get_aircraft(&self) -> Vec<&Aircraft>;
    pub fn get_by_icao(&self, icao: &str) -> Option<&Aircraft>;
    pub fn subscribe(&self) -> broadcast::Receiver<TrackerEvent>;
    pub fn cleanup_stale(&mut self);
}
```

## Connection Layer

Async TCP with automatic reconnection and address hot-reload.

### Types

```rust
pub struct ConnectionConfig {
    pub address: String,
    pub reconnect_delay: Duration,        // Default 5 secs
    pub read_timeout: Option<Duration>,
}

pub enum ConnectionState {
    Connecting,
    Connected,
    Disconnected,
    Error(String),
}

pub enum ConnectionEvent {
    StateChanged(ConnectionState),
    DataReceived(Vec<u8>),
}
```

### Connection Handle

```rust
pub struct Connection {
    event_rx: mpsc::Receiver<ConnectionEvent>,
    address_tx: watch::Sender<String>,
    cancel_token: CancellationToken,
}

impl Connection {
    pub fn spawn(config: ConnectionConfig) -> Self;
    pub async fn recv(&mut self) -> Option<ConnectionEvent>;
    pub fn set_address(&self, address: String);
    pub fn shutdown(&self);
}
```

### Standalone Usage

```rust
let mut conn = Connection::spawn(ConnectionConfig {
    address: "localhost:30003".into(),
    ..Default::default()
});

while let Some(event) = conn.recv().await {
    match event {
        ConnectionEvent::DataReceived(line) => { /* feed to parser */ }
        ConnectionEvent::StateChanged(state) => { /* update UI */ }
    }
}
```

## Top-Level Client

Wires all layers together for full-stack usage.

### Types

```rust
pub struct Client {
    tracker: Arc<RwLock<AircraftTracker>>,
    connection: Connection,
    tracker_events: broadcast::Receiver<TrackerEvent>,
}

pub struct ClientConfig {
    pub connection: ConnectionConfig,
    pub tracker: TrackerConfig,
    pub protocol: ProtocolType,
}

pub enum ProtocolType {
    BaseStation,
    // Future: Beast, Avr, etc.
}
```

### API

```rust
impl Client {
    pub fn spawn(config: ClientConfig) -> Self;
    pub fn get_aircraft(&self) -> Vec<Aircraft>;
    pub fn get_by_icao(&self, icao: &str) -> Option<Aircraft>;
    pub fn subscribe(&self) -> broadcast::Receiver<TrackerEvent>;
    pub fn connection_state(&self) -> ConnectionState;
    pub fn set_address(&self, address: String);
    pub fn shutdown(&self);
}
```

### Full Example

```rust
let client = Client::spawn(ClientConfig {
    connection: ConnectionConfig {
        address: "localhost:30003".into(),
        ..Default::default()
    },
    tracker: TrackerConfig {
        center: Some((33.9425, -118.4081)),
        max_distance_miles: 200.0,
        ..Default::default()
    },
    protocol: ProtocolType::BaseStation,
});

// Polling approach
loop {
    for aircraft in client.get_aircraft() {
        println!("{}: {:?}", aircraft.icao, aircraft.callsign);
    }
    tokio::time::sleep(Duration::from_secs(1)).await;
}

// Reactive approach
let mut events = client.subscribe();
while let Ok(event) = events.recv().await {
    match event {
        TrackerEvent::AircraftAdded(icao) => { /* ... */ }
        TrackerEvent::PositionUpdated(icao) => { /* ... */ }
        TrackerEvent::AircraftRemoved(icao) => { /* ... */ }
    }
}
```

## Error Handling

```rust
// Protocol layer
pub enum ParseError {
    InvalidFormat(String),
    MissingField(&'static str),
    InvalidValue { field: &'static str, value: String },
}

// Connection layer
pub enum ConnectionError {
    Io(std::io::Error),
    Timeout,
    Shutdown,
}

// Top-level
pub enum ClientError {
    Connection(ConnectionError),
    Parse(ParseError),
}
```

## Migration Path

### Code to Extract from airjedi-desktop

1. `src/aircraft/tracker.rs` - Position validation, haversine distance, Aircraft/AircraftTracker
2. `src/network/tcp_client.rs` - Connection logic, reconnection, hot-reload

### Changes During Extraction

- Remove app-specific fields (video_links, photo_url, metadata_fetched)
- Remove SystemStatus dependency (use TrackerEvent instead)
- Generalize parse_basestation_message into Protocol trait
- Keep haversine and position validation logic intact

### What Stays in airjedi-desktop

- Video link management
- Photo/metadata services
- UI-specific aircraft wrapper with app data
- Status/telemetry tracking (hook into TrackerEvent)

## Crate Name Options

- `adsb-client`
- `sbs1-client`
- `basestation-rs`
- `adsb-feed`

## Dependencies

```toml
[dependencies]
tokio = { version = "1", features = ["net", "io-util", "sync", "time"] }
tokio-util = "0.7"
chrono = { version = "0.4", features = ["serde"] }
log = "0.4"
thiserror = "1"
```
