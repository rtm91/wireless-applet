// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 Artem Wittermann

use std::collections::HashMap;

use zbus::{OwnedObjectPath, OwnedVariant};

use super::{ChargeState, DeviceKind, WirelessDevice};

const BLUEZ: &str = "org.bluez";
const ROOT: &str = "/";
const OBJECT_MANAGER: &str = "org.freedesktop.DBus.ObjectManager";

const BATTERY: &str = "org.bluez.Battery1";
const DEVICE: &str = "org.bluez.Device1";

type Interfaces = HashMap<String, HashMap<String, OwnedVariant>>;
type Objects = HashMap<OwnedObjectPath, Interfaces>;

/// Reads every Bluetooth device that currently reports a battery.
///
/// BlueZ only publishes `org.bluez.Battery1` while a device is connected and
/// actually reports a level, so a paired-but-idle headset drops out of the
/// list instead of showing a stale number.
pub async fn scan(connection: &zbus::Connection) -> zbus::Result<Vec<WirelessDevice>> {
    let reply = connection
        .call_method(
            Some(BLUEZ),
            ROOT,
            Some(OBJECT_MANAGER),
            "GetManagedObjects",
            &(),
        )
        .await?;

    let objects: Objects = reply.body().deserialize()?;

    Ok(devices_from_objects(&objects))
}

fn devices_from_objects(objects: &Objects) -> Vec<WirelessDevice> {
    objects
        .iter()
        .filter_map(|(path, interfaces)| device_from_object(path, interfaces))
        .collect()
}

fn device_from_object(path: &OwnedObjectPath, interfaces: &Interfaces) -> Option<WirelessDevice> {
    let battery = interfaces.get(BATTERY)?;
    let capacity = battery.get("Percentage").and_then(as_u8)?;

    let device = interfaces.get(DEVICE);

    let name = device
        .and_then(|props| props.get("Alias").or_else(|| props.get("Name")))
        .and_then(as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| path.as_str().to_owned());

    let connected = device
        .and_then(|props| props.get("Connected"))
        .and_then(as_bool)
        .unwrap_or(false);

    let kind = device
        .and_then(|props| props.get("Icon"))
        .and_then(as_str)
        .map(DeviceKind::from_bluez_icon)
        .unwrap_or(DeviceKind::Other);

    Some(WirelessDevice {
        id: path.as_str().to_owned(),
        name,
        kind,
        capacity: Some(capacity.min(100)),
        // BlueZ reports a level but no charge state.
        state: ChargeState::Unknown,
        connected,
        wireless: true,
    })
}

fn as_u8(value: &OwnedVariant) -> Option<u8> {
    value.downcast_ref::<u8>().ok()
}

fn as_bool(value: &OwnedVariant) -> Option<bool> {
    value.downcast_ref::<bool>().ok()
}

