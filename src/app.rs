// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 Artem Wittermann

use std::sync::LazyLock;

use cosmic::app::{Core, Task};
use cosmic::applet::padded_control;
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::core::window;
use cosmic::iced::window::Id;
use cosmic::iced::{Alignment, Length, Rectangle, Subscription};
use cosmic::surface::action::{app_popup, destroy_popup};
use cosmic::theme;
use cosmic::widget::{
    Id as WidgetId, autosize, button, column, divider, icon, row, spin_button, text, toggler,
};
use cosmic::Element;

use crate::backend::{self, BackendEvent, ChargeState, WirelessDevice};
use crate::config::{APP_ID, Config, ICON, MAX_LOW_THRESHOLD, MIN_LOW_THRESHOLD};
use crate::fl;

static AUTOSIZE_ID: LazyLock<WidgetId> =
    LazyLock::new(|| WidgetId::new("wireless-applet-autosize"));

pub fn run() -> cosmic::iced::Result {
    cosmic::applet::run::<Applet>(())
}

pub struct Applet {
    core: Core,
    popup: Option<Id>,
    devices: Vec<WirelessDevice>,
    config: Config,
}

impl Default for Applet {
    fn default() -> Self {
        Self {
            core: Core::default(),
            popup: None,
            devices: Vec::new(),
            config: Config::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    Surface(cosmic::surface::Action<Message>),
    PopupClosed(Id),
    Backend(BackendEvent),
    ConfigChanged(Config),
    ToggleShowPercentage(bool),
    ToggleHideDisconnected(bool),
    SetLowThreshold(u8),
}

impl Applet {
    /// One panel entry: the device's own icon, then its charge.
    ///
    /// Only reachable devices get an entry. A device that is out of range
    /// keeps reporting its last known capacity, which would otherwise pin the
    /// panel to a stale number.
    fn device_group(
        &self,
        device: &WirelessDevice,
        horizontal: bool,
        suggested_size: (u16, u16),
        spacing: u16,
    ) -> Element<'_, Message> {
        let mut children: Vec<Element<'_, Message>> = Vec::with_capacity(2);

        children.push(
            icon::from_name(device.kind.icon_name())
                .size(suggested_size.0)
                .icon()
                .into(),
        );

        if self.config.show_percentage
            && let Some(capacity) = device.capacity
        {
            let label = self.core.applet.text(format!("{capacity}%"));
            let label = if horizontal {
                label
            } else {
                label.width(Length::Fixed(f32::from(suggested_size.0)))
            };

            children.push(
                label
                    .height(Length::Fixed(f32::from(suggested_size.1)))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .into(),
            );
        }

        // Tighter than the gap between devices, so each icon stays bound to
        // its own percentage.
        if horizontal {
            row::with_children(children)
                .spacing(spacing)
                .align_y(Alignment::Center)
                .into()
        } else {
            column::with_children(children)
                .spacing(spacing)
                .align_x(Alignment::Center)
                .into()
        }
    }
}

impl cosmic::Application for Applet {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: Self::Flags) -> (Self, Task<Message>) {
        (
            Self {
                core,
                ..Default::default()
            },
            Task::none(),
        )
    }

