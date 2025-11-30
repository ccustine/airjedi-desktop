//! Aircraft tracking and data management.
//!
//! This module provides aircraft tracking, ADS-B message parsing, aircraft databases,
//! metadata services, and type information.

pub mod tracker;
pub mod adsb;
pub mod database;
pub mod metadata;
pub mod types;

pub use tracker::{Aircraft, AircraftTracker};
pub use adsb_client::tracker::PositionPoint;
pub use database::AircraftDatabase;
pub use metadata::MetadataService;
pub use types::AircraftTypeDatabase;

