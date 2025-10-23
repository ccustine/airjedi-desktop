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

/// Web Mercator projection utilities
pub struct WebMercator;

impl WebMercator {
    /// Convert latitude to Web Mercator Y coordinate (0.0 to 1.0)
    pub fn lat_to_y(lat: f64, zoom: u8) -> f64 {
        let lat_rad = lat.to_radians();
        let n = 2_f64.powi(zoom as i32);
        let y = (1.0 - (lat_rad.tan() + 1.0 / lat_rad.cos()).ln() / std::f64::consts::PI) / 2.0;
        y * n
    }

    /// Convert longitude to Web Mercator X coordinate (0.0 to 1.0)
    pub fn lon_to_x(lon: f64, zoom: u8) -> f64 {
        let n = 2_f64.powi(zoom as i32);
        ((lon + 180.0) / 360.0) * n
    }

    /// Convert tile coordinates back to latitude
    #[allow(dead_code)]
    pub fn tile_to_lat(y: f64, zoom: u8) -> f64 {
        let n = 2_f64.powi(zoom as i32);
        let lat_rad = ((std::f64::consts::PI * (1.0 - 2.0 * y / n)).sinh()).atan();
        lat_rad.to_degrees()
    }

    /// Convert tile coordinates back to longitude
    #[allow(dead_code)]
    pub fn tile_to_lon(x: f64, zoom: u8) -> f64 {
        let n = 2_f64.powi(zoom as i32);
        x / n * 360.0 - 180.0
    }
}
