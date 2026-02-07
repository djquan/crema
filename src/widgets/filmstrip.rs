use std::collections::HashMap;

use iced::widget::{button, container, image, row, scrollable, text};
use iced::{Border, Color, Element, Length};

use crema_catalog::models::{Photo, PhotoId};

use crate::app::Message;

const THUMB_SIZE: f32 = 80.0;
const STRIP_HEIGHT: f32 = 100.0;
const HIGHLIGHT_COLOR: Color = Color::from_rgb(0.3, 0.5, 1.0);

pub fn view<'a>(
    photos: &[Photo],
    thumbnails: &'a HashMap<PhotoId, iced::widget::image::Handle>,
    selected: Option<PhotoId>,
) -> Element<'a, Message> {
    let items: Vec<Element<'a, Message>> = photos
        .iter()
        .map(|photo| {
            let is_selected = selected == Some(photo.id);

            let thumb_content: Element<'a, Message> =
                if let Some(handle) = thumbnails.get(&photo.id) {
                    image(handle.clone())
                        .width(THUMB_SIZE)
                        .height(THUMB_SIZE)
                        .into()
                } else {
                    container(text("...").size(10))
                        .width(THUMB_SIZE)
                        .height(THUMB_SIZE)
                        .center_x(THUMB_SIZE)
                        .center_y(THUMB_SIZE)
                        .into()
                };

            let border = if is_selected {
                Border {
                    color: HIGHLIGHT_COLOR,
                    width: 2.0,
                    radius: 2.0.into(),
                }
            } else {
                Border::default()
            };

            let cell = container(thumb_content).style(move |_theme: &_| container::Style {
                border,
                ..Default::default()
            });

            button(cell)
                .on_press(Message::SelectPhoto(photo.id))
                .padding(2)
                .style(button::text)
                .into()
        })
        .collect();

    container(
        scrollable(row(items).spacing(4).padding(6))
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            ))
            .width(Length::Fill),
    )
    .height(STRIP_HEIGHT)
    .style(|_theme: &_| container::Style {
        background: Some(Color::from_rgb(0.06, 0.06, 0.06).into()),
        ..Default::default()
    })
    .width(Length::Fill)
    .into()
}
