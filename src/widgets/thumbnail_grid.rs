use std::collections::HashMap;

use iced::widget::{Space, button, column, container, image, responsive, row, text};
use iced::{Background, Border, Color, Element, Length, Shadow, Theme};

use crema_catalog::models::{Photo, PhotoId};

use crate::app::Message;

const TARGET_WIDTH: f32 = 210.0;
const MIN_WIDTH: f32 = 170.0;
const MAX_WIDTH: f32 = 240.0;
const CARD_BG: Color = Color::from_rgb(0.11, 0.11, 0.12);
const CARD_HOVER: Color = Color::from_rgb(0.14, 0.14, 0.16);
const CARD_SELECTED: Color = Color::from_rgb(0.16, 0.20, 0.28);
const BORDER: Color = Color::from_rgb(0.20, 0.20, 0.22);
const ACCENT: Color = Color::from_rgb(0.26, 0.52, 0.94);
const MUTED: Color = Color::from_rgb(0.66, 0.66, 0.69);

pub fn view<'a>(
    photos: Vec<&'a Photo>,
    thumbnails: &'a HashMap<PhotoId, iced::widget::image::Handle>,
    selected: Option<PhotoId>,
) -> Element<'a, Message> {
    if photos.is_empty() {
        return container(
            column![
                text("No photos in this view.").size(18),
                text("Import files or choose a different date filter.")
                    .size(13)
                    .color(MUTED),
            ]
            .spacing(6),
        )
        .padding(40)
        .center_x(Length::Fill)
        .into();
    }

    responsive(move |size| {
        let available = (size.width - 24.0).max(MIN_WIDTH);
        let columns = (available / TARGET_WIDTH).floor().max(1.0) as usize;
        let cell_width = (available / columns as f32).clamp(MIN_WIDTH, MAX_WIDTH);

        let mut grid_rows: Vec<Element<'a, Message>> = Vec::new();
        let mut current_row: Vec<Element<'a, Message>> = Vec::new();

        for photo in &photos {
            current_row.push(photo_cell(photo, thumbnails.get(&photo.id), selected, cell_width));

            if current_row.len() == columns {
                grid_rows.push(row(std::mem::take(&mut current_row)).spacing(12).into());
            }
        }

        if !current_row.is_empty() {
            while current_row.len() < columns {
                current_row.push(Space::new().width(cell_width).into());
            }
            grid_rows.push(row(std::mem::take(&mut current_row)).spacing(12).into());
        }

        column(grid_rows).spacing(12).into()
    })
    .into()
}

fn photo_cell<'a>(
    photo: &'a Photo,
    thumbnail: Option<&'a iced::widget::image::Handle>,
    selected: Option<PhotoId>,
    width: f32,
) -> Element<'a, Message> {
    let is_selected = selected == Some(photo.id);
    let filename = std::path::Path::new(&photo.file_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let thumb_height = width * 0.72;
    let thumb_content: Element<'a, Message> = if let Some(handle) = thumbnail {
        image(handle.clone())
            .width(width)
            .height(thumb_height)
            .content_fit(iced::ContentFit::Cover)
            .into()
    } else {
        container(text("Loading thumbnail").size(11).color(MUTED))
            .width(width)
            .height(thumb_height)
            .center_x(width)
            .center_y(thumb_height)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(Color::from_rgb(0.13, 0.13, 0.14))),
                border: Border {
                    color: BORDER,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..Default::default()
            })
            .into()
    };

    let date_label = photo
        .date_taken
        .as_deref()
        .map(|value| value.chars().take(10).collect::<String>())
        .unwrap_or_else(|| "Unknown date".into());

    let mut card = column![
        button(thumb_content)
            .on_press(Message::SelectPhoto(photo.id))
            .padding(0)
            .width(width)
            .style(move |_theme: &Theme, status| thumb_button_style(status, is_selected)),
        text(filename).size(12),
        text(date_label).size(11).color(MUTED),
    ]
    .spacing(6)
    .width(width);

    if is_selected {
        card = card.push(
            row![
                text("Selected").size(11).color(ACCENT),
                Space::new().width(Length::Fill),
                button("Develop")
                    .on_press(Message::OpenPhoto(photo.id))
                    .padding([3, 8])
                    .style(move |_theme: &Theme, status| open_button_style(status)),
            ]
            .align_y(iced::Alignment::Center),
        );
    }

    container(card)
        .padding(10)
        .style(move |_theme: &Theme| container::Style {
            background: Some(Background::Color(if is_selected { CARD_SELECTED } else { CARD_BG })),
            border: Border {
                color: if is_selected { ACCENT } else { BORDER },
                width: 1.0,
                radius: 8.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn thumb_button_style(status: button::Status, selected: bool) -> button::Style {
    let background = match status {
        button::Status::Hovered => CARD_HOVER,
        button::Status::Pressed => Color::from_rgb(0.12, 0.12, 0.14),
        _ if selected => CARD_SELECTED,
        _ => CARD_BG,
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::WHITE,
        border: Border {
            color: if selected { ACCENT } else { BORDER },
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

fn open_button_style(status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color::from_rgb(0.30, 0.56, 0.98),
        button::Status::Pressed => Color::from_rgb(0.21, 0.44, 0.82),
        _ => ACCENT,
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::WHITE,
        border: Border {
            color: background,
            width: 1.0,
            radius: 5.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}
