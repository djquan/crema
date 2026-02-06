use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::app::{App, Message};
use crate::widgets;

pub fn view(app: &App) -> Element<'_, Message> {
    let toolbar = row![
        text("Photors").size(24),
        Space::new().width(Length::Fill),
        button("Import Folder").on_press(Message::ImportFolder),
    ]
    .spacing(10)
    .padding(10)
    .align_y(Alignment::Center);

    let grid = widgets::thumbnail_grid::view(app.photos(), app.thumbnails());

    let status = container(text(app.status_message()).size(12))
        .padding(5)
        .width(Length::Fill);

    column![toolbar, scrollable(grid).height(Length::Fill), status]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
