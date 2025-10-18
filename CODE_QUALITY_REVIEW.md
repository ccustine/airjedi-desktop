# Rust Code Quality Review - AirJedi Desktop

**Review Date**: 2025-10-18
**Reviewed PR**: Recent changes adding aircraft type database, CLI args, and trail improvements

---

## Critical Issues

### 1. **Return Type Antipattern** - `src/aircraft_types.rs:74`
**Current**:
```rust
pub fn lookup(&self, type_code: &str) -> Option<&String>
```

**Issue**: Returning `&String` instead of `&str` is a Rust antipattern (flagged by `clippy::ptr_arg`).

**Fix**:
```rust
pub fn lookup(&self, type_code: &str) -> Option<&str> {
    self.type_map.get(type_code).map(|s| s.as_str())
}
```

---

### 2. **Inefficient Iterator Usage** - `src/aircraft_types.rs:46`
**Current**:
```rust
let parts: Vec<&str> = line.split(';').collect();
```

**Issue**: Unnecessary heap allocation when we only need indexed access.

**Better**:
```rust
let parts: Vec<&str> = line.splitn(6, ';').collect();  // Stops after 6 parts
```

---

### 3. **Magic Numbers Everywhere** - Multiple files

**Issues in `src/basestation.rs`:**
- Line 40: `1.15078` (nautical mile conversion)
- Line 213: `20` seconds (time threshold)
- Line 215: `10.0` miles (jump detection)
- Line 218: `3` rejections
- Line 237: `0.001` degrees (~100m threshold)
- Line 330: `300` seconds (5 minutes)

**Fix**: Define constants at module level:
```rust
const NAUTICAL_MILE_CONVERSION: f64 = 1.15078;
const JUMP_DETECTION_THRESHOLD_MILES: f64 = 10.0;
const JUMP_TIME_WINDOW_SECONDS: i64 = 20;
const MAX_CONSECUTIVE_REJECTIONS: u32 = 3;
const POSITION_CHANGE_THRESHOLD_DEGREES: f64 = 0.001;
const TRAIL_HISTORY_SECONDS: i64 = 300;
```

---

### 4. **Lock Poisoning Panic Paths** - `src/basestation.rs` (multiple)
**Current**:
```rust
self.inner.read().unwrap()
self.inner.write().unwrap()
```

**Better** (makes intent clearer):
```rust
self.inner.read()
    .expect("Aircraft data lock poisoned - unrecoverable state")
```

**Note**: Panicking on poisoned locks is often the right choice, as it indicates unrecoverable state.

---

## Moderate Issues

### 5. **Missing Must-Use Annotations** - `src/aircraft_types.rs:79-85`
**Fix**:
```rust
#[must_use]
pub fn len(&self) -> usize { ... }

#[must_use]
pub fn is_empty(&self) -> bool { ... }
```

---

### 6. **Inconsistent Distance Calculation** - `src/basestation.rs:236`
**Current**:
```rust
let distance = ((lat - last_lat).powi(2) + (lon - last_lon).powi(2)).sqrt();
```

**Issue**: Uses Euclidean distance in degrees instead of haversine.

**Why it works**: At small distances (~100m), error is negligible and this is much faster.

**Best practice**: Add a comment:
```rust
// Fast Euclidean approximation - accurate enough for ~100m threshold
let distance = ((lat - last_lat).powi(2) + (lon - last_lon).powi(2)).sqrt();
```

---

### 7. **Library Code Using println!** - Multiple locations

**Locations**:
- `src/aircraft_types.rs:68`
- `src/basestation.rs:219, 226`
- `src/tcp_client.rs:33, 37, 42, 55, 58, 88`

**Issue**: Library modules shouldn't write directly to stdout. Makes testing and integration difficult.

**Better approach**: Use the `log` crate:

Add to `Cargo.toml`:
```toml
log = "0.4"
env_logger = "0.11"  # or another logger implementation
```

In code:
```rust
use log::{info, warn, error, debug};

info!("Aircraft type database loaded: {} unique types", unique_types);
warn!("Rejected position for {}: jumped {:.1} miles", data.icao, distance);
```

---

### 8. **Redundant CSV Parsing** - `src/aircraft_types.rs:39-63`

**Issue**: Project already depends on the `csv` crate (Cargo.toml:16), but manually parses CSV.

**Better approach**:
```rust
use csv::ReaderBuilder;

pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P)
    -> Result<usize, Box<dyn std::error::Error>>
{
    let mut rdr = ReaderBuilder::new()
        .delimiter(b';')
        .has_headers(false)
        .from_path(path)?;

    let mut type_map = HashMap::new();
    let mut processed = 0;

    for result in rdr.records() {
        let record = result?;
        if record.len() < 6 {
            continue;
        }

        let type_code = record[2].trim();
        let full_name = record[4].trim();

        if !type_code.is_empty() && !full_name.is_empty() {
            type_map.entry(type_code.to_string())
                .or_insert_with(|| full_name.to_string());
            processed += 1;
        }
    }

    let unique_types = type_map.len();
    self.type_map = type_map;
    Ok(unique_types)
}
```

