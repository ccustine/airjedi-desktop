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

use log::{info, warn, error};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::time::{sleep, Duration};
use tokio::sync::watch;

use crate::basestation::AircraftTracker;
use crate::status::{SharedSystemStatus, ConnectionStatus};

pub async fn connect_adsb_feed(
    mut address_rx: watch::Receiver<String>,
    tracker: Arc<Mutex<AircraftTracker>>,
    status: SharedSystemStatus
) {
    loop {
        // Get current server address from the watch channel
        let current_address = address_rx.borrow_and_update().clone();

        // Set current connection address in status
        status.lock().unwrap().connection_address = current_address.clone();

        // Set status to connecting
        status.lock().unwrap().set_connection_status(ConnectionStatus::Connecting);

        // Clone for use in the async block
        let address = current_address.clone();

        match connect_and_process(&address, tracker.clone(), status.clone(), address_rx.clone()).await {
            Ok(reconnect_reason) => {
                match reconnect_reason {
                    ReconnectReason::ServerAddressChanged => {
                        info!("Server address changed, reconnecting immediately...");
                        continue; // Skip the 5-second delay
                    }
                    ReconnectReason::ConnectionClosed => {
                        info!("ADSB connection closed normally");
                        status.lock().unwrap().set_connection_status(ConnectionStatus::Disconnected);
                    }
                }
            }
            Err(e) => {
                error!("ADSB connection error: {}", e);
                status.lock().unwrap().set_connection_error(e.to_string());
            }
        }

        warn!("Reconnecting in 5 seconds...");
        sleep(Duration::from_secs(5)).await;
    }
}

enum ReconnectReason {
    ServerAddressChanged,
    ConnectionClosed,
}

const CLEANUP_INTERVAL_MESSAGES: u32 = 100;
const AIRCRAFT_TIMEOUT_SECONDS: i64 = 180; // 3 minutes

async fn connect_and_process(
    address: &str,
    tracker: Arc<Mutex<AircraftTracker>>,
    status: SharedSystemStatus,
    mut address_rx: watch::Receiver<String>,
) -> Result<ReconnectReason, Box<dyn std::error::Error>> {
    info!("Connecting to {}...", address);

    let stream = TcpStream::connect(address).await?;
    info!("Connected to BaseStation feed");

    // Mark connection as successful
    status.lock().unwrap().set_connection_status(ConnectionStatus::Connected);

    let reader = BufReader::new(stream);
    let mut lines = reader.lines();
    let mut cleanup_counter: u32 = 0;

    while let Some(line) = lines.next_line().await? {
        // Parse the BaseStation message - scope lock to drop before next await
        {
            let mut tracker_lock = tracker.lock()
                .expect("Aircraft tracker mutex poisoned");
            tracker_lock.parse_basestation_message(&line);
        }

        // Increment message counter
        status.lock().unwrap().increment_message_count();

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