    fn on_close_requested(&self, id: window::Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Surface(action) => {
                return cosmic::task::message(cosmic::Action::Surface(action));
            }
            Message::PopupClosed(id) => {
                if self.popup == Some(id) {
                    self.popup = None;
                }
            }
            Message::Backend(BackendEvent::Snapshot(devices)) => {
                self.devices = devices;
            }
            Message::ConfigChanged(config) => {
                tracing::debug!(?config, "config loaded");
                self.config = config;
            }
            Message::ToggleShowPercentage(enabled) => {
                self.config.show_percentage = enabled;
                self.config.write();
            }
            Message::ToggleHideDisconnected(enabled) => {
                self.config.hide_disconnected = enabled;
                self.config.write();
            }
            Message::SetLowThreshold(threshold) => {
                self.config.low_threshold = threshold.clamp(MIN_LOW_THRESHOLD, MAX_LOW_THRESHOLD);
                self.config.write();
            }
        }

        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            backend::subscription().map(Message::Backend),
            self.core.watch_config(Self::APP_ID).map(|update| {
                for err in update.errors {
                    tracing::error!(?err, "failed to watch config");
                }

                Message::ConfigChanged(update.config)
            }),
        ])
    }

    fn view(&self) -> Element<'_, Message> {
        let horizontal = self.core.applet.is_horizontal();
        let suggested_size = self.core.applet.suggested_size(true);
        let applet_padding = self.core.applet.suggested_padding(true);
        let group_spacing = theme::active().cosmic().spacing.space_xxs;

        let connected: Vec<&WirelessDevice> = self
            .devices
            .iter()
            .filter(|device| device.connected)
            .collect();

        let mut groups: Vec<Element<'_, Message>> = Vec::with_capacity(connected.len());

        if connected.is_empty() {
            groups.push(
                icon::from_name(ICON)
                    .size(suggested_size.0)
                    .icon()
                    .into(),
            );
        } else {
            for device in connected {
                groups.push(self.device_group(device, horizontal, suggested_size, group_spacing));
            }
        }

        let content: Element<'_, Message> = if horizontal {
            row::with_children(groups)
                .spacing(applet_padding.1)
                .align_y(Alignment::Center)
                .into()
        } else {
            column::with_children(groups)
                .spacing(applet_padding.1)
                .align_x(Alignment::Center)
                .into()
        };

        let (horizontal_padding, vertical_padding) = if horizontal {
            (applet_padding.0, applet_padding.1)
        } else {
            (applet_padding.1, applet_padding.0)
        };

        let popup = self.popup;
        let button = button::custom(content)
            .padding([vertical_padding, horizontal_padding])
            .on_press_with_rectangle(move |offset, bounds| {
                if let Some(id) = popup {
                    Message::Surface(destroy_popup(id))
                } else {
                    Message::Surface(app_popup::<Self>(
                        |_| Default::default(),
                        move |state: &mut Self| {
                            let new_id = Id::unique();
                            state.popup = Some(new_id);

                            let mut settings = state.core.applet.get_popup_settings(
                                state.core.main_window_id().unwrap(),
                                new_id,
                                None,
                                None,
                                None,
                            );

                            settings.positioner.anchor_rect = Rectangle {
                                x: (bounds.x - offset.x) as i32,
                                y: (bounds.y - offset.y) as i32,
                                width: bounds.width as i32,
                                height: bounds.height as i32,
                            };

                            settings
                        },
                        Some(Box::new(|state: &Self| {
                            Element::from(
                                state.core.applet.popup_container::<Message>(popup_content(
                                    &state.devices,
                                    &state.config,
                                )),
                            )
                            .map(cosmic::Action::App)
                        })),
                    ))
                }
            })
            .class(theme::Button::AppletIcon);

        autosize::autosize(button, AUTOSIZE_ID.clone()).into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Message> {
        self.core
            .applet
            .popup_container::<Message>(popup_content(&self.devices, &self.config))
            .into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

/// The popup: the device list, then the settings.
fn popup_content<'a>(devices: &'a [WirelessDevice], config: &'a Config) -> Element<'a, Message> {
    let Spacing {
        space_xxs, space_s, ..
    } = theme::active().cosmic().spacing;

    let visible: Vec<&WirelessDevice> = devices
        .iter()
        .filter(|device| device.connected || !config.hide_disconnected)
        .collect();

    let mut rows: Vec<Element<'_, Message>> = Vec::with_capacity(visible.len());

    if visible.is_empty() {
        rows.push(padded_control(text::body(fl!("no-devices"))).into());
    } else {
        for device in visible {
            rows.push(device_row(device, config.low_threshold));
        }
    }

    column::with_children(rows)
        .padding([8, 0])
        .push(padded_control(divider::horizontal::default()).padding([space_xxs, space_s]))
        .push(padded_control(
            toggler(config.show_percentage)
                .on_toggle(Message::ToggleShowPercentage)
                .label(fl!("show-percentage"))
                .text_size(14)
                .width(Length::Fill),
        ))
        .push(padded_control(
            toggler(config.hide_disconnected)
                .on_toggle(Message::ToggleHideDisconnected)
                .label(fl!("hide-disconnected"))
                .text_size(14)
                .width(Length::Fill),
        ))
        .push(padded_control(
            row::with_children(vec![
                text::body(fl!("low-threshold"))
                    .width(Length::Fill)
                    .into(),
                spin_button(
                    format!("{}%", config.low_threshold),
                    config.low_threshold,
                    5u8,
                    MIN_LOW_THRESHOLD,
                    MAX_LOW_THRESHOLD,
                    Message::SetLowThreshold,
                )
                .into(),
            ])
            .align_y(Alignment::Center)
            .spacing(8),
        ))
        .into()
}

/// Dims text for devices that are out of range.
fn dimmed(theme: &cosmic::Theme) -> cosmic::iced::advanced::widget::text::Style {
    let mut color = theme.cosmic().background(theme.transparent).component.on;
    color.alpha *= 0.5;
    cosmic::iced::advanced::widget::text::Style {
        color: Some(color.into()),
        ..Default::default()
    }
}

/// Highlights a battery that is charging.
fn charging(theme: &cosmic::Theme) -> cosmic::iced::advanced::widget::text::Style {
    cosmic::iced::advanced::widget::text::Style {
        color: Some(theme.cosmic().accent_text_color().into()),
        ..Default::default()
    }
}

/// Warns about a battery that is running low.
fn low(theme: &cosmic::Theme) -> cosmic::iced::advanced::widget::text::Style {
    cosmic::iced::advanced::widget::text::Style {
        color: Some(theme.cosmic().warning_text_color().into()),
        ..Default::default()
    }
}

const DIMMED: theme::Text = theme::Text::Custom(dimmed);
const CHARGING: theme::Text = theme::Text::Custom(charging);
const LOW: theme::Text = theme::Text::Custom(low);

fn device_row(device: &WirelessDevice, low_threshold: u8) -> Element<'_, Message> {
    let mut details: Vec<Element<'_, Message>> = Vec::with_capacity(2);

    let name = text::body(device.name.clone());
    details.push(if device.connected {
        name.into()
    } else {
        name.class(DIMMED).into()
    });
    details.push(text::caption(device.status_label()).into());

    let capacity = match device.capacity {
        Some(value) if device.connected => format!("{value}%"),
        _ => "--".to_owned(),
    };

    let capacity = text::body(capacity);
    let capacity: Element<'_, Message> = if !device.connected {
        capacity.class(DIMMED).into()
    } else if device.state == ChargeState::Charging {
        capacity.class(CHARGING).into()
    } else if device.capacity.is_some_and(|value| value <= low_threshold) {
        capacity.class(LOW).into()
    } else {
        capacity.into()
    };

    padded_control(
        row::with_children(vec![
            icon::from_name(device.kind.icon_name())
                .size(24)
                .icon()
                .into(),
            column::with_children(details).width(Length::Fill).into(),
            capacity,
        ])
        .align_y(Alignment::Center)
        .spacing(12),
    )
    .into()
}
