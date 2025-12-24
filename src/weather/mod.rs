//! Weather overlay tile management.
//!
//! This module provides weather tile fetching from OpenWeatherMap
//! with support for precipitation, cloud, and wind layers.

pub mod openweathermap;

pub use openweathermap::{WeatherLayer, OpenWeatherMapSource, WeatherTiles};
