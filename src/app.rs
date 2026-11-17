// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 Artem Wittermann

use cosmic::app::{Core, Task};
use cosmic::iced::Rectangle;
use cosmic::iced::core::window;
use cosmic::iced::window::Id;
use cosmic::surface::action::{app_popup, destroy_popup};
use cosmic::widget;
use cosmic::Element;

use crate::config::APP_ID;

pub fn run() -> cosmic::iced::Result {
    cosmic::applet::run::<Applet>(())
}

#[derive(Default)]
pub struct Applet {
    core: Core,
    popup: Option<Id>,
}

#[derive(Clone, Debug)]
pub enum Message {
    Surface(cosmic::surface::Action<Message>),
    PopupClosed(Id),
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
                popup: None,
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
        }

        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let popup = self.popup;

        let button = self
            .core
            .applet
            .icon_button("network-wireless-symbolic")
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
                            let content = widget::container(widget::text::body(
                                "No wireless devices found",
                            ));

                            Element::from(state.core.applet.popup_container::<Message>(content))
                                .map(cosmic::Action::App)
                        })),
                    ))
                }
            });

        Element::from(self.core.applet.applet_tooltip::<Message>(
            button,
            "Wireless",
            self.popup.is_some(),
            Message::Surface,
            None,
        ))
    }

    fn view_window(&self, _id: Id) -> Element<'_, Message> {
        widget::text::body("Wireless").into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
