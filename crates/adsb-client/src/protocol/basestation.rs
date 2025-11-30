// Copyright 2025 Chris Custine
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! BaseStation/SBS-1 protocol parser.
//!
//! Parses the CSV-based BaseStation protocol format commonly used by
//! dump1090 and similar ADS-B decoders.
//!
//! Message format:
//! ```text
//! MSG,<type>,<session>,<aircraft>,<icao>,<flight>,<date>,<time>,<date>,<time>,<fields...>
//! ```

use super::{AircraftMessage, ParseError, Protocol};

/// Parser for BaseStation/SBS-1 protocol messages.
#[derive(Debug, Default)]
pub struct BaseStationParser;

impl BaseStationParser {
    /// Create a new BaseStation parser.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Protocol for BaseStationParser {
    type Message = AircraftMessage;
    type Error = ParseError;

    fn parse(&mut self, input: &[u8]) -> Result<Option<AircraftMessage>, ParseError> {
        let line = std::str::from_utf8(input)
            .map_err(|_| ParseError::InvalidFormat("invalid UTF-8".to_string()))?;

        parse_basestation_line(line)
    }
}

/// Parse a single BaseStation message line.
fn parse_basestation_line(line: &str) -> Result<Option<AircraftMessage>, ParseError> {
    let parts: Vec<&str> = line.split(',').collect();

    if parts.is_empty() {
        return Ok(None);
    }

    let msg_type = parts[0];

    // We only handle MSG type messages
    if msg_type != "MSG" {
        return Ok(None);
    }

    // We need at least the ICAO field (index 4)
    if parts.len() < 5 {
        return Ok(None);
    }

    let icao = parts[4].trim();
    if icao.is_empty() {
        return Ok(None);
    }

    // Need at least 11 fields to determine transmission type
    if parts.len() < 11 {
        return Ok(None);
    }

    let transmission_type = parts[1];

    match transmission_type {
        "1" => {
            // Aircraft identification (callsign)
            if parts.len() > 10 && !parts[10].is_empty() {
                return Ok(Some(AircraftMessage::Identification {
                    icao: icao.to_string(),
                    callsign: parts[10].trim().to_string(),
                }));
            }
            Ok(None)
        }
        "3" => {
            // Airborne position
            if parts.len() > 15 {
                let altitude = if !parts[11].is_empty() {
                    parts[11].parse::<i32>().ok()
                } else {
                    None
                };

                if !parts[14].is_empty() && !parts[15].is_empty() {
                    let lat = parts[14].parse::<f64>().map_err(|_| ParseError::InvalidValue {
                        field: "latitude",
                        value: parts[14].to_string(),
                    })?;
                    let lon = parts[15].parse::<f64>().map_err(|_| ParseError::InvalidValue {
                        field: "longitude",
                        value: parts[15].to_string(),
                    })?;

                    return Ok(Some(AircraftMessage::Position {
                        icao: icao.to_string(),
                        latitude: lat,
                        longitude: lon,
                        altitude,
                    }));
                }
            }
            Ok(None)
        }
        "4" => {
            // Airborne velocity
            if parts.len() > 13 {
                let speed = if !parts[12].is_empty() {
                    parts[12].parse::<f64>().ok()
                } else {
                    None
                };
                let track = if !parts[13].is_empty() {
                    parts[13].parse::<f64>().ok()
                } else {
                    None
                };
                let vertical_rate = if parts.len() > 16 && !parts[16].is_empty() {
                    parts[16].parse::<i32>().ok()
                } else {
                    None
                };

                if let (Some(speed), Some(track)) = (speed, track) {
                    return Ok(Some(AircraftMessage::Velocity {
                        icao: icao.to_string(),
                        speed,
                        track,
                        vertical_rate,
                    }));
                }
            }
            Ok(None)
        }
        "5" | "6" | "7" | "8" => {
            // Surveillance altitude / position / air-to-air / all call reply
            // These all provide altitude data at minimum
            if parts.len() > 11 && !parts[11].is_empty() {
                if let Ok(alt) = parts[11].parse::<i32>() {
                    // For MSG type 6, also check for position data
                    if transmission_type == "6" && parts.len() > 15 {
                        if !parts[14].is_empty() && !parts[15].is_empty() {
                            if let (Ok(lat), Ok(lon)) =
                                (parts[14].parse::<f64>(), parts[15].parse::<f64>())
                            {
                                return Ok(Some(AircraftMessage::Position {
                                    icao: icao.to_string(),
                                    latitude: lat,
                                    longitude: lon,
                                    altitude: Some(alt),
                                }));
                            }
                        }
                    }

                    return Ok(Some(AircraftMessage::Altitude {
                        icao: icao.to_string(),
                        altitude: alt,
                    }));
                }
            }
            Ok(None)
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_identification() {
        let mut parser = BaseStationParser::new();
        let line = b"MSG,1,1,1,A1B2C3,1,2024/01/01,12:00:00.000,2024/01/01,12:00:00.000,UAL123";
        let result = parser.parse(line).unwrap();
        assert!(matches!(
            result,
            Some(AircraftMessage::Identification { icao, callsign })
            if icao == "A1B2C3" && callsign == "UAL123"
        ));
    }

    #[test]
    fn test_parse_position() {
        let mut parser = BaseStationParser::new();
        let line = b"MSG,3,1,1,A1B2C3,1,2024/01/01,12:00:00.000,2024/01/01,12:00:00.000,,35000,,,33.9425,-118.4081,";
        let result = parser.parse(line).unwrap();
        assert!(matches!(
            result,
            Some(AircraftMessage::Position { icao, latitude, longitude, altitude })
            if icao == "A1B2C3"
                && (latitude - 33.9425).abs() < 0.0001
                && (longitude - (-118.4081)).abs() < 0.0001
                && altitude == Some(35000)
        ));
    }

    #[test]
    fn test_parse_velocity() {
        let mut parser = BaseStationParser::new();
        let line = b"MSG,4,1,1,A1B2C3,1,2024/01/01,12:00:00.000,2024/01/01,12:00:00.000,,,450,270,,,1500";
        let result = parser.parse(line).unwrap();
        assert!(matches!(
            result,
            Some(AircraftMessage::Velocity { icao, speed, track, vertical_rate })
            if icao == "A1B2C3"
                && (speed - 450.0).abs() < 0.01
                && (track - 270.0).abs() < 0.01
                && vertical_rate == Some(1500)
        ));
    }

    #[test]
    fn test_parse_altitude() {
        let mut parser = BaseStationParser::new();
        let line = b"MSG,5,1,1,A1B2C3,1,2024/01/01,12:00:00.000,2024/01/01,12:00:00.000,,30000";
        let result = parser.parse(line).unwrap();
        assert!(matches!(
            result,
            Some(AircraftMessage::Altitude { icao, altitude })
            if icao == "A1B2C3" && altitude == 30000
        ));
    }

    #[test]
    fn test_parse_empty_line() {
        let mut parser = BaseStationParser::new();
        let result = parser.parse(b"").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_non_msg_type() {
        let mut parser = BaseStationParser::new();
        let result = parser.parse(b"STA,1,1,1,A1B2C3").unwrap();
        assert!(result.is_none());
    }
}
