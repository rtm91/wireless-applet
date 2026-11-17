// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 Artem Wittermann

use std::path::Path;

use super::{ChargeState, DeviceKind, WirelessDevice};

const POWER_SUPPLY: &str = "/sys/class/power_supply";

/// Reads every wireless device currently exposed by the kernel.
pub fn scan() -> Vec<WirelessDevice> {
    scan_in(Path::new(POWER_SUPPLY))
}

fn scan_in(root: &Path) -> Vec<WirelessDevice> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut devices: Vec<WirelessDevice> = entries
        .flatten()
        .filter_map(|entry| read_device(&entry.path()))
        .collect();

    // Reachable devices first, so a peripheral that is switched off does not
    // push the live ones down the list.
    devices.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then_with(|| a.name.cmp(&b.name))
    });
    devices
}

fn read_device(dir: &Path) -> Option<WirelessDevice> {
    if read_attr(dir, "present").as_deref() == Some("0") {
        return None;
    }

    let kind = DeviceKind::from_sysfs(&read_attr(dir, "type")?);
    let model = read_attr(dir, "model_name").unwrap_or_default();
    let scope = read_attr(dir, "scope").unwrap_or_default();
    let wireless = read_attr(dir, "wireless").as_deref() == Some("1");

    if !is_wireless(wireless, &scope, &model) {
        return None;
    }

    let kind = infer_kind(kind, &model);

    let id = dir.file_name()?.to_string_lossy().into_owned();
    let name = if model.is_empty() { id.clone() } else { model };

    let capacity = read_capacity(dir);
    let mut state = ChargeState::from_sysfs(&read_attr(dir, "status").unwrap_or_default());

    // Plenty of devices report `Discharging` no matter what, so a battery at
    // full is reported as full.
    if capacity == Some(100) {
        state = ChargeState::Full;
    }

    Some(WirelessDevice {
        id,
        name,
        kind,
        capacity,
        state,
        connected: read_connected(dir, &scope, state),
        wireless,
    })
}

/// Reads the charge percentage, falling back to the coarse `capacity_level`
/// attribute that some devices report instead of a number.
fn read_capacity(dir: &Path) -> Option<u8> {
    if let Some(value) = read_attr(dir, "capacity").and_then(|value| value.parse::<u8>().ok()) {
        return Some(value.min(100));
    }

    let percent = match read_attr(dir, "capacity_level")?.to_ascii_lowercase().as_str() {
        "full" => 100,
        "high" => 70,
        "normal" => 55,
        "low" => 10,
        "critical" => 5,
        "unknown" => 50,
        _ => return None,
    };

    Some(percent)
}

/// Decides whether the peripheral is currently reachable.
///
/// Device-scoped supplies reuse `online` to report whether the wireless link
/// is up rather than whether external power is present. System-scoped supplies
/// keep the usual meaning, so only trust it for peripherals.
fn read_connected(dir: &Path, scope: &str, state: ChargeState) -> bool {
    let Some(value) = read_attr(dir, "online") else {
        return true;
    };

    if !scope.eq_ignore_ascii_case("device") {
        return true;
    }

    let link_up = value == "1";

    // Logitech devices reuse `online` for battery status instead: it reads 0
    // while the device is wired or charging, where `status` is still
    // meaningful. Only treat that as a disconnect when the charge state is
    // unknown as well.
    let logitech_quirk = !link_up
        && state != ChargeState::Unknown
        && read_attr(dir, "manufacturer")
            .is_some_and(|vendor| vendor.to_ascii_lowercase().contains("logitech"));

    link_up || logitech_quirk
}

fn read_attr(dir: &Path, name: &str) -> Option<String> {
    std::fs::read_to_string(dir.join(name))
        .ok()
        .map(|value| value.trim().to_owned())
}

/// Decides whether a power supply belongs to a peripheral rather than the
/// system.
///
/// The kernel only exposes `POWER_SUPPLY_PROP_WIRELESS` for hardware that
/// reports it, which is rare. Peripherals behind a 2.4 GHz dongle or a
/// Bluetooth link instead appear as a device-scoped battery, so scope and
/// model name serve as fallbacks.
fn is_wireless(wireless: bool, scope: &str, model: &str) -> bool {
    if wireless {
        return true;
    }

    let model = model.to_ascii_lowercase();
    if ["dongle", "receiver", "wireless", "2.4g", "bluetooth"]
        .iter()
        .any(|hint| model.contains(hint))
    {
        return true;
    }

    // `scope=Device` is the kernel declaring that this battery powers a
    // peripheral rather than the system, which is the strongest signal
    // available. Model names are only a hint and must not gate this, or
    // devices with unrecognised names get dropped.
    scope.eq_ignore_ascii_case("device")
}

