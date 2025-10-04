use std::collections::HashMap;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct Aircraft {
    pub icao: u32,
    pub callsign: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub altitude: Option<i32>,
    pub track: Option<f64>,
    pub velocity: Option<f64>,
    pub vertical_rate: Option<i32>,
    pub last_seen: DateTime<Utc>,
}

impl Aircraft {
    pub fn new(icao: u32) -> Self {
        Self {
            icao,
            callsign: None,
            latitude: None,
            longitude: None,
            altitude: None,
            track: None,
            velocity: None,
            vertical_rate: None,
            last_seen: Utc::now(),
        }
    }

    pub fn update_position(&mut self, lat: f64, lon: f64, alt: i32) {
        self.latitude = Some(lat);
        self.longitude = Some(lon);
        self.altitude = Some(alt);
        self.last_seen = Utc::now();
    }

    pub fn update_velocity(&mut self, track: f64, velocity: f64) {
        self.track = Some(track);
        self.velocity = Some(velocity);
        self.last_seen = Utc::now();
    }
}

pub struct AircraftTracker {
    aircraft: HashMap<u32, Aircraft>,
}

impl AircraftTracker {
    pub fn new() -> Self {
        Self {
            aircraft: HashMap::new(),
        }
    }

    pub fn update_aircraft(&mut self, icao: u32) -> &mut Aircraft {
        self.aircraft.entry(icao).or_insert_with(|| Aircraft::new(icao))
    }

    pub fn get_aircraft(&self) -> Vec<&Aircraft> {
        self.aircraft.values().collect()
    }

    pub fn cleanup_old(&mut self, max_age_seconds: i64) {
        let now = Utc::now();
        self.aircraft.retain(|_, aircraft| {
            (now - aircraft.last_seen).num_seconds() < max_age_seconds
        });
    }
}

// BEAST protocol decoder
pub struct BeastDecoder {
    buffer: Vec<u8>,
}

