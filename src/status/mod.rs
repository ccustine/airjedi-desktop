//! System status and diagnostics.
//!
//! This module provides system status tracking, connection status, and diagnostic logging.

pub mod system;

pub use system::{SystemStatus, SharedSystemStatus, ConnectionStatus, DiagnosticLevel, ServerStatus};

