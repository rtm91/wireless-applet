// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 Artem Wittermann

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};

pub const APP_ID: &str = "io.github.rtm91.WirelessApplet";

/// Charge at or below which the percentage is called out.
pub const DEFAULT_LOW_THRESHOLD: u8 = 20;

/// Lowest threshold the settings allow.
pub const MIN_LOW_THRESHOLD: u8 = 5;

/// Highest threshold the settings allow.
pub const MAX_LOW_THRESHOLD: u8 = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, CosmicConfigEntry)]
#[version = 1]
pub struct Config {
    /// Show the charge percentage next to the panel icon.
    pub show_percentage: bool,
    /// Charge at or below which the percentage is called out.
    pub low_threshold: u8,
    /// Hide peripherals that are switched off or out of range.
    pub hide_disconnected: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_percentage: true,
            low_threshold: DEFAULT_LOW_THRESHOLD,
            hide_disconnected: false,
        }
    }
}

impl Config {
    /// Persists the current settings, logging rather than propagating a
    /// failure: a read-only config directory should not take the applet down.
    pub fn write(&self) {
        let Ok(helper) = cosmic_config::Config::new(APP_ID, Self::VERSION) else {
            tracing::error!("failed to open config");
            return;
        };

        if let Err(err) = self.write_entry(&helper) {
            tracing::error!(?err, "failed to write config");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_show_the_percentage() {
        let config = Config::default();

        assert!(config.show_percentage);
        assert!(!config.hide_disconnected);
        assert_eq!(config.low_threshold, DEFAULT_LOW_THRESHOLD);
    }

    #[test]
    fn the_default_threshold_is_within_the_allowed_range() {
        assert!((MIN_LOW_THRESHOLD..=MAX_LOW_THRESHOLD).contains(&DEFAULT_LOW_THRESHOLD));
    }
}