/// Refines a generic battery type using the model name, since dongles report
/// `type=Battery` rather than the kind of peripheral they carry.
fn infer_kind(kind: DeviceKind, model: &str) -> DeviceKind {
    if kind != DeviceKind::Other {
        return kind;
    }

    let model = model.to_ascii_lowercase();
    if model.contains("mouse") {
        DeviceKind::Mouse
    } else if model.contains("keyboard") {
        DeviceKind::Keyboard
    } else if model.contains("headset")
        || model.contains("headphone")
        || model.contains("airpods")
        || model.contains("earbud")
    {
        DeviceKind::Headset
    } else {
        DeviceKind::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(name: &str, attrs: &[(&str, &str)]) -> Self {
            let dir = std::env::temp_dir().join(format!("wireless-applet-{name}"));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            for (key, value) in attrs {
                fs::write(dir.join(key), value).unwrap();
            }
            Self(dir)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_device_kind() {
        assert_eq!(DeviceKind::from_sysfs("Mouse"), DeviceKind::Mouse);
        assert_eq!(DeviceKind::from_sysfs("Keyboard\n"), DeviceKind::Keyboard);
        assert_eq!(DeviceKind::from_sysfs("Headset"), DeviceKind::Headset);
        assert_eq!(DeviceKind::from_sysfs("Battery"), DeviceKind::Other);
    }

    #[test]
    fn parses_charge_state() {
        assert_eq!(ChargeState::from_sysfs("Charging"), ChargeState::Charging);
        assert_eq!(
            ChargeState::from_sysfs("Not charging"),
            ChargeState::NotCharging
        );
        assert_eq!(ChargeState::from_sysfs("Full"), ChargeState::Full);
        assert_eq!(ChargeState::from_sysfs("Unknown"), ChargeState::Unknown);
        assert_eq!(ChargeState::from_sysfs(""), ChargeState::Unknown);
    }

    #[test]
    fn wireless_attribute_wins() {
        assert!(is_wireless(true, "System", ""));
    }

    #[test]
    fn device_scope_peripheral_is_wireless() {
        assert!(is_wireless(false, "Device", ""));
    }

    #[test]
    fn system_scope_battery_is_not_wireless() {
        assert!(!is_wireless(false, "System", ""));
    }

    #[test]
    fn model_name_hint_marks_wireless() {
        assert!(is_wireless(false, "System", "Compx VXE NordicMouse 1K Dongle"));
    }

    #[test]
    fn unrecognised_model_name_is_still_a_peripheral() {
        // Regression: a device-scoped battery used to be dropped unless its
        // model name matched a known keyword.
        assert!(is_wireless(false, "Device", "WH-1000XM5"));
    }

    #[test]
    fn infers_kind_from_model_name() {
        assert_eq!(
            infer_kind(DeviceKind::Other, "Compx VXE NordicMouse 1K Dongle"),
            DeviceKind::Mouse
        );
        assert_eq!(
            infer_kind(DeviceKind::Other, "2.4G Wireless Keyboard"),
            DeviceKind::Keyboard
        );
        assert_eq!(
            infer_kind(DeviceKind::Other, "AirPods Pro"),
            DeviceKind::Headset
        );
        assert_eq!(infer_kind(DeviceKind::Other, "UPS"), DeviceKind::Other);
        assert_eq!(infer_kind(DeviceKind::Mouse, "anything"), DeviceKind::Mouse);
    }

    #[test]
    fn reads_a_dongle_battery() {
        let fixture = Fixture::new(
            "dongle",
            &[
                ("type", "Battery"),
                ("status", "Unknown"),
                ("capacity", "60"),
                ("online", "0"),
                ("present", "1"),
                ("scope", "Device"),
                ("model_name", "Compx VXE NordicMouse 1K Dongle"),
            ],
        );

        let device = read_device(&fixture.0).expect("device should be recognised");

        assert_eq!(device.name, "Compx VXE NordicMouse 1K Dongle");
        assert_eq!(device.capacity, Some(60));
        assert_eq!(device.state, ChargeState::Unknown);
        assert!(!device.connected);
        assert!(!device.wireless);
    }

    #[test]
    fn online_marks_a_device_connected() {
        let fixture = Fixture::new(
            "connected",
            &[
                ("type", "Battery"),
                ("status", "Discharging"),
                ("capacity", "80"),
                ("online", "1"),
                ("present", "1"),
                ("scope", "Device"),
                ("model_name", "Compx VXE NordicMouse 1K Dongle"),
            ],
        );

        let device = read_device(&fixture.0).expect("device should be recognised");

        assert!(device.connected);
        assert_eq!(device.status_label(), "Discharging");
    }

    #[test]
    fn missing_online_attribute_is_connected() {
        let fixture = Fixture::new(
            "no-online",
            &[
                ("type", "Mouse"),
                ("scope", "Device"),
                ("capacity", "55"),
            ],
        );

        assert!(read_device(&fixture.0).unwrap().connected);
    }

    #[test]
    fn system_scope_online_is_not_a_link_state() {
        let fixture = Fixture::new(
            "system-online",
            &[
                ("type", "Battery"),
                ("scope", "System"),
                ("online", "0"),
                ("capacity", "80"),
                ("model_name", "Wireless Keyboard"),
            ],
        );

        assert!(read_device(&fixture.0).unwrap().connected);
    }

    #[test]
    fn disconnected_device_reports_its_link_state() {
        let fixture = Fixture::new(
            "disconnected",
            &[
                ("type", "Battery"),
                ("status", "Unknown"),
                ("capacity", "90"),
                ("online", "0"),
                ("present", "1"),
                ("scope", "Device"),
                ("model_name", "Compx VXE NordicMouse 1K Dongle"),
            ],
        );

        let device = read_device(&fixture.0).unwrap();

        assert!(!device.connected);
        assert_eq!(device.status_label(), "Disconnected");
    }

    #[test]
    fn reads_a_device_with_an_unrecognised_name() {
        let fixture = Fixture::new(
            "unknown-name",
            &[
                ("type", "Battery"),
                ("status", "Discharging"),
                ("capacity", "70"),
                ("online", "1"),
                ("present", "1"),
                ("scope", "Device"),
                ("model_name", "WH-1000XM5"),
            ],
        );

        let device = read_device(&fixture.0).expect("device should be recognised");

        assert_eq!(device.name, "WH-1000XM5");
        assert_eq!(device.kind, DeviceKind::Other);
        assert_eq!(device.capacity, Some(70));
    }

    #[test]
    fn falls_back_to_capacity_level() {
        let fixture = Fixture::new(
            "capacity-level",
            &[
                ("type", "Mouse"),
                ("scope", "Device"),
                ("capacity_level", "Low"),
            ],
        );

        assert_eq!(read_device(&fixture.0).unwrap().capacity, Some(10));
    }

    #[test]
    fn capacity_wins_over_capacity_level() {
        let fixture = Fixture::new(
            "both-capacities",
            &[
                ("type", "Mouse"),
                ("scope", "Device"),
                ("capacity", "42"),
                ("capacity_level", "Full"),
            ],
        );

        assert_eq!(read_device(&fixture.0).unwrap().capacity, Some(42));
    }

    #[test]
    fn parses_empty_charge_state() {
        assert_eq!(ChargeState::from_sysfs("Empty"), ChargeState::Empty);
        assert_eq!(ChargeState::Empty.label(), "Empty");
    }

    #[test]
    fn full_capacity_overrides_discharging() {
        let fixture = Fixture::new(
            "full-capacity",
            &[
                ("type", "Mouse"),
                ("scope", "Device"),
                ("capacity", "100"),
                ("status", "Discharging"),
            ],
        );

        assert_eq!(read_device(&fixture.0).unwrap().state, ChargeState::Full);
    }

    #[test]
    fn logitech_offline_with_known_state_stays_connected() {
        let fixture = Fixture::new(
            "logitech-wired",
            &[
                ("type", "Mouse"),
                ("scope", "Device"),
                ("online", "0"),
                ("status", "Charging"),
                ("manufacturer", "Logitech"),
                ("capacity", "60"),
            ],
        );

        assert!(read_device(&fixture.0).unwrap().connected);
    }

    #[test]
    fn logitech_offline_with_unknown_state_is_disconnected() {
        let fixture = Fixture::new(
            "logitech-off",
            &[
                ("type", "Mouse"),
                ("scope", "Device"),
                ("online", "0"),
                ("status", "Unknown"),
                ("manufacturer", "Logitech"),
                ("capacity", "60"),
            ],
        );

        assert!(!read_device(&fixture.0).unwrap().connected);
    }

    #[test]
    fn other_vendors_offline_is_disconnected() {
        let fixture = Fixture::new(
            "other-vendor-off",
            &[
                ("type", "Mouse"),
                ("scope", "Device"),
                ("online", "0"),
                ("status", "Charging"),
                ("manufacturer", "Compx"),
                ("capacity", "60"),
            ],
        );

        assert!(!read_device(&fixture.0).unwrap().connected);
    }

    #[test]
    fn skips_absent_devices() {
        let fixture = Fixture::new(
            "absent",
            &[("type", "Battery"), ("present", "0"), ("scope", "Device")],
        );

        assert!(read_device(&fixture.0).is_none());
    }

    #[test]
    fn skips_system_batteries() {
        let fixture = Fixture::new(
            "system",
            &[
                ("type", "Battery"),
                ("scope", "System"),
                ("capacity", "80"),
            ],
        );

        assert!(read_device(&fixture.0).is_none());
    }

    #[test]
    fn clamps_capacity_above_one_hundred() {
        let fixture = Fixture::new(
            "clamp",
            &[
                ("type", "Mouse"),
                ("scope", "Device"),
                ("capacity", "150"),
            ],
        );

        assert_eq!(read_device(&fixture.0).unwrap().capacity, Some(100));
    }

    #[test]
    fn missing_capacity_is_none() {
        let fixture = Fixture::new(
            "no-capacity",
            &[("type", "Mouse"), ("scope", "Device")],
        );

        assert_eq!(read_device(&fixture.0).unwrap().capacity, None);
    }

    /// Smoke test against the live sysfs tree, so the heuristics can be checked
    /// against real hardware. Run with `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn scans_the_live_system() {
        let devices = scan();
        println!("{devices:#?}");
        assert!(!devices.is_empty(), "expected at least one wireless device");
    }
}
