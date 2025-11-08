//! SDR (Software Defined Radio) module for IQ data processing and FFT visualization.
//!
//! This module provides FutureSDR-based signal processing capabilities including:
//! - IQ data ingestion from files or hardware
//! - FFT computation and spectrum analysis
//! - Waterfall data generation for visualization

pub mod waterfall_sink;
pub mod iq_processor;
pub mod rtlsdr_source;
pub mod complex_to_mag;
pub mod wav_source;

pub use iq_processor::{IqProcessor, ProcessorConfig, SourceType};
pub use rtlsdr_source::{list_devices, DeviceInfo, GainMode};