fn as_str(value: &OwnedVariant) -> Option<&str> {
    value.downcast_ref::<&str>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zbus::Str;

    fn interfaces(entries: Vec<(&str, Vec<(&str, OwnedVariant)>)>) -> Interfaces {
        entries
            .into_iter()
            .map(|(name, props)| {
                (
                    name.to_owned(),
                    props
                        .into_iter()
                        .map(|(key, value)| (key.to_owned(), value))
                        .collect(),
                )
            })
            .collect()
    }

    fn path(value: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(value).unwrap()
    }

    fn airpods(percentage: u8, connected: bool) -> Interfaces {
        interfaces(vec![
            (BATTERY, vec![("Percentage", OwnedVariant::from(percentage))]),
            (
                DEVICE,
                vec![
                    ("Alias", OwnedVariant::from(Str::from("AirPods Pro"))),
                    ("Connected", OwnedVariant::from(connected)),
                    ("Icon", OwnedVariant::from(Str::from("audio-headphones"))),
                ],
            ),
        ])
    }

    #[test]
    fn reads_a_bluetooth_battery() {
        let path = path("/org/bluez/hci0/dev_68_CA_C4_D3_21_90");
        let device = device_from_object(&path, &airpods(85, true)).expect("battery should be read");

        assert_eq!(device.name, "AirPods Pro");
        assert_eq!(device.capacity, Some(85));
        assert_eq!(device.kind, DeviceKind::Headset);
        assert!(device.connected);
        assert!(device.wireless);
        assert_eq!(device.id, "/org/bluez/hci0/dev_68_CA_C4_D3_21_90");
    }

    #[test]
    fn bluez_devices_have_no_charge_state() {
        let path = path("/org/bluez/hci0/dev_68_CA_C4_D3_21_90");
        let device = device_from_object(&path, &airpods(85, true)).unwrap();

        assert_eq!(device.state, ChargeState::Unknown);
        assert_eq!(device.status_label(), "Connected");
    }

    #[test]
    fn disconnected_bluetooth_device_reports_its_link_state() {
        let path = path("/org/bluez/hci0/dev_68_CA_C4_D3_21_90");
        let device = device_from_object(&path, &airpods(85, false)).unwrap();

        assert!(!device.connected);
        assert_eq!(device.status_label(), "Disconnected");
    }

    #[test]
    fn skips_devices_without_a_battery_interface() {
        let path = path("/org/bluez/hci0/dev_68_CA_C4_D3_21_90");
        let interfaces = interfaces(vec![(
            DEVICE,
            vec![("Alias", OwnedVariant::from(Str::from("AirPods Pro")))],
        )]);

        assert!(device_from_object(&path, &interfaces).is_none());
    }

    #[test]
    fn skips_devices_without_a_percentage() {
        let path = path("/org/bluez/hci0/dev_68_CA_C4_D3_21_90");
        let interfaces = interfaces(vec![(BATTERY, vec![])]);

        assert!(device_from_object(&path, &interfaces).is_none());
    }

    #[test]
    fn falls_back_to_the_device_name() {
        let path = path("/org/bluez/hci0/dev_68_CA_C4_D3_21_90");
        let interfaces = interfaces(vec![
            (BATTERY, vec![("Percentage", OwnedVariant::from(60u8))]),
            (
                DEVICE,
                vec![("Name", OwnedVariant::from(Str::from("WH-1000XM5")))],
            ),
        ]);

        assert_eq!(device_from_object(&path, &interfaces).unwrap().name, "WH-1000XM5");
    }

    #[test]
    fn falls_back_to_the_object_path() {
        let path = path("/org/bluez/hci0/dev_68_CA_C4_D3_21_90");
        let interfaces = interfaces(vec![(
            BATTERY,
            vec![("Percentage", OwnedVariant::from(60u8))],
        )]);

        let device = device_from_object(&path, &interfaces).unwrap();

        assert_eq!(device.name, "/org/bluez/hci0/dev_68_CA_C4_D3_21_90");
        assert_eq!(device.kind, DeviceKind::Other);
        assert!(!device.connected);
    }

    #[test]
    fn clamps_percentage_above_one_hundred() {
        let path = path("/org/bluez/hci0/dev_68_CA_C4_D3_21_90");
        let interfaces = interfaces(vec![(
            BATTERY,
            vec![("Percentage", OwnedVariant::from(150u8))],
        )]);

        assert_eq!(
            device_from_object(&path, &interfaces).unwrap().capacity,
            Some(100)
        );
    }

    #[test]
    fn ignores_objects_that_are_not_devices() {
        let objects = HashMap::from([
            (
                path("/org/bluez/hci0"),
                interfaces(vec![(
                    "org.bluez.Adapter1",
                    vec![("Powered", OwnedVariant::from(true))],
                )]),
            ),
            (
                path("/org/bluez/hci0/dev_68_CA_C4_D3_21_90"),
                airpods(85, true),
            ),
        ]);

        let devices = devices_from_objects(&objects);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "AirPods Pro");
    }

    /// Smoke test against the live system bus, so the parsing can be checked
    /// against real BlueZ. Run with `cargo test -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn scans_the_live_system() {
        let connection = zbus::Connection::system().await.unwrap();
        let devices = scan(&connection).await.unwrap();

        println!("{devices:#?}");
    }
}
