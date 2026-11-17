// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 Artem Wittermann

pub mod sysfs;

use std::time::Duration;

use cosmic::iced::Subscription;
use cosmic::iced::futures::SinkExt;
use cosmic::iced::futures::channel::mpsc::Sender;
use cosmic::iced::stream;
use rustix::event::{PollFd, PollFlags, poll};
use rustix::io::Errno;

/// A wireless device that reports a battery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WirelessDevice {
    /// Stable identifier, unique per device.
    pub id: String,
    /// Human readable name.
    pub name: String,
    pub kind: DeviceKind,
    /// Charge percentage, when the device reports one.
    pub capacity: Option<u8>,
    pub state: ChargeState,
    /// The wireless link is up.
    ///
    /// For device-scoped supplies the kernel reuses `online` to report whether
    /// the peripheral is reachable, not whether external power is present. A
    /// mouse that is switched off or out of range reports `online=0` while
    /// leaving its last known capacity in place.
    pub connected: bool,
    /// The device reports charging wirelessly.
    pub wireless: bool,
}

impl WirelessDevice {
    /// Label describing the device's current state.
    ///
    /// A device that is out of range keeps reporting its last known capacity,
    /// so the link state takes precedence over the charge state.
    pub fn status_label(&self) -> &'static str {
        if self.connected {
            self.state.label()
        } else {
            "Disconnected"
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind {
    Mouse,
    Keyboard,
    Headset,
    Gamepad,
    Other,
}

impl DeviceKind {
    pub fn from_sysfs(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "mouse" => Self::Mouse,
            "keyboard" => Self::Keyboard,
            "headset" | "headphones" | "audio" => Self::Headset,
            "joystick" | "gamepad" | "gaming-input" => Self::Gamepad,
            _ => Self::Other,
        }
    }

    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Mouse => "input-mouse-symbolic",
            Self::Keyboard => "input-keyboard-symbolic",
            Self::Headset => "audio-headphones-symbolic",
            Self::Gamepad => "input-gaming-symbolic",
            Self::Other => "network-wireless-symbolic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeState {
    Charging,
    Discharging,
    Full,
    NotCharging,
    Empty,
    Unknown,
}

impl ChargeState {
    pub fn from_sysfs(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "charging" => Self::Charging,
            "discharging" => Self::Discharging,
            "full" => Self::Full,
            "not charging" => Self::NotCharging,
            "empty" => Self::Empty,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Charging => "Charging",
            Self::Discharging => "Discharging",
            Self::Full => "Full",
            Self::NotCharging => "Not charging",
            Self::Empty => "Empty",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub enum BackendEvent {
    Snapshot {
        devices: Vec<WirelessDevice>,
        /// Local time the scan finished, formatted for display.
        scanned_at: String,
    },
}

/// Reads the current state and stamps it with the time it was taken.
fn snapshot() -> BackendEvent {
    BackendEvent::Snapshot {
        devices: sysfs::scan(),
        scanned_at: jiff::Zoned::now().strftime("%H:%M:%S").to_string(),
    }
}

/// How often the power supplies are re-read.
///
/// Polling is the only way to observe this hardware. The `kysona` driver
/// exposes `capacity` and `status` as attributes computed on read: it never
/// calls `power_supply_changed()`, so the kernel emits no uevent, and because
/// nothing is written to sysfs there is no inotify notification either. A
/// scan is a handful of small file reads, so a short interval is cheap.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Emits a snapshot on start and again whenever the devices change.
pub fn subscription() -> Subscription<BackendEvent> {
    Subscription::run_with("wireless-backend", |_| {
        stream::channel(16, |mut output: Sender<BackendEvent>| async move {
            let _ = output.send(snapshot()).await;

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

            // The monitor is built inside the thread because udev::MonitorSocket
            // is not Send. Only the channel sender crosses the boundary.
            std::thread::spawn(move || {
                let socket = match udev::MonitorBuilder::new()
                    .and_then(|builder| builder.match_subsystem("power_supply"))
                    .and_then(|builder| builder.listen())
                {
                    Ok(socket) => socket,
                    Err(err) => {
                        tracing::warn!(?err, "failed to start udev monitor");
                        return;
                    }
                };

                tracing::debug!("udev monitor listening on power_supply");

                loop {
                    let mut fds = [PollFd::new(&socket, PollFlags::IN)];

                    match poll(&mut fds, -1) {
                        Ok(0) => continue,
                        Ok(_) => {}
                        Err(Errno::INTR) => continue,
                        Err(err) => {
                            tracing::warn!(?err, "udev poll failed");
                            break;
                        }
                    }

                    // The monitor socket is non-blocking, so every queued event
                    // has to be drained before waiting again.
                    let mut received = false;
                    for _event in socket.iter() {
                        received = true;
                    }

                    if received && tx.send(()).is_err() {
                        break;
                    }
                }
            });

            // The monitor still earns its keep for genuine hotplug, such as the
            // receiver being plugged in or pulled out.
            let mut interval = tokio::time::interval(POLL_INTERVAL);
            interval.tick().await;

            loop {
                tokio::select! {
                    Some(()) = rx.recv() => {
                        tracing::debug!("power supply changed, rescanning");
                    }
                    _ = interval.tick() => {}
                }

                let _ = output.send(snapshot()).await;
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_stamps_the_scan_time() {
        let BackendEvent::Snapshot { scanned_at, .. } = snapshot();

        assert_eq!(scanned_at.len(), 8, "expected HH:MM:SS, got {scanned_at}");
        assert_eq!(scanned_at.as_bytes()[2], b':');
        assert_eq!(scanned_at.as_bytes()[5], b':');
    }
}
