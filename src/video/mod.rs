//! Video streaming and playback.
//!
//! This module manages video player windows, stream protocols, and resource management.

pub mod manager;
pub mod player;
pub mod protocol;

pub use manager::VideoManager;

