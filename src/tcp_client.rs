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

//! Async TCP client for BaseStation ADS-B feeds.
//!
//! Handles connection to BaseStation protocol TCP feeds with automatic
//! reconnection, hot-reload of server addresses, and graceful shutdown.
//! Implements periodic cleanup of stale aircraft data.

use log::{info, warn, error};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{sleep, Duration};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::basestation::AircraftTracker;
use crate::status::{SharedSystemStatus, ConnectionStatus};

pub async fn connect_adsb_feed(
    server_id: String,
    server_name: String,
    mut address_rx: watch::Receiver<String>,
    tracker: Arc<Mutex<AircraftTracker>>,
    status: SharedSystemStatus,
    cancel_token: CancellationToken,
) {
    loop {
        // Check for cancellation
        if cancel_token.is_cancelled() {
            info!("[{}] Connection cancelled", server_name);
            return;
        }

        // Get current server address from the watch channel
        let current_address = address_rx.borrow_and_update().clone();

        // Set status to connecting
        status.lock().unwrap().update_server_status(&server_id, ConnectionStatus::Connecting);

        // Clone for use in the async block
        let address = current_address.clone();

        match connect_and_process(
            &server_id,
            &server_name,
            &address,
            tracker.clone(),
            status.clone(),
            address_rx.clone(),
            cancel_token.clone(),
        ).await {
            Ok(reconnect_reason) => {
                match reconnect_reason {
                    ReconnectReason::ServerAddressChanged => {
                        info!("[{}] Server address changed, reconnecting immediately...", server_name);
                        continue; // Skip the 5-second delay
                    }
                    ReconnectReason::ConnectionClosed => {
                        info!("[{}] Connection closed normally", server_name);
                        status.lock().unwrap().update_server_status(&server_id, ConnectionStatus::Disconnected);
                    }
                    ReconnectReason::Cancelled => {
                        info!("[{}] Connection cancelled", server_name);
                        return; // Exit completely
                    }
                }
            }
            Err(e) => {
                error!("[{}] Connection error: {}", server_name, e);
                status.lock().unwrap().update_server_error(&server_id, e.to_string());
            }
        }

        warn!("Reconnecting in 5 seconds...");
        sleep(Duration::from_secs(5)).await;
    }
}

enum ReconnectReason {
    ServerAddressChanged,
    ConnectionClosed,
    Cancelled,
}

const CLEANUP_INTERVAL_MESSAGES: u32 = 100;
const AIRCRAFT_TIMEOUT_SECONDS: i64 = 180; // 3 minutes

async fn connect_and_process(
    server_id: &str,
    server_name: &str,
    address: &str,
    tracker: Arc<Mutex<AircraftTracker>>,
    status: SharedSystemStatus,
    mut address_rx: watch::Receiver<String>,
    cancel_token: CancellationToken,
) -> Result<ReconnectReason, Box<dyn std::error::Error>> {
    info!("[{}] Connecting to {}...", server_name, address);

    let stream = TcpStream::connect(address).await?;
    info!("[{}] Connected to BaseStation feed", server_name);

    // Mark connection as successful
    status.lock().unwrap().update_server_status(server_id, ConnectionStatus::Connected);

    let reader = BufReader::new(stream);
    let mut lines = reader.lines();
    let mut cleanup_counter: u32 = 0;

    while let Some(line) = lines.next_line().await? {
        // Check for cancellation
        if cancel_token.is_cancelled() {
            info!("[{}] Connection cancelled during message processing", server_name);
            return Ok(ReconnectReason::Cancelled);
        }

        // Parse the BaseStation message - scope lock to drop before next await
        {
            let mut tracker_lock = tracker.lock()
                .expect("Aircraft tracker mutex poisoned");
            tracker_lock.parse_basestation_message(&line);
        }

        // Increment message counter for this server
        status.lock().unwrap().increment_server_message_count(server_id);

        // Cleanup old aircraft every N messages
        cleanup_counter = cleanup_counter.saturating_add(1);
        if cleanup_counter >= CLEANUP_INTERVAL_MESSAGES {
            let mut tracker_lock = tracker.lock()
                .expect("Aircraft tracker mutex poisoned");
            tracker_lock.cleanup_old(AIRCRAFT_TIMEOUT_SECONDS);
            cleanup_counter = 0;

            // Check if server address has changed (piggyback on cleanup interval)
            if address_rx.has_changed().unwrap_or(false) {
                let new_address = address_rx.borrow_and_update().clone();
                if new_address != address {
                    info!("Server address changed from {} to {}, reconnecting...", address, new_address);
                    return Ok(ReconnectReason::ServerAddressChanged);
                }
            }
        }
    }

    info!("Connection closed by server");
    Ok(ReconnectReason::ConnectionClosed)
}
