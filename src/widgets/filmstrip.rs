use std::collections::HashMap;

use iced::widget::{button, column, container, image, row, scrollable, text};
use iced::{Background, Border, Color, Element, Length, Shadow, Theme};

use crema_catalog::models::{Photo, PhotoId};

use crate::app::Message;

const THUMB_SIZE: f32 = 92.0;
const STRIP_HEIGHT: f32 = 118.0;
const BG: Color = Color::from_rgb(0.08, 0.08, 0.09);
const CARD_BG: Color = Color::from_rgb(0.12, 0.12, 0.13);
const BORDER: Color = Color::from_rgb(0.20, 0.20, 0.22);
const ACCENT: Color = Color::from_rgb(0.26, 0.52, 0.94);
const MUTED: Color = Color::from_rgb(0.66, 0.66, 0.69);

pub fn view<'a>(
    photos: &[Photo],
    thumbnails: &'a HashMap<PhotoId, iced::widget::image::Handle>,
    selected: Option<PhotoId>,
) -> Element<'a, Message> {
    let items: Vec<Element<'a, Message>> = photos
        .iter()
        .map(|photo| {
            let is_selected = selected == Some(photo.id);
            let name = std::path::Path::new(&photo.file_path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .chars()
                .take(14)
                .collect::<String>();

            let thumb_content: Element<'a, Message> =
                if let Some(handle) = thumbnails.get(&photo.id) {
                    image(handle.clone())
                        .width(THUMB_SIZE)
                        .height(THUMB_SIZE)
                        .content_fit(iced::ContentFit::Cover)
                        .into()
                } else {
                    container(text("Loading").size(10).color(MUTED))
                        .width(THUMB_SIZE)
                        .height(THUMB_SIZE)
                        .center_x(THUMB_SIZE)
                        .center_y(THUMB_SIZE)
                        .into()
                };

            let cell = column![thumb_content, text(name).size(10).color(MUTED)].spacing(4);

            button(cell)
                .on_press(Message::OpenPhoto(photo.id))
                .padding(4)
                .style(move |_theme: &Theme, status| filmstrip_button_style(status, is_selected))
                .into()
        })
        .collect();

    container(
        scrollable(row(items).spacing(8).padding(8))
            .direction(scrollable::Direction::Horizontal(
                scrollable::Scrollbar::default(),
            ))
            .width(Length::Fill),
    )
    .height(STRIP_HEIGHT)
    .style(|_theme: &Theme| container::Style {
        background: Some(Background::Color(BG)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .width(Length::Fill)
    .into()
}

fn filmstrip_button_style(status: button::Status, is_selected: bool) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color::from_rgb(0.15, 0.15, 0.17),
        button::Status::Pressed => Color::from_rgb(0.11, 0.11, 0.12),
        _ if is_selected => Color::from_rgb(0.16, 0.20, 0.28),
        _ => CARD_BG,
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::WHITE,
        border: Border {
            color: if is_selected { ACCENT } else { BORDER },
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}
