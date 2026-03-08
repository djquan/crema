use iced::Element;
use iced::widget::{column, row, text};

use crate::app::Message;

pub fn view(exif_data: &[(String, String)]) -> Element<'_, Message> {
    let mut items: Vec<Element<'_, Message>> = Vec::new();

    if exif_data.is_empty() {
        items.push(
            text("No EXIF data available for this photo.")
                .size(12)
                .into(),
        );
    } else {
        for (key, value) in exif_data {
            items.push(
                row![
                    text(format!("{key}:")).size(11).width(110),
                    text(value).size(11),
                ]
                .spacing(8)
                .into(),
            );
        }
    }

    column(items).spacing(6).into()
}
