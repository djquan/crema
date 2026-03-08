use iced::widget::{Space, button, column, container, image, row, scrollable, text};
use iced::{Alignment, Background, Border, Color, ContentFit, Element, Length, Shadow, Theme};

use crate::app::{App, Message, PanelSection, Workspace};
use crate::widgets;

const APP_BG: Color = Color::from_rgb(0.08, 0.08, 0.09);
const PANEL_BG: Color = Color::from_rgb(0.12, 0.12, 0.13);
const PANEL_ALT_BG: Color = Color::from_rgb(0.10, 0.10, 0.11);
const CANVAS_BG: Color = Color::from_rgb(0.06, 0.06, 0.07);
const BORDER: Color = Color::from_rgb(0.20, 0.20, 0.22);
const MUTED: Color = Color::from_rgb(0.66, 0.66, 0.69);
const ACCENT: Color = Color::from_rgb(0.26, 0.52, 0.94);

pub fn view(app: &App) -> Element<'_, Message> {
    let content = match app.workspace() {
        Workspace::Library => library_body(app),
        Workspace::Develop => develop_body(app),
    };

    container(
        column![toolbar(app), content, bottom_bar(app)]
            .width(Length::Fill)
            .height(Length::Fill)
            .spacing(0),
    )
    .style(app_shell)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn toolbar(app: &App) -> Element<'_, Message> {
    let workspace_switcher = row![
        workspace_button("Library", Workspace::Library, app.workspace(), true),
        workspace_button(
            "Develop",
            Workspace::Develop,
            app.workspace(),
            app.has_selection()
        ),
    ]
    .spacing(6);

    let import_btn = button("Import")
        .on_press(Message::Import)
        .padding([8, 14])
        .style(primary_action);

    let export_btn = button("Export")
        .on_press_maybe(app.can_export().then_some(Message::Export))
        .padding([8, 14])
        .style(secondary_action);

    let panel_btn = button(if app.right_panel_open() {
        "Hide Panels"
    } else {
        "Show Panels"
    })
    .on_press_maybe((app.workspace() == Workspace::Develop).then_some(Message::ToggleRightPanel))
    .padding([8, 12])
    .style(secondary_action);

    let photo_summary = if app.workspace() == Workspace::Develop {
        column![
            text(app.current_photo_label()).size(14),
            text(app.current_photo_summary()).size(11).color(MUTED),
        ]
        .spacing(2)
        .width(Length::Fill)
    } else {
        column![
            text(format!("{} photos", app.filtered_photos().len())).size(14),
            text("Select in Library, then open Develop.")
                .size(11)
                .color(MUTED),
        ]
        .spacing(2)
        .width(Length::Fill)
    };

    container(
        row![
            text("Crema").size(24),
            Space::new().width(16),
            workspace_switcher,
            Space::new().width(24),
            container(photo_summary).width(Length::Fill),
            Space::new().width(16),
            import_btn,
            Space::new().width(8),
            export_btn,
            Space::new().width(8),
            panel_btn,
        ]
        .align_y(Alignment::Center),
    )
    .padding([12, 14])
    .style(panel_container)
    .into()
}

