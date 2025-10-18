# Cross-Compiling AirJedi Desktop for Raspberry Pi 5

This guide explains how to cross-compile AirJedi Desktop from macOS (or Linux) to run on Raspberry Pi 5.

## Target Platform

- **Device**: Raspberry Pi 5
- **Architecture**: aarch64 (64-bit ARM Cortex-A76)
- **OS**: Raspberry Pi OS (64-bit) or Ubuntu 64-bit
- **Target Triple**: `aarch64-unknown-linux-gnu` or `aarch64-unknown-linux-musl`

## Quick Start for Apple Silicon Macs (Recommended)

**If you're on Apple Silicon (M1/M2/M3/M4 Mac)**, use this method for native ARM64→ARM64 compilation:

### 1. Install musl-cross toolchain

```bash
brew install filosottile/musl-cross/musl-cross
```

### 2. Add Rust target

```bash
rustup target add aarch64-unknown-linux-musl
```

### 3. Build for Raspberry Pi 5

```bash
cargo build --target aarch64-unknown-linux-musl --release
```

The binary will be created at:
```
target/aarch64-unknown-linux-musl/release/airjedi-desktop
```

**Why musl for Apple Silicon?**
- ✅ **Native compilation**: No emulation, no QEMU crashes
- ✅ **Fast builds**: 2-3 minutes (vs 20+ minutes with containers)
- ✅ **Static linking**: Binary runs on any ARM64 Linux without dependencies
- ✅ **Optimized**: Built with `-C target-cpu=cortex-a76` for Raspberry Pi 5

**Note**: The `cross` tool (Method 1 below) doesn't work reliably on Apple Silicon because cross-rs only publishes x86_64 container images, requiring unstable QEMU emulation.

---

## Prerequisites

### On Your Development Machine (macOS/Linux)

- Rust toolchain installed (1.70+)
- Docker installed and running (for Method 1 - recommended)
- OR aarch64 cross-compilation toolchain (for Method 2)

### On Raspberry Pi 5

The Pi will need the following packages installed:

```bash
sudo apt update
sudo apt install -y \
    libgl1-mesa-dev \
    libgl1-mesa-dri \
    libx11-dev \
    libxcursor-dev \
    libxrandr-dev \
    libxi-dev \
    libxinerama-dev
```

## Method 1: Using `cross` (Recommended for x86_64/Intel Macs)

**⚠️ Note**: This method **does not work on Apple Silicon Macs**. Use the Quick Start method above instead.

This method uses Docker containers with pre-configured cross-compilation environments.

### 1. Install `cross`

```bash
cargo install cross --git https://github.com/cross-rs/cross
```

### 2. Build for Raspberry Pi 5

```bash
# From the project root directory
cross build --target aarch64-unknown-linux-gnu --release
```

The binary will be created at:
```
target/aarch64-unknown-linux-gnu/release/airjedi-desktop
```

### Configuration

The project includes `Cross.toml` which configures the Docker image and build settings. The configuration:
- Uses the official cross-rs aarch64 image
- Includes OpenGL/Mesa development libraries
- Optimizes for Raspberry Pi 5's Cortex-A76 CPU (via `.cargo/config.toml`)

## Method 2: Manual Cross-Compilation

If you prefer not to use Docker or need more control over the build process.

### 1. Add Rust Target

```bash
rustup target add aarch64-unknown-linux-gnu
```

### 2. Install aarch64 Linker (macOS)

Using Homebrew's musl-cross:

```bash
brew install filosottile/musl-cross/musl-cross
```

Or using a pre-built aarch64 toolchain:

```bash
brew install aarch64-elf-gcc
```

### 3. Configure Linker

Edit `.cargo/config.toml` and uncomment the linker line:

```toml
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"  # Uncomment this line
rustflags = ["-C", "target-cpu=cortex-a76"]
```

### 4. Build

```bash
cargo build --target aarch64-unknown-linux-gnu --release
```

## Deploying to Raspberry Pi 5

### 1. Copy Binary to Pi

```bash
# Replace <pi-ip> with your Raspberry Pi's IP address or hostname

# If you built with musl (Apple Silicon method):
scp target/aarch64-unknown-linux-musl/release/airjedi-desktop pi@<pi-ip>:~/

# If you built with cross (Intel Mac / x86_64 method):
# scp target/aarch64-unknown-linux-gnu/release/airjedi-desktop pi@<pi-ip>:~/
```

