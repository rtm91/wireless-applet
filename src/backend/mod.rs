// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 Artem Wittermann

pub mod bluez;
pub mod sysfs;

use std::cmp::Ordering;
use std::time::Duration;

use cosmic::iced::Subscription;
use cosmic::iced::futures::SinkExt;
use cosmic::iced::futures::channel::mpsc::Sender;
use cosmic::iced::stream;
use rustix::event::{PollFd, PollFlags, poll};
use rustix::io::Errno;

use crate::config::ICON;
use crate::fl;

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
    /// so the link state takes precedence over the charge state. BlueZ reports
    /// a level but no charge state, so a reachable Bluetooth device reads as
    /// connected rather than unknown.
    pub fn status_label(&self) -> String {
        if !self.connected {
            fl!("state-disconnected")
        } else if self.state == ChargeState::Unknown {
            fl!("state-connected")
        } else {
            self.state.label()
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

    /// Maps a BlueZ `Device1.Icon` name to a device kind.
    ///
    /// BlueZ reports freedesktop icon names, without the `-symbolic` suffix
    /// the theme appends.
    pub fn from_bluez_icon(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "input-mouse" => Self::Mouse,
            "input-keyboard" => Self::Keyboard,
            "audio-headphones" | "audio-headset" | "audio-card" => Self::Headset,
            "input-gaming" => Self::Gamepad,
            _ => Self::Other,
        }
    }

    pub fn icon_name(self) -> &'static str {
        match self {
            Self::Mouse => "input-mouse-symbolic",
            Self::Keyboard => "input-keyboard-symbolic",
            Self::Headset => "audio-headphones-symbolic",
            Self::Gamepad => "input-gaming-symbolic",
            Self::Other => ICON,
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

    pub fn label(self) -> String {
        match self {
            Self::Charging => fl!("state-charging"),
            Self::Discharging => fl!("state-discharging"),
            Self::Full => fl!("state-full"),
            Self::NotCharging => fl!("state-not-charging"),
            Self::Empty => fl!("state-empty"),
            Self::Unknown => fl!("state-unknown"),
        }
    }
}

#[derive(Debug, Clone)]
pub enum BackendEvent {
    Snapshot(Vec<WirelessDevice>),
}

/// How often the power supplies are re-read.
///
/// Polling is the only way to observe this hardware. The `kysona` driver
/// exposes `capacity` and `status` as attributes computed on read: it never
/// calls `power_supply_changed()`, so the kernel emits no uevent, and because
/// nothing is written to sysfs there is no inotify notification either. A
/// scan is a handful of small file reads, so a short interval is cheap.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Reachable devices first, so a peripheral that is switched off does not push
/// the live ones down the list.
fn by_reachability(a: &WirelessDevice, b: &WirelessDevice) -> Ordering {
    b.connected
        .cmp(&a.connected)
        .then_with(|| a.name.cmp(&b.name))
}

/// Combines the two sources into one list.
///
/// A Bluetooth peripheral can appear in both: the kernel exposes a
/// `power_supply` for HID devices it drives, and BlueZ reports the same battery
/// over D-Bus. BlueZ wins, because it knows the real link state.
fn merge(sysfs: Vec<WirelessDevice>, bluez: Vec<WirelessDevice>) -> Vec<WirelessDevice> {
    let mut devices = bluez;

    for device in sysfs {
        if !devices.iter().any(|known| same_device(known, &device)) {
            devices.push(device);
        }
    }

    devices.sort_by(by_reachability);
    devices
}

/// The MAC address BlueZ encodes in a device's object path.
fn bluez_address(id: &str) -> Option<String> {
    let address = id.rsplit('/').next()?.strip_prefix("dev_")?;

    Some(address.replace('_', ":").to_ascii_lowercase())
}

/// Whether a BlueZ device and a sysfs device describe the same peripheral.
///
/// The kernel names a Bluetooth HID battery after the device's address, so the
/// address is the only identifier the two sources share. Devices whose supply
/// is named after the driver instead, such as `hidpp_battery_0`, are not
/// matched and show up twice.
fn same_device(bluez: &WirelessDevice, sysfs: &WirelessDevice) -> bool {
    bluez_address(&bluez.id)
        .is_some_and(|address| sysfs.id.to_ascii_lowercase().contains(&address))
}

/// Keeps a system bus connection alive across scans.
///
/// BlueZ is optional: a machine without bluetoothd still gets the sysfs
/// devices, so a failed connection is not an error.
#[derive(Default)]
struct BluezLink {
    connection: Option<zbus::Connection>,
    failing: bool,
}

impl BluezLink {
    async fn scan(&mut self) -> Vec<WirelessDevice> {
        if self
            .connection
            .as_ref()
            .is_some_and(|connection| connection.is_closed())
        {
            self.connection = None;
        }

        if self.connection.is_none() {
            match zbus::Connection::system().await {
                Ok(connection) => self.connection = Some(connection),
                Err(err) => {
                    self.report(&err);
                    return Vec::new();
                }
            }
        }

        let Some(connection) = self.connection.as_ref() else {
            return Vec::new();
        };

        match bluez::scan(connection).await {
            Ok(devices) => {
                self.failing = false;
                devices
            }
            Err(err) => {
                self.report(&err);
                Vec::new()
            }
        }
    }

    /// Logs a failure once, rather than on every poll.
    fn report(&mut self, err: &zbus::Error) {
        if !self.failing {
            tracing::debug!(?err, "bluetooth unavailable");
            self.failing = true;
        }
    }
}

/// Emits a snapshot on start and again whenever the devices change.
pub fn subscription() -> Subscription<BackendEvent> {
    Subscription::run_with("wireless-backend", |_| {
        stream::channel(16, |mut output: Sender<BackendEvent>| async move {
            let mut bluez = BluezLink::default();

            let mut last = merge(sysfs::scan(), bluez.scan().await);
            let _ = output.send(BackendEvent::Snapshot(last.clone())).await;

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

                let devices = merge(sysfs::scan(), bluez.scan().await);
                if devices == last {
                    continue;
                }

                last = devices.clone();
                let _ = output.send(BackendEvent::Snapshot(devices)).await;
            }
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(id: &str, name: &str, connected: bool) -> WirelessDevice {
        WirelessDevice {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: DeviceKind::Other,
            capacity: Some(50),
            state: ChargeState::Unknown,
            connected,
            wireless: true,
        }
    }

    #[test]
    fn extracts_the_address_from_a_bluez_path() {
        assert_eq!(
            bluez_address("/org/bluez/hci0/dev_68_CA_C4_D3_21_90").as_deref(),
            Some("68:ca:c4:d3:21:90")
        );
    }

    #[test]
    fn adapter_paths_have_no_address() {
        assert_eq!(bluez_address("/org/bluez/hci0"), None);
        assert_eq!(bluez_address("/org/bluez"), None);
    }

    #[test]
    fn matches_a_bluetooth_device_to_its_sysfs_supply() {
        let bluez = device("/org/bluez/hci0/dev_68_CA_C4_D3_21_90", "AirPods Pro", true);
        let sysfs = device("hid-68:ca:c4:d3:21:90-battery", "AirPods Pro", true);

        assert!(same_device(&bluez, &sysfs));
    }

    #[test]
    fn does_not_match_unrelated_devices() {
        let bluez = device("/org/bluez/hci0/dev_68_CA_C4_D3_21_90", "AirPods Pro", true);
        let sysfs = device("kysona-0003:3554:F58A.0005-battery", "Compx VXE", true);

        assert!(!same_device(&bluez, &sysfs));
    }

    #[test]
    fn bluez_wins_over_a_duplicate_sysfs_entry() {
        let bluez = device("/org/bluez/hci0/dev_68_CA_C4_D3_21_90", "AirPods Pro", true);
        let sysfs = device("hid-68:ca:c4:d3:21:90-battery", "AirPods Pro", false);

        let merged = merge(vec![sysfs], vec![bluez]);

        assert_eq!(merged.len(), 1);
        assert!(merged[0].connected);
        assert_eq!(merged[0].id, "/org/bluez/hci0/dev_68_CA_C4_D3_21_90");
    }

    #[test]
    fn keeps_devices_from_both_sources() {
        let bluez = device("/org/bluez/hci0/dev_68_CA_C4_D3_21_90", "AirPods Pro", true);
        let sysfs = device("kysona-0003:3554:F58A.0005-battery", "Compx VXE", true);

        let merged = merge(vec![sysfs], vec![bluez]);

        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn reachable_devices_sort_first() {
        let offline = device("a", "Zeta", false);
        let online = device("b", "Alpha", true);

        let merged = merge(vec![offline], vec![online]);

        assert_eq!(merged[0].name, "Alpha");
        assert_eq!(merged[1].name, "Zeta");
    }

    #[test]
    fn connected_device_without_a_charge_state_reads_as_connected() {
        let mut device = device("a", "AirPods Pro", true);
        device.state = ChargeState::Unknown;

        assert_eq!(device.status_label(), "Connected");
    }

    #[test]
    fn connected_device_with_a_charge_state_reports_it() {
        let mut device = device("a", "Compx VXE", true);
        device.state = ChargeState::Discharging;

        assert_eq!(device.status_label(), "Discharging");
    }

    #[test]
    fn disconnected_device_ignores_its_charge_state() {
        let mut device = device("a", "Compx VXE", false);
        device.state = ChargeState::Charging;

        assert_eq!(device.status_label(), "Disconnected");
    }

    #[test]
    fn maps_bluez_icons() {
        assert_eq!(DeviceKind::from_bluez_icon("input-mouse"), DeviceKind::Mouse);
        assert_eq!(
            DeviceKind::from_bluez_icon("input-keyboard"),
            DeviceKind::Keyboard
        );
        assert_eq!(
            DeviceKind::from_bluez_icon("audio-headphones"),
            DeviceKind::Headset
        );
        assert_eq!(
            DeviceKind::from_bluez_icon("input-gaming"),
            DeviceKind::Gamepad
        );
        assert_eq!(DeviceKind::from_bluez_icon("phone"), DeviceKind::Other);
        assert_eq!(DeviceKind::from_bluez_icon(""), DeviceKind::Other);
    }
}
