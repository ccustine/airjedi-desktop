# Aviation Data Files

This directory contains aviation data files (airports, runways, navaids) used for map overlays.

## Automatic Download

**The application will automatically download these files on first startup if they don't exist.**

No manual download is required! The app will download the data in the background and display the overlays once loaded.

## Data Source: OurAirports

The following CSV files are automatically downloaded from [OurAirports](https://ourairports.com/data/):

1. **airports.csv** - Airport locations and information
   - Direct download: https://davidmegginson.github.io/ourairports-data/airports.csv

2. **runways.csv** - Runway data for all airports
   - Direct download: https://davidmegginson.github.io/ourairports-data/runways.csv

3. **navaids.csv** - Navigational aids (VOR, NDB, DME, etc.)
   - Direct download: https://davidmegginson.github.io/ourairports-data/navaids.csv

## Quick Download (macOS/Linux)

```bash
cd data
curl -O https://davidmegginson.github.io/ourairports-data/airports.csv
curl -O https://davidmegginson.github.io/ourairports-data/runways.csv
curl -O https://davidmegginson.github.io/ourairports-data/navaids.csv
```

## Quick Download (Windows PowerShell)

```powershell
cd data
Invoke-WebRequest -Uri "https://davidmegginson.github.io/ourairports-data/airports.csv" -OutFile "airports.csv"
Invoke-WebRequest -Uri "https://davidmegginson.github.io/ourairports-data/runways.csv" -OutFile "runways.csv"
Invoke-WebRequest -Uri "https://davidmegginson.github.io/ourairports-data/navaids.csv" -OutFile "navaids.csv"
```

## File Structure

After downloading, your data directory should contain:
```
data/
├── README.md
├── airports.csv
├── runways.csv
└── navaids.csv
```

## Data License

OurAirports data is released to the Public Domain (no license required).
See: https://ourairports.com/about.html#credits

## Alternative Data Sources

If you prefer official FAA data:

### FAA NASR (National Airspace System Resources)
- Download: https://www.faa.gov/air_traffic/flight_info/aeronav/aero_data/NASR_Subscription/
- Format: Fixed-width text files (requires custom parser)
- Updated: Every 28 days
- Coverage: US airports only

### OpenAIP
- Download: https://www.openaip.net/
- Format: GeoJSON or KML
- Coverage: Worldwide
- Note: Requires free account for downloads

## Notes

- The application will run without these files, but map overlays will not be available
- Files are loaded at startup - restart the application after downloading
- File size: ~50MB total (uncompressed CSV)
- Updates: OurAirports data is updated regularly - re-download periodically for latest data
