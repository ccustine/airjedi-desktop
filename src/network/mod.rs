//! Network connectivity and connection management.
//!
//! This module handles TCP connections to ADS-B feeds and manages multiple
//! concurrent server connections.

pub mod tcp_client;
pub mod connection_manager;

pub use connection_manager::ConnectionManager;

