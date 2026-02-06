use iced::Element;
use iced::widget::{column, container, row, text};

use crate::app::Message;

pub fn view(exif_data: &[(String, String)]) -> Element<'_, Message> {
    let mut items: Vec<Element<'_, Message>> = vec![text("Metadata").size(16).into()];

    if exif_data.is_empty() {
        items.push(text("No EXIF data available").size(12).into());
    } else {
        for (key, value) in exif_data {
            items.push(
                row![text(format!("{key}:")).size(11), text(value).size(11),]
                    .spacing(8)
                    .into(),
            );
        }
    }

    container(column(items).spacing(6)).padding(10).into()
}
