// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 Artem Wittermann

use std::sync::LazyLock;

use cosmic::app::{Core, Task};
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::applet::{menu_control_padding, padded_control};
use cosmic::iced::core::window;
use cosmic::iced::window::Id;
use cosmic::iced::{Alignment, Length, Rectangle, Subscription};
use cosmic::surface::action::{app_popup, destroy_popup};
use cosmic::theme;
use cosmic::widget::{
    Id as WidgetId, autosize, button, column, divider, icon, list_column, row, text,
};
use cosmic::Element;

use crate::backend::{self, BackendEvent, ChargeState, WirelessDevice};
use crate::config::APP_ID;

static AUTOSIZE_ID: LazyLock<WidgetId> =
    LazyLock::new(|| WidgetId::new("wireless-applet-autosize"));

pub fn run() -> cosmic::iced::Result {
    cosmic::applet::run::<Applet>(())
}

pub struct Applet {
    core: Core,
    popup: Option<Id>,
    devices: Vec<WirelessDevice>,
    /// Local time of the last scan, shown in the popup.
    last_updated: Option<String>,
}

impl Default for Applet {
    fn default() -> Self {
        Self {
            core: Core::default(),
            popup: None,
            devices: Vec::new(),
            last_updated: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Message {
    Surface(cosmic::surface::Action<Message>),
    PopupClosed(Id),
    Backend(BackendEvent),
}

impl Applet {
    /// Lowest charge among devices that are currently reachable.
    ///
    /// A device that is out of range keeps reporting its last known capacity,
    /// which would otherwise pin the panel to a stale number.
    fn lowest_capacity(&self) -> Option<u8> {
        self.devices
            .iter()
            .filter(|device| device.connected)
            .filter_map(|device| device.capacity)
            .min()
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
            Message::Backend(BackendEvent::Snapshot {
                devices,
                scanned_at,
            }) => {
                self.devices = devices;
                self.last_updated = Some(scanned_at);
            }
        }

        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        backend::subscription().map(Message::Backend)
    }

    fn view(&self) -> Element<'_, Message> {
        let horizontal = matches!(
            self.core.applet.anchor,
            PanelAnchor::Top | PanelAnchor::Bottom
        );

        let mut children: Vec<Element<'_, Message>> = Vec::with_capacity(2);
        children.push(
            icon::from_name("network-wireless-symbolic")
                .size(16)
                .icon()
                .into(),
        );

        if let Some(capacity) = self.lowest_capacity() {
            children.push(text::body(format!("{capacity}%")).into());
        }

        let content = row::with_children(children)
            .spacing(4)
            .align_y(Alignment::Center);

        let popup = self.popup;
        let button = button::custom(content)
            .padding(if horizontal {
                [0, self.core.applet.suggested_padding(true).0]
            } else {
                [self.core.applet.suggested_padding(true).0, 0]
            })
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
                            Element::from(state.core.applet.popup_container::<Message>(
                                device_list(&state.devices, state.last_updated.as_deref()),
                            ))
                            .map(cosmic::Action::App)
                        })),
                    ))
                }
            })
            .class(theme::Button::AppletIcon);

        autosize::autosize(button, AUTOSIZE_ID.clone()).into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Message> {
        device_list(&self.devices, self.last_updated.as_deref())
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

fn device_list<'a>(
    devices: &'a [WirelessDevice],
    last_updated: Option<&str>,
) -> Element<'a, Message> {
    let mut list = list_column();

    if devices.is_empty() {
        list = list.add(
            cosmic::widget::container(text::body("No wireless devices found"))
                .padding(menu_control_padding()),
        );
    } else {
        for device in devices {
            list = list.add(device_row(device));
        }
    }

    let Some(updated) = last_updated else {
        return list.into();
    };

    let spacing = theme::active().cosmic().spacing;

    column::with_children(vec![
        list.into(),
        padded_control(divider::horizontal::default())
            .padding([spacing.space_xxs, spacing.space_s])
            .into(),
        padded_control(text::caption(format!("Updated {updated}"))).into(),
    ])
    .into()
}

/// Charge at or below which the percentage is called out.
const LOW_THRESHOLD: u8 = 20;

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

fn device_row(device: &WirelessDevice) -> Element<'_, Message> {
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
    } else if device.capacity.is_some_and(|value| value <= LOW_THRESHOLD) {
        capacity.class(LOW).into()
    } else {
        capacity.into()
    };

    row::with_children(vec![
        icon::from_name(device.kind.icon_name())
            .size(24)
            .icon()
            .into(),
        column::with_children(details).width(Length::Fill).into(),
        capacity,
    ])
    .align_y(Alignment::Center)
    .spacing(12)
    .padding(menu_control_padding())
    .into()
}
