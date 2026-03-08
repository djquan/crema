use iced::widget::{Space, button, column, row, slider, text};
use iced::{Color, Element, Length};

use crate::app::{App, EditControl, EditSection, Message, PanelSection};
use crate::views::unified::section_card;

const MUTED: Color = Color::from_rgb(0.66, 0.66, 0.69);
const ACTIVE: Color = Color::from_rgb(0.82, 0.86, 0.95);

pub fn view(app: &App) -> Element<'_, Message> {
    column![
        section_card(
            "Light",
            app.is_panel_open(PanelSection::Light),
            Message::TogglePanelSection(PanelSection::Light),
            Some(Message::ResetSection(EditSection::Light)),
            light_controls(app),
        ),
        section_card(
            "Color",
            app.is_panel_open(PanelSection::Color),
            Message::TogglePanelSection(PanelSection::Color),
            Some(Message::ResetSection(EditSection::Color)),
            color_controls(app),
        ),
    ]
    .spacing(10)
    .into()
}

fn light_controls(app: &App) -> Element<'_, Message> {
    let params = app.edit_params();

    column![
        control(
            "Exposure",
            format!("{:+.1} EV", params.exposure),
            -5.0..=5.0,
            params.exposure,
            0.01,
            app.is_control_adjusted(EditControl::Exposure),
            Message::ExposureChanged,
            Message::ResetControl(EditControl::Exposure),
        ),
        control(
            "Contrast",
            format!("{:.0}", params.contrast),
            -100.0..=100.0,
            params.contrast,
            1.0,
            app.is_control_adjusted(EditControl::Contrast),
            Message::ContrastChanged,
            Message::ResetControl(EditControl::Contrast),
        ),
        control(
            "Highlights",
            format!("{:.0}", params.highlights),
            -100.0..=100.0,
            params.highlights,
            1.0,
            app.is_control_adjusted(EditControl::Highlights),
            Message::HighlightsChanged,
            Message::ResetControl(EditControl::Highlights),
        ),
        control(
            "Shadows",
            format!("{:.0}", params.shadows),
            -100.0..=100.0,
            params.shadows,
            1.0,
            app.is_control_adjusted(EditControl::Shadows),
            Message::ShadowsChanged,
            Message::ResetControl(EditControl::Shadows),
        ),
        control(
            "Blacks",
            format!("{:.0}", params.blacks),
            -100.0..=100.0,
            params.blacks,
            1.0,
            app.is_control_adjusted(EditControl::Blacks),
            Message::BlacksChanged,
            Message::ResetControl(EditControl::Blacks),
        ),
    ]
    .spacing(10)
    .into()
}

fn color_controls(app: &App) -> Element<'_, Message> {
    let params = app.edit_params();

    column![
        control(
            "Temperature",
            format!("{:.0} K", params.wb_temp),
            2000.0..=25000.0,
            params.wb_temp,
            10.0,
            app.is_control_adjusted(EditControl::WbTemp),
            Message::WbTempChanged,
            Message::ResetControl(EditControl::WbTemp),
        ),
        control(
            "Tint",
            format!("{:+.0}", params.wb_tint),
            -150.0..=150.0,
            params.wb_tint,
            1.0,
            app.is_control_adjusted(EditControl::WbTint),
            Message::WbTintChanged,
            Message::ResetControl(EditControl::WbTint),
        ),
        control(
            "Vibrance",
            format!("{:.0}", params.vibrance),
            -100.0..=100.0,
            params.vibrance,
            1.0,
            app.is_control_adjusted(EditControl::Vibrance),
            Message::VibranceChanged,
            Message::ResetControl(EditControl::Vibrance),
        ),
        control(
            "Saturation",
            format!("{:.0}", params.saturation),
            -100.0..=100.0,
            params.saturation,
            1.0,
            app.is_control_adjusted(EditControl::Saturation),
            Message::SaturationChanged,
            Message::ResetControl(EditControl::Saturation),
        ),
    ]
    .spacing(10)
    .into()
}

fn control<'a>(
    label: &'static str,
    value_text: String,
    range: std::ops::RangeInclusive<f32>,
    value: f32,
    step: f32,
    adjusted: bool,
    on_change: impl Fn(f32) -> Message + 'a,
    reset: Message,
) -> Element<'a, Message> {
    let label_color = if adjusted { ACTIVE } else { MUTED };

    column![
        row![
            text(label).size(12).color(label_color),
            Space::new().width(Length::Fill),
            text(value_text).size(12).color(label_color),
            Space::new().width(8),
            button("Reset")
                .on_press(reset)
                .padding([2, 6])
                .style(button::text),
        ]
        .align_y(iced::Alignment::Center),
        slider(range, value, on_change).step(step),
    ]
    .spacing(5)
    .into()
}
