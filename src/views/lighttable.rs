use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::app::{App, Message};
use crate::widgets;

pub fn view(app: &App) -> Element<'_, Message> {
    let toolbar = row![
        text("Crema").size(24),
        Space::new().width(Length::Fill),
        button("Import Folder").on_press(Message::ImportFolder),
    ]
    .spacing(10)
    .padding(10)
    .align_y(Alignment::Center);

    let sidebar =
        widgets::date_sidebar::view(app.photos(), app.date_filter(), app.expanded_dates());

    let grid = scrollable(widgets::thumbnail_grid::view(
        app.filtered_photos(),
        app.thumbnails(),
    ))
    .height(Length::Fill)
    .width(Length::Fill);

    let status = container(text(app.status_message()).size(12))
        .padding(5)
        .width(Length::Fill);

    column![toolbar, row![sidebar, grid].height(Length::Fill), status]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}
