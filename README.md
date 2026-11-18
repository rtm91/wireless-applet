# Wireless

A COSMIC panel applet showing the battery level and charging state of
wireless peripherals: mice, keyboards, headsets and gamepads.

Every reachable device gets its own icon and percentage in the panel, so two
connected peripherals read as two entries rather than one merged number. The
popup lists each device with its charge state, dims the ones that are switched
off or out of range, and holds the settings.

## Requirements

- COSMIC desktop
- Linux with `/sys/class/power_supply`
- BlueZ, for Bluetooth peripherals (optional — the applet runs without it)
- A Rust toolchain with edition 2024 support, to build

## Build

```sh
cargo build --release
just run            # runs the applet outside the panel
```

## Install

```sh
just install-user   # into ~/.local, no root needed
just install        # into /usr, needs root
```

Both install the binary, the desktop entry, the symbolic icon and the
metainfo. `just uninstall-user` and `just uninstall` remove them again.

`just validate` checks the desktop entry and the metainfo against their specs.

## Configuration

The settings sit in the popup, below the divider:

| Setting | Default | Meaning |
|---|---|---|
| Show percentage in panel | on | Show each device's charge next to its icon |
| Hide disconnected devices | off | Drop out-of-range peripherals instead of dimming them |
| Low battery warning | 20% | Charge at or below which the percentage is called out, 5–50% |

They are stored through cosmic-config at
`~/.config/cosmic/io.github.rtm91.WirelessApplet/v1/`, one RON file per key.

## How it works

Two sources are merged into one list:

- **sysfs** — every `/sys/class/power_supply` entry that looks like a
  peripheral: `scope=Device`, or a `wireless` attribute, or a model name that
  names a receiver. This covers 2.4 GHz receivers, which the kernel exposes as
  a power supply named after the driver. For device-scoped supplies the kernel
  reuses `online` to report whether the peripheral is reachable, not whether
  external power is present, so `online=0` means the device is off or out of
  range.
- **BlueZ** — `org.bluez.Battery1` on every device in `GetManagedObjects`.

A Bluetooth HID peripheral can appear in both. The kernel names its supply
after the device's address, so the address is the identifier the two sources
share, and BlueZ wins on a match because it knows the real link state.

Both sources are polled once a second. Polling is the only option for the
`kysona` driver: it computes `capacity` and `status` on read and never calls
`power_supply_changed()`, so no uevent and no inotify notification ever fires.
A udev monitor still runs alongside the poll to catch genuine hotplug, such as
the receiver being plugged in or pulled out.

## Known limitations

- Peripherals that only expose battery through a vendor protocol do not
  appear. BlueZ publishes `Battery1` only for devices that expose the standard
  GATT battery service, which AirPods and similar do not.
- Logitech peripherals whose supply is named after the driver
  (`hidpp_battery_0`) do not match their BlueZ entry and show up twice. A
  visible duplicate was chosen over a silent wrong merge.
- UPower is not used. It reads the same sysfs tree, polls at 30 seconds, and
  enumerates no devices at all on the machine this was developed on.

## License

GPL-3.0-only. See [LICENSE](LICENSE).
