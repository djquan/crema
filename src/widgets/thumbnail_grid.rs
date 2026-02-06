use std::collections::HashMap;

use iced::widget::{button, column, container, image, row, text, Space};
use iced::{Element, Length};

use photors_catalog::models::{Photo, PhotoId};

use crate::app::Message;

const THUMB_SIZE: f32 = 200.0;
const GRID_COLUMNS: usize = 5;

pub fn view<'a>(
    photos: &'a [Photo],
    thumbnails: &'a HashMap<PhotoId, iced::widget::image::Handle>,
) -> Element<'a, Message> {
    if photos.is_empty() {
        return container(text("No photos. Click 'Import Folder' to add some.").size(16))
            .padding(40)
            .center_x(Length::Fill)
            .into();
    }

    let mut grid_rows: Vec<Element<'a, Message>> = Vec::new();
    let mut current_row: Vec<Element<'a, Message>> = Vec::new();

    for photo in photos {
        let cell = photo_cell(photo, thumbnails.get(&photo.id));
        current_row.push(cell);

        if current_row.len() >= GRID_COLUMNS {
            grid_rows.push(
                row(std::mem::take(&mut current_row))
                    .spacing(8)
                    .into(),
            );
        }
    }

    if !current_row.is_empty() {
        // Pad incomplete row
        while current_row.len() < GRID_COLUMNS {
            current_row.push(Space::new().width(THUMB_SIZE).into());
        }
        grid_rows.push(
            row(std::mem::take(&mut current_row))
                .spacing(8)
                .into(),
        );
    }

    column(grid_rows).spacing(8).padding(10).into()
}

fn photo_cell<'a>(
    photo: &'a Photo,
    thumbnail: Option<&'a iced::widget::image::Handle>,
) -> Element<'a, Message> {
    let filename = std::path::Path::new(&photo.file_path)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let display_name = if filename.len() > 20 {
        format!("{}...", &filename[..17])
    } else {
        filename
    };

    let thumb_content: Element<'a, Message> = if let Some(handle) = thumbnail {
        image(handle.clone())
            .width(THUMB_SIZE)
            .height(THUMB_SIZE)
            .into()
    } else {
        container(text("...").size(12))
            .width(THUMB_SIZE)
            .height(THUMB_SIZE)
            .center_x(THUMB_SIZE)
            .center_y(THUMB_SIZE)
            .into()
    };

    let cell = column![thumb_content, text(display_name).size(11),]
        .spacing(4)
        .width(THUMB_SIZE);

    button(cell)
        .on_press(Message::OpenPhoto(photo.id))
        .padding(4)
        .into()
}