### 2. Copy Data Files

The application needs the aircraft type database:

```bash
# Create data directory on Pi
ssh pi@<pi-ip> "mkdir -p ~/airjedi-data"

# Copy aircraft database
scp data/aircraft.csv pi@<pi-ip>:~/airjedi-data/
```

### 3. Set Permissions and Run

```bash
ssh pi@<pi-ip>
chmod +x ~/airjedi-desktop

# Run the application
cd ~
./airjedi-desktop
```

## Application Setup on Raspberry Pi 5

### BaseStation ADS-B Feed

The application connects to `localhost:30003` for BaseStation protocol data. You'll need:

1. **An ADS-B receiver** (RTL-SDR dongle or similar)
2. **dump1090 or readsb** running on the Pi:

```bash
# Install readsb (recommended)
sudo apt install readsb

# Or dump1090-fa
sudo apt install dump1090-fa
```

These tools decode ADS-B data and provide it on port 30003.

### Display Configuration

For best performance:

1. **Use X11** (recommended for Raspberry Pi 5):
   - Set `DISPLAY=:0` if running remotely
   - Or use VNC for remote GUI access

2. **Hardware Acceleration**:
   - Raspberry Pi 5 has built-in GPU acceleration
   - Mesa drivers should be automatically configured
   - Verify with: `glxinfo | grep "OpenGL version"`

### Network Configuration

The app requires internet access for:
- **IP Geolocation** (ipapi.co, ip-api.com)
- **Map Tiles** (Carto basemaps via CDN)
- **Aviation Data** (OurAirports databases - downloaded once)
- **Aircraft Photos** (Planespotters API)

Cached data is stored in:
- `~/.cache/airjedi_egui/tiles/` - Map tiles (7-day cache)
- `./data/` - Aviation databases (airports, runways, navaids)

## Performance Optimization

### Raspberry Pi 5 Specific

The build is optimized for Raspberry Pi 5's Cortex-A76 cores via the rustflags in `.cargo/config.toml`:

```toml
rustflags = ["-C", "target-cpu=cortex-a76"]
```

This enables ARM NEON SIMD instructions and other CPU-specific optimizations.

### Runtime Tips

1. **Run in release mode** (already built with `--release`)
2. **Allocate GPU memory**: Edit `/boot/config.txt`:
   ```
   gpu_mem=256
   ```
3. **Enable OpenGL**: Ensure KMS (Kernel Mode Setting) is enabled:
   ```
   dtoverlay=vc4-kms-v3d
   ```

## Troubleshooting

### Binary Won't Run

**Error: `cannot execute binary file: Exec format error`**
- Verify you're on 64-bit Raspberry Pi OS: `uname -m` should show `aarch64`
- If it shows `armv7l`, you need to install 64-bit OS

### OpenGL/Graphics Issues

**Error: `failed to create EGL context`**
- Install Mesa drivers: `sudo apt install libgl1-mesa-dri`
- Check GPU memory allocation in `/boot/config.txt`

### No Aircraft Shown

1. **Verify BaseStation feed**:
   ```bash
   nc localhost 30003
   ```
   You should see MSG lines streaming if dump1090/readsb is running

2. **Check ADS-B receiver**:
   ```bash
   rtl_test  # Test RTL-SDR dongle
   ```

### Slow Performance

1. **Check CPU governor**:
   ```bash
   cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor
   ```
   Should be `performance` or `ondemand`, not `powersave`

2. **Monitor resources**:
   ```bash
   htop  # Check CPU usage
   ```

## Build Artifacts

After cross-compilation, you'll have:

```
target/aarch64-unknown-linux-gnu/
├── release/
│   ├── airjedi-desktop          # Main executable (~15-25 MB)
│   ├── airjedi-desktop.d        # Dependency info
│   └── deps/                     # Compiled dependencies
└── debug/                        # Debug builds (if built)
```

## Additional Resources

- **Raspberry Pi 5 Documentation**: https://www.raspberrypi.com/documentation/computers/raspberry-pi-5.html
- **cross-rs Project**: https://github.com/cross-rs/cross
- **egui on embedded Linux**: https://github.com/emilk/egui/discussions

## Notes

- The macOS-specific GPS location code is automatically excluded from Linux builds
- Location will fall back to IP-based geolocation on Raspberry Pi
- All network features (tile loading, metadata fetching) work identically on Pi
- Trail rendering may be slower on Pi - adjust zoom levels for best performance
