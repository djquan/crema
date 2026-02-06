use iced::widget::{Space, button, column, container, image, row, scrollable, text};
use iced::{Alignment, Element, Length};

use super::CANVAS_BG;
use crate::app::{App, Message};
use crate::widgets;

pub fn view(app: &App) -> Element<'_, Message> {
    let toolbar = row![
        button("< Back").on_press(Message::BackToGrid),
        Space::new().width(Length::Fill),
        text("Darkroom").size(20),
        Space::new().width(Length::Fill),
    ]
    .spacing(10)
    .padding(10)
    .align_y(Alignment::Center);

    let canvas_style = |_theme: &_| container::Style {
        background: Some(CANVAS_BG.into()),
        ..Default::default()
    };

    let image_view = if let Some(handle) = app.processed_image() {
        container(
            image(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .style(canvas_style)
        .width(Length::Fill)
        .height(Length::Fill)
    } else {
        container(text("Loading image...").size(16))
            .style(canvas_style)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
    };

    let edit_panel = widgets::edit_panel::view(app.edit_params());
    let metadata_panel = widgets::metadata_panel::view(app.current_exif());
    let histogram = widgets::histogram::view(app.histogram());

    let sidebar = scrollable(
        column![histogram, edit_panel, metadata_panel]
            .spacing(10)
            .padding(10)
            .width(300),
    )
    .height(Length::Fill);

    let content = row![image_view, sidebar]
        .spacing(0)
        .width(Length::Fill)
        .height(Length::Fill);

    column![toolbar, content]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