fn library_body(app: &App) -> Element<'_, Message> {
    row![
        widgets::date_sidebar::view(app.photos(), app.date_filter(), app.expanded_dates()),
        library_grid(app),
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn library_grid(app: &App) -> Element<'_, Message> {
    let selection_label: Element<'_, Message> = if app.has_selection() {
        text(format!("Selected: {}", app.current_photo_label()))
            .size(12)
            .color(MUTED)
            .into()
    } else {
        text("No photo selected").size(12).color(MUTED).into()
    };

    let open_button: Element<'_, Message> = button("Open In Develop")
        .on_press_maybe(
            app.has_selection()
                .then_some(Message::SetWorkspace(Workspace::Develop)),
        )
        .padding([8, 14])
        .style(primary_action)
        .into();

    let heading = row![
        column![
            text("Library").size(20),
            text(format!("{} visible photos", app.filtered_photos().len()))
                .size(12)
                .color(MUTED),
        ]
        .spacing(2),
        Space::new().width(Length::Fill),
        selection_label,
        Space::new().width(12),
        open_button,
    ]
    .align_y(Alignment::Center);

    container(
        column![
            heading,
            scrollable(widgets::thumbnail_grid::view(
                app.filtered_photos(),
                app.thumbnails(),
                app.selected_photo(),
            ))
            .height(Length::Fill)
            .width(Length::Fill),
        ]
        .spacing(12)
        .padding(14),
    )
    .style(canvas_panel)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn develop_body(app: &App) -> Element<'_, Message> {
    let mut center = row![photo_area(app)]
        .width(Length::Fill)
        .height(Length::Fill);

    if app.right_panel_open() {
        center = center.push(right_panel(app));
    }

    column![
        center,
        widgets::filmstrip::view(app.photos(), app.thumbnails(), app.selected_photo())
    ]
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn photo_area(app: &App) -> Element<'_, Message> {
    let status_message: Element<'_, Message> = if app.is_loading_photo() {
        text("Loading original + preview")
            .size(11)
            .color(MUTED)
            .into()
    } else if app.is_processing() {
        text("Rendering preview").size(11).color(MUTED).into()
    } else {
        text(app.status_message()).size(11).color(MUTED).into()
    };

    let status_line = row![
        text("Fit").size(11).color(MUTED),
        Space::new().width(Length::Fill),
        status_message
    ]
    .align_y(Alignment::Center);

    let content: Element<'_, Message> = if let Some(handle) = app.processed_image() {
        container(
            image(handle.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(ContentFit::Contain)
                .expand(true),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .into()
    } else if app.is_loading_photo() {
        empty_viewport(
            "Loading photo",
            "Crema is decoding the original file and building the Develop preview.",
        )
    } else if app.has_selection() {
        empty_viewport(
            "Preparing preview",
            "The selected photo will appear here once the Develop preview is ready.",
        )
    } else {
        empty_viewport(
            "No photo selected",
            "Select a photo in Library, then switch to Develop to edit it.",
        )
    };

    container(column![status_line, content].spacing(10).padding(14))
        .style(canvas_panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn empty_viewport<'a>(title: &'a str, body: &'a str) -> Element<'a, Message> {
    container(
        column![text(title).size(20), text(body).size(13).color(MUTED),]
            .spacing(6)
            .width(Length::Fill),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn right_panel(app: &App) -> Element<'_, Message> {
    let tools_header = row![
        text("Develop").size(16),
        Space::new().width(Length::Fill),
        button("Auto")
            .on_press_maybe(
                app.preview_image()
                    .is_some()
                    .then_some(Message::AutoEnhance)
            )
            .padding([4, 10])
            .style(secondary_action),
        Space::new().width(6),
        button("Reset")
            .on_press(Message::ResetEdits)
            .padding([4, 10])
            .style(secondary_action),
    ]
    .align_y(Alignment::Center);

    let content = column![
        tools_header,
        section_card(
            "Histogram",
            app.is_panel_open(PanelSection::Histogram),
            Message::TogglePanelSection(PanelSection::Histogram),
            None,
            widgets::histogram::view(app.histogram()),
        ),
        widgets::edit_panel::view(app),
        section_card(
            "Metadata",
            app.is_panel_open(PanelSection::Metadata),
            Message::TogglePanelSection(PanelSection::Metadata),
            None,
            widgets::metadata_panel::view(app.current_exif()),
        ),
    ]
    .spacing(10)
    .padding(12)
    .width(320);

    container(scrollable(content).height(Length::Fill))
        .style(side_panel)
        .width(320)
        .height(Length::Fill)
        .into()
}

pub fn section_card<'a>(
    title: &'a str,
    is_open: bool,
    toggle: Message,
    reset: Option<Message>,
    body: Element<'a, Message>,
) -> Element<'a, Message> {
    let reset_button: Element<'a, Message> = if let Some(reset_message) = reset {
        button("Reset")
            .on_press(reset_message)
            .padding([3, 8])
            .style(button::text)
            .into()
    } else {
        Space::new().width(1).into()
    };

    let header = row![
        button(if is_open { "v" } else { ">" })
            .on_press(toggle)
            .padding([2, 6])
            .style(button::text),
        text(title).size(13),
        Space::new().width(Length::Fill),
        reset_button
    ]
    .align_y(Alignment::Center);

    let mut card = column![header].spacing(10).padding(10);
    if is_open {
        card = card.push(body);
    }

    container(card).style(card_container).into()
}

fn bottom_bar(app: &App) -> Element<'_, Message> {
    container(text(app.footer_status()).size(11).color(MUTED))
        .padding([8, 14])
        .style(footer_container)
        .width(Length::Fill)
        .into()
}

fn workspace_button(
    label: &'static str,
    workspace: Workspace,
    active: Workspace,
    enabled: bool,
) -> iced::widget::Button<'static, Message, Theme> {
    button(label)
        .on_press_maybe(enabled.then_some(Message::SetWorkspace(workspace)))
        .padding([8, 14])
        .style(move |theme: &Theme, status| {
            if workspace == active {
                primary_action(theme, status)
            } else {
                secondary_action(theme, status)
            }
        })
}

fn app_shell(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(APP_BG)),
        ..Default::default()
    }
}

fn panel_container(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(PANEL_BG)),
        border: Border {
            color: BORDER,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn canvas_panel(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(CANVAS_BG)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn side_panel(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(PANEL_ALT_BG)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn card_container(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(PANEL_BG)),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

fn footer_container(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(PANEL_BG)),
        border: Border {
            color: BORDER,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn primary_action(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered => Color::from_rgb(0.30, 0.56, 0.98),
        button::Status::Pressed => Color::from_rgb(0.21, 0.44, 0.82),
        button::Status::Disabled => Color::from_rgb(0.18, 0.22, 0.28),
        _ => ACCENT,
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color: Color::WHITE,
        border: Border {
            color: background,
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

fn secondary_action(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = match status {
        button::Status::Hovered => (Color::from_rgb(0.18, 0.18, 0.20), Color::WHITE),
        button::Status::Pressed => (Color::from_rgb(0.14, 0.14, 0.16), Color::WHITE),
        button::Status::Disabled => (Color::from_rgb(0.11, 0.11, 0.12), MUTED),
        _ => (Color::from_rgb(0.13, 0.13, 0.14), Color::WHITE),
    };

    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 6.0.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}