**Benefits**: Handles quoted fields, escaping, and edge cases correctly.

---

## Minor Issues

### 9. **Non-Idiomatic Error Propagation** - `src/tcp_client.rs:70-72`

**Current**:
```rust
let mut tracker_lock = tracker.lock()
    .expect("Aircraft tracker mutex poisoned");
```

**Alternative** (for library code):
```rust
let mut tracker_lock = tracker.lock()
    .map_err(|e| format!("Aircraft tracker mutex poisoned: {}", e))?;
```

**Note**: For poisoned mutex, panic is often the right choice.

---

### 10. **Missing Input Validation** - `src/main.rs:50`

**Current**:
```rust
#[arg(short, long, default_value = "localhost:30003")]
server: String,
```

**Enhancement**:
```rust
use std::net::SocketAddr;

#[arg(
    short,
    long,
    default_value = "localhost:30003",
    value_parser = validate_server_address
)]
server: String,

fn validate_server_address(s: &str) -> Result<String, String> {
    // Basic validation
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return Err("Must be in format host:port".to_string());
    }

    // Validate port
    parts[1].parse::<u16>()
        .map_err(|_| "Invalid port number".to_string())?;

    Ok(s.to_string())
}
```

---

### 11. **Potential Integer Overflow** - `src/tcp_client.rs:79`

**Current**:
```rust
cleanup_counter += 1;
```

**Issue**: u32 could theoretically overflow after 4 billion messages.

**Fix**:
```rust
cleanup_counter = cleanup_counter.saturating_add(1);
```

Or use a `bool` flag that resets every N messages.

---

## Performance Optimizations

### 12. **Unnecessary Clones** - `src/basestation.rs:166-167`

**Current**:
```rust
pub fn position_history(&self) -> Vec<PositionPoint> {
    self.inner.read().unwrap().position_history.clone()
}
```

**Issue**: Clones entire position history on every call. For rendering trails, this happens every frame!

**Better approach - Option 1**:
```rust
pub fn with_position_history<F, R>(&self, f: F) -> R
where
    F: FnOnce(&[PositionPoint]) -> R,
{
    let data = self.inner.read().unwrap();
    f(&data.position_history)
}
```

**Better approach - Option 2**:
```rust
// Change AircraftData
pub struct AircraftData {
    position_history: Arc<Vec<PositionPoint>>,
    // ...
}

// Return Arc clone (cheap)
pub fn position_history(&self) -> Arc<Vec<PositionPoint>> {
    Arc::clone(&self.inner.read().unwrap().position_history)
}
```

---

### 13. **Redundant String Allocations** - `src/aircraft_types.rs:59-60`

**Current**:
```rust
type_map.entry(type_code.to_string())
    .or_insert_with(|| full_name.to_string());
```

**Issue**: Creates `type_code.to_string()` even if entry exists.

**Alternative**:
```rust
if !type_map.contains_key(type_code) {
    type_map.insert(type_code.to_string(), full_name.to_string());
}
```

**Note**: The `entry()` API is more idiomatic despite the allocation. This is a micro-optimization.

---

## Positive Observations ✅

1. **Proper use of Arc/Mutex for shared state** - Appropriate for egui + tokio architecture
2. **Good error handling with Result types** in new code
3. **Comprehensive documentation** - Doc comments on public APIs
4. **Platform-specific compilation** - Good use of `#[cfg(target_os = "macos")]`
5. **Const for configuration** - TRAIL_MAX_AGE_SECONDS, etc. at module level
6. **Default trait implementation** - Good Rust idiom (aircraft_types.rs:89-92)

---

## Summary & Prioritization

### High Priority
1. ✅ Change `lookup()` return type from `Option<&String>` to `Option<&str>`
2. ✅ Extract magic numbers to named constants
3. ✅ Replace println! with proper logging (log crate)

### Medium Priority
4. ⚠️ Use the existing csv crate instead of manual parsing
5. ⚠️ Add `#[must_use]` to `len()` and `is_empty()`
6. ⚠️ Add comments explaining performance trade-offs
7. ⚠️ Optimize position_history cloning (major performance win)

### Low Priority
8. 📝 Add CLI argument validation
9. 📝 Use saturating arithmetic for counters
10. 📝 Consider more specific error types instead of Box<dyn Error>

---

## Additional Notes

### Architecture Insights
- The codebase uses Arc<RwLock<T>> extensively for shared state between async and UI threads
- Common pattern in egui applications where UI runs on main thread and background tasks run in tokio runtime
- Lock poisoning handled with `.unwrap()` which is reasonable since poisoned lock = unrecoverable state

### Dependencies to Add
For logging improvements:
```toml
[dependencies]
log = "0.4"
env_logger = "0.11"  # Initialize in main()
```

### Clippy Configuration
Consider adding to `.cargo/config.toml`:
```toml
[target.'cfg(all())']
rustflags = ["-W", "clippy::all", "-W", "clippy::pedantic"]
```

Run `cargo clippy` to catch many of these issues automatically.

---

**Generated by**: Claude Code
**Review Type**: Rust Best Practices & Performance