impl BeastDecoder {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
        }
    }

    pub fn decode(&mut self, data: &[u8], tracker: &mut AircraftTracker) {
        self.buffer.extend_from_slice(data);

        while !self.buffer.is_empty() {
            // BEAST format: <esc> "1" <6 bytes timestamp> <1 byte signal> <14 bytes message>
            // or Mode-S short: <esc> "2" <6 bytes timestamp> <1 byte signal> <7 bytes message>
            // or Mode-S long: <esc> "3" <6 bytes timestamp> <1 byte signal> <14 bytes message>

            if self.buffer[0] != 0x1A {
                // Not a BEAST escape, skip byte
                self.buffer.remove(0);
                continue;
            }

            if self.buffer.len() < 2 {
                break; // Need more data
            }

            let msg_type = self.buffer[1];
            let (msg_len, payload_len) = match msg_type {
                b'1' | b'3' => (23, 14), // Mode-S long (includes ADS-B)
                b'2' => (16, 7),          // Mode-S short
                _ => {
                    self.buffer.remove(0);
                    continue;
                }
            };

            if self.buffer.len() < msg_len {
                break; // Need more data
            }

            // Extract message (skip <esc>, type, 6 bytes timestamp, 1 byte signal)
            let msg_start = 9;
            let msg = &self.buffer[msg_start..msg_start + payload_len];

            // Decode Mode-S message
            self.decode_mode_s(msg, tracker);

            // Remove processed message
            self.buffer.drain(0..msg_len);
        }
    }

    fn decode_mode_s(&self, msg: &[u8], tracker: &mut AircraftTracker) {
        if msg.len() < 7 {
            return;
        }

        let df = (msg[0] >> 3) & 0x1F; // Downlink Format

        // DF 17/18 are ADS-B messages
        if df == 17 || df == 18 {
            // Extract ICAO address (3 bytes)
            let icao = ((msg[1] as u32) << 16) | ((msg[2] as u32) << 8) | (msg[3] as u32);

            if msg.len() < 14 {
                return;
            }

            // Type Code
            let tc = (msg[4] >> 3) & 0x1F;

            match tc {
                1..=4 => {
                    // Aircraft identification
                    let callsign = self.decode_callsign(&msg[5..11]);
                    let aircraft = tracker.update_aircraft(icao);
                    aircraft.callsign = Some(callsign);
                }
                9..=18 => {
                    // Airborne position
                    if let Some((lat, lon, alt)) = self.decode_airborne_position(msg, tc) {
                        let aircraft = tracker.update_aircraft(icao);
                        aircraft.update_position(lat, lon, alt);
                    }
                }
                19 => {
                    // Airborne velocity
                    if let Some((track, velocity)) = self.decode_velocity(&msg[5..12]) {
                        let aircraft = tracker.update_aircraft(icao);
                        aircraft.update_velocity(track, velocity);
                    }
                }
                _ => {}
            }
        }
    }

    fn decode_callsign(&self, data: &[u8]) -> String {
        let charset = "?ABCDEFGHIJKLMNOPQRSTUVWXYZ????? ???????????????0123456789??????";
        let mut callsign = String::new();

        // Extract 6-bit characters
        let bits: u64 = data.iter().take(6).fold(0u64, |acc, &b| (acc << 8) | b as u64);

        for i in 0..8 {
            let char_idx = ((bits >> (42 - i * 6)) & 0x3F) as usize;
            if char_idx < charset.len() {
                let c = charset.chars().nth(char_idx).unwrap_or('?');
                if c != ' ' {
                    callsign.push(c);
                }
            }
        }

        callsign.trim().to_string()
    }

    fn decode_airborne_position(&self, msg: &[u8], tc: u8) -> Option<(f64, f64, i32)> {
        // Simplified CPR decoding - in production, you'd need proper CPR decoding with even/odd frames
        let alt_encoded = ((msg[5] as u16) << 4) | ((msg[6] as u16) >> 4);

        let altitude = if tc >= 9 && tc <= 18 {
            // Altitude encoding
            let q_bit = (alt_encoded >> 4) & 1;
            if q_bit == 1 {
                // 25ft resolution
                let n = ((alt_encoded & 0x0F) | ((alt_encoded & 0x0FF0) >> 1)) as i32;
                Some((n * 25) - 1000)
            } else {
                None
            }
        } else {
            None
        };

        // CPR latitude/longitude (17 bits each)
        let lat_cpr = (((msg[6] as u32) & 0x03) << 15) | ((msg[7] as u32) << 7) | ((msg[8] as u32) >> 1);
        let lon_cpr = (((msg[8] as u32) & 0x01) << 16) | ((msg[9] as u32) << 8) | (msg[10] as u32);

        // Simplified: assume surface reference position (this is not accurate, needs proper CPR)
        let lat = (lat_cpr as f64 / 131072.0) * 90.0;
        let lon = (lon_cpr as f64 / 131072.0) * 180.0;

        if let Some(alt) = altitude {
            Some((lat, lon, alt))
        } else {
            None
        }
    }

    fn decode_velocity(&self, data: &[u8]) -> Option<(f64, f64)> {
        let subtype = (data[0] >> 5) & 0x07;

        if subtype == 1 || subtype == 2 {
            // Ground speed
            let ew_sign = (data[0] >> 2) & 1;
            let ew_vel = ((data[0] as u16 & 0x03) << 8) | (data[1] as u16);
            let ns_sign = (data[2] >> 7) & 1;
            let ns_vel = (((data[2] as u16) & 0x7F) << 3) | ((data[3] as u16) >> 5);

            let ew = if ew_sign == 0 { ew_vel as f64 - 1.0 } else { 1.0 - ew_vel as f64 };
            let ns = if ns_sign == 0 { ns_vel as f64 - 1.0 } else { 1.0 - ns_vel as f64 };

            let velocity = (ew * ew + ns * ns).sqrt();
            let track = ns.atan2(ew).to_degrees();
            let track = if track < 0.0 { track + 360.0 } else { track };

            Some((track, velocity))
        } else {
            None
        }
    }
}
