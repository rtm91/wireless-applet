// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 Artem Wittermann

mod app;
mod backend;
mod config;

pub fn run() -> cosmic::iced::Result {
    app::run()
}
