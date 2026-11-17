// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 Artem Wittermann

pub mod sysfs;

use cosmic::iced::Subscription;
use cosmic::iced::futures::SinkExt;
use cosmic::iced::futures::channel::mpsc::Sender;
use cosmic::iced::stream;

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
    /// External power is present.
    ///
    /// Unreliable for device-scoped supplies: some dongles report `online=1`
    /// while simultaneously reporting `status=Discharging`. Prefer [`state`].
    ///
    /// [`state`]: WirelessDevice::state
    pub online: bool,
    /// The device reports charging wirelessly.
    pub wireless: bool,
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
    Unknown,
}

impl ChargeState {
    pub fn from_sysfs(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "charging" => Self::Charging,
            "discharging" => Self::Discharging,
            "full" => Self::Full,
            "not charging" => Self::NotCharging,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Charging => "Charging",
            Self::Discharging => "Discharging",
            Self::Full => "Full",
            Self::NotCharging => "Not charging",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub enum BackendEvent {
    Snapshot(Vec<WirelessDevice>),
}

/// Emits a snapshot on start and again whenever the kernel reports a change
/// to any power supply.
pub fn subscription() -> Subscription<BackendEvent> {
    Subscription::run_with("wireless-backend", |_| {
        stream::channel(16, |mut output: Sender<BackendEvent>| async move {
            let _ = output.send(BackendEvent::Snapshot(sysfs::scan())).await;

            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

            // The monitor is built inside the thread because udev::MonitorSocket
            // is not Send. Only the channel sender crosses the boundary.
            std::thread::spawn(move || {
                let socket = udev::MonitorBuilder::new()
                    .and_then(|builder| builder.match_subsystem("power_supply"))
                    .and_then(|builder| builder.listen());

                match socket {
                    Ok(socket) => {
                        tracing::debug!("udev monitor listening on power_supply");
                        for _event in socket.iter() {
                            if tx.send(()).is_err() {
                                break;
                            }
                        }
                    }
                    Err(err) => tracing::warn!(?err, "failed to start udev monitor"),
                }
            });

            while rx.recv().await.is_some() {
                tracing::debug!("power supply changed, rescanning");
                let _ = output.send(BackendEvent::Snapshot(sysfs::scan())).await;
            }
        })
    })
}
