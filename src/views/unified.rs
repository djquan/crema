use iced::widget::{Space, button, column, container, image, row, scrollable, text};
use iced::{Alignment, Element, Length};

use super::CANVAS_BG;
use crate::app::{App, Message, ViewMode};
use crate::widgets;

pub fn view(app: &App) -> Element<'_, Message> {
    let mut content = column![]
        .width(Length::Fill)
        .height(Length::Fill);

    content = content.push(toolbar(app));

    let sidebar =
        widgets::date_sidebar::view(app.photos(), app.date_filter(), app.expanded_dates());

    let main_area = match app.view_mode() {
        ViewMode::Grid => grid_area(app),
        ViewMode::Photo => photo_area(app),
    };

    let mut center_row = row![sidebar, main_area]
        .spacing(0)
        .width(Length::Fill)
        .height(Length::Fill);

    if app.right_panel_open() {
        center_row = center_row.push(right_panel(app));
    }

    content = content.push(center_row);

    if app.view_mode() == ViewMode::Photo {
        content = content.push(widgets::filmstrip::view(
            app.photos(),
            app.thumbnails(),
            app.selected_photo(),
        ));
    }

    content = content.push(bottom_bar(app));

    content.into()
}

fn toolbar(app: &App) -> Element<'_, Message> {
    let import_btn = button("Import").on_press(Message::Import);

    let has_image = app.selected_photo().is_some() && app.processed_image().is_some();
    let export_btn = if has_image {
        button("Export").on_press(Message::Export)
    } else {
        button("Export")
    };

    let edit_label = if app.right_panel_open() {
        "Edit <"
    } else {
        "Edit >"
    };
    let edit_toggle = button(edit_label).on_press(Message::ToggleRightPanel);

    row![
        text("Crema").size(24),
        Space::new().width(10),
        import_btn,
        Space::new().width(Length::Fill),
        export_btn,
        Space::new().width(8),
        edit_toggle,
    ]
    .spacing(0)
    .padding(10)
    .align_y(Alignment::Center)
    .into()
}

fn grid_area(app: &App) -> Element<'_, Message> {
    container(
        scrollable(widgets::thumbnail_grid::view(
            app.filtered_photos(),
            app.thumbnails(),
        ))
        .height(Length::Fill)
        .width(Length::Fill),
    )
    .style(|_theme| container::Style {
        background: Some(CANVAS_BG.into()),
        ..Default::default()
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn photo_area(app: &App) -> Element<'_, Message> {
    let canvas_style = |_theme: &_| container::Style {
        background: Some(CANVAS_BG.into()),
        ..Default::default()
    };

    if let Some(handle) = app.processed_image() {
        container(
            image(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .style(canvas_style)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    } else if app.selected_photo().is_some() {
        container(text("Loading...").size(16))
            .style(canvas_style)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    } else {
        container(text("Select a photo").size(16))
            .style(canvas_style)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into()
    }
}

fn right_panel(app: &App) -> Element<'_, Message> {
    let edit_panel = widgets::edit_panel::view(app.edit_params(), app.preview_image().is_some());
    let metadata_panel = widgets::metadata_panel::view(app.current_exif());
    let histogram = widgets::histogram::view(app.histogram());

    scrollable(
        column![histogram, edit_panel, metadata_panel]
            .spacing(10)
            .padding(10)
            .width(300),
    )
    .height(Length::Fill)
    .into()
}

fn bottom_bar(app: &App) -> Element<'_, Message> {
    let grid_btn = button(text("Grid").size(12))
        .on_press(Message::SetViewMode(ViewMode::Grid))
        .padding([4, 12])
        .style(if app.view_mode() == ViewMode::Grid {
            button::primary
        } else {
            button::secondary
        });

    let photo_btn = button(text("Photo").size(12))
        .on_press(Message::SetViewMode(ViewMode::Photo))
        .padding([4, 12])
        .style(if app.view_mode() == ViewMode::Photo {
            button::primary
        } else {
            button::secondary
        });

    row![
        grid_btn,
        photo_btn,
        Space::new().width(16),
        text(app.status_message()).size(12),
    ]
    .spacing(4)
    .padding(5)
    .align_y(Alignment::Center)
    .width(Length::Fill)
    .into()
}
