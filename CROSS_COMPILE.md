# Building AirJedi Desktop for Raspberry Pi 5

This guide explains how to build AirJedi Desktop for Raspberry Pi 5, either natively on the Pi or cross-compiled from macOS/Linux.

## Target Platform

- **Device**: Raspberry Pi 5
- **Architecture**: aarch64 (64-bit ARM Cortex-A76)
- **OS**: Raspberry Pi OS (64-bit) or Ubuntu 64-bit
- **Target Triple**: `aarch64-unknown-linux-gnu`

## Native Compilation on Raspberry Pi 5 (Recommended)

**The simplest and most reliable method is to build directly on the Raspberry Pi 5.**

### 1. Install Rust on Raspberry Pi

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### 2. Install Build Dependencies

```bash
sudo apt update
sudo apt install -y \
    build-essential \
    libgl1-mesa-dev \
    libgl1-mesa-dri \
    libx11-dev \
    libxcursor-dev \
    libxrandr-dev \
    libxi-dev \
    libxinerama-dev \
    pkg-config
```

### 3. Clone and Build

```bash
git clone https://github.com/yourusername/airjedi-desktop.git
cd airjedi-desktop
cargo build --release
```

The binary will be created at:
```
target/release/airjedi-desktop
```

**Build Time**: Approximately 10-15 minutes on Raspberry Pi 5 (first build includes dependency compilation).

**Why Native Compilation?**
- ✅ **Simple setup**: No cross-compilation toolchains needed
- ✅ **100% compatibility**: Binary is guaranteed to work on the Pi
- ✅ **Optimized**: Built with `-C target-cpu=cortex-a76` for Raspberry Pi 5 (via `.cargo/config.toml`)
- ✅ **No deployment hassle**: Binary is already on the target device

---

## Cross-Compilation from Apple Silicon Macs

**If you're on Apple Silicon (M1/M2/M3/M4 Mac)**, you can cross-compile using native ARM64→ARM64 compilation:

### 1. Add Rust target

```bash
rustup target add aarch64-unknown-linux-gnu
```

### 2. Install aarch64 linker

```bash
brew install filosottile/musl-cross/musl-cross
```

### 3. Build for Raspberry Pi 5

```bash
cargo build --target aarch64-unknown-linux-gnu --release
```

The binary will be created at:
```
target/aarch64-unknown-linux-gnu/release/airjedi-desktop
```

**Why Apple Silicon Cross-Compilation Works Well:**
- ✅ **Native ARM64**: No emulation needed (ARM Mac → ARM Pi)
- ✅ **Fast builds**: 2-3 minutes vs 10-15 minutes on Pi
- ✅ **Same architecture**: Both use aarch64, simplifying the process
- ✅ **Optimized**: Built with `-C target-cpu=cortex-a76` for Raspberry Pi 5

**Note**: The `cross` tool doesn't work reliably on Apple Silicon because it requires QEMU emulation of x86_64 Docker containers.

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

## Cross-Compilation from Intel/x86_64 Macs (Using `cross`)

**For Intel Macs only.** This method uses Docker containers with pre-configured cross-compilation environments.

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

**Note**: The `cross` tool works well on Intel/x86_64 Macs but has issues on Apple Silicon due to Docker x86_64 emulation requirements.

## Deploying to Raspberry Pi 5 (Cross-Compilation Only)

**Skip this section if you built natively on the Raspberry Pi.**

### 1. Copy Binary to Pi

```bash
# Replace <pi-ip> with your Raspberry Pi's IP address or hostname

# If you cross-compiled from Apple Silicon Mac:
scp target/aarch64-unknown-linux-gnu/release/airjedi-desktop pi@<pi-ip>:~/

# If you used the cross tool (Intel Mac / x86_64):
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